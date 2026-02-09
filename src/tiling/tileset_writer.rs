use std::fs;
use std::path::Path;

use rayon::prelude::*;
use serde_json::json;
use tracing::info;

use crate::config::{TextureConfig, TilingConfig};
use crate::error::{PhotoTilerError, Result};
use crate::tiling::atlas_repacker;
use crate::tiling::glb_writer::write_glb_compressed;
use crate::tiling::lod::LodChain;
use crate::tiling::octree::{child_bounds, split_mesh};
use crate::tiling::simplifier::simplify_mesh;
use crate::types::{BoundingBox, IndexedMesh, MaterialLibrary, TileContent, TileNode};

/// Intermediate output of tile hierarchy construction.
pub struct TilesetOutput {
    pub root: TileNode,
    pub root_transform: [f64; 16],
}

/// Convert a tile address to a hierarchical URI path.
///
/// - `"root"` → `"tiles/root.glb"`
/// - `"0"` → `"tiles/0/tile.glb"`
/// - `"0_3"` → `"tiles/0/0_3/tile.glb"`
/// - `"0_3_1"` → `"tiles/0/0_3/0_3_1/tile.glb"`
fn address_to_uri(address: &str) -> String {
    if address == "root" {
        return "tiles/root.glb".into();
    }

    // Build hierarchical path from address segments
    // Address "0_3_1" → path components: ["0", "0_3", "0_3_1"]
    let parts: Vec<&str> = address.split('_').collect();
    let mut path_segments = Vec::with_capacity(parts.len());
    let mut accum = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            accum.push_str(part);
        } else {
            accum.push('_');
            accum.push_str(part);
        }
        path_segments.push(accum.clone());
    }

    let dir_path = path_segments.join("/");
    format!("tiles/{dir_path}/tile.glb")
}

/// Write a tile's GLB using atlas repacking when textures are enabled,
/// then eagerly flush to disk and free the data.
///
/// Applies vertex cache optimization before writing to improve GPU
/// rendering performance and meshopt compression ratios.
fn write_tile_glb_to_disk(
    mesh: &IndexedMesh,
    materials: &MaterialLibrary,
    texture_config: &TextureConfig,
    out_dir: &Path,
    address: &str,
) -> TileContent {
    // Vertex cache optimization: improves GPU rendering perf and compression ratios
    let mesh = if !mesh.is_empty() {
        let optimized_indices = meshopt::optimize_vertex_cache(&mesh.indices, mesh.vertex_count());
        &IndexedMesh {
            positions: mesh.positions.clone(),
            normals: mesh.normals.clone(),
            uvs: mesh.uvs.clone(),
            colors: mesh.colors.clone(),
            indices: optimized_indices,
            material_index: mesh.material_index,
        }
    } else {
        mesh
    };

    let glb_data = if texture_config.enabled && mesh.has_uvs() {
        if let Some(result) = atlas_repacker::repack_atlas(mesh, materials, texture_config) {
            write_glb_compressed(&result.mesh, materials, Some(&result.atlas_texture))
        } else {
            write_glb_compressed(mesh, materials, None)
        }
    } else {
        write_glb_compressed(mesh, materials, None)
    };

    let uri = address_to_uri(address);
    let glb_path = out_dir.join(&uri);

    // Write to disk immediately
    if let Some(parent) = glb_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&glb_path, &glb_data) {
        tracing::error!("Failed to write {}: {e}", glb_path.display());
    }

    // Return content with empty data (already on disk)
    TileContent {
        glb_data: vec![],
        uri,
    }
}

/// Build a tile hierarchy from LOD chains, writing GLBs eagerly to disk.
///
/// Merges all LOD-0 meshes into a single mesh, then builds a unified
/// spatial-LOD hierarchy where every internal node has content (a simplified
/// mesh of its spatial region) and children are spatial subdivisions.
pub fn build_tileset(
    lod_chains: Vec<LodChain>,
    bounds: &BoundingBox,
    config: &TilingConfig,
    materials: &MaterialLibrary,
    texture_config: &TextureConfig,
    out_dir: &Path,
) -> TilesetOutput {
    // Merge all LOD-0 (finest) meshes into a single mesh
    let mut merged = IndexedMesh::default();
    for chain in &lod_chains {
        if let Some(level) = chain.levels.iter().find(|l| l.level == 0) {
            merged = merge_meshes(merged, &level.mesh);
        }
    }

    drop(lod_chains);

    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    let root = build_tile_recursive(
        merged,
        bounds,
        0,
        config.max_depth,
        config.max_triangles_per_tile,
        "root",
        materials,
        texture_config,
        out_dir,
    );

    TilesetOutput {
        root,
        root_transform: identity,
    }
}

/// Recursively build a unified spatial-LOD tile hierarchy.
///
/// Each node gets a simplified version of its mesh as display content, while
/// the original (unsimplified) mesh is spatially subdivided into octant children.
/// This ensures every internal node has renderable content and the tree combines
/// both spatial subdivision and LOD at every level.
///
/// Leaf condition: `triangle_count <= max_tris` OR `depth >= max_depth`.
fn build_tile_recursive(
    mesh: IndexedMesh,
    bounds: &BoundingBox,
    depth: u32,
    max_depth: u32,
    max_tris: usize,
    address: &str,
    materials: &MaterialLibrary,
    texture_config: &TextureConfig,
    out_dir: &Path,
) -> TileNode {
    let is_leaf = mesh.triangle_count() <= max_tris || depth >= max_depth;

    let geometric_error = if is_leaf {
        0.0
    } else {
        bounds.diagonal() * 0.5_f64.powi(depth as i32)
    };

    if is_leaf {
        // Leaf: write the full-detail mesh as content, no children
        let content = if !mesh.is_empty() {
            Some(write_tile_glb_to_disk(
                &mesh, materials, texture_config, out_dir, address,
            ))
        } else {
            None
        };

        return TileNode {
            address: address.into(),
            level: depth,
            bounds: *bounds,
            geometric_error,
            content,
            children: vec![],
        };
    }

    // Internal node: simplify the mesh for this node's display content,
    // then spatially split the ORIGINAL mesh for children.
    // Deeper levels use relaxed simplification (less aggressive, faster).
    let content_mesh = if mesh.triangle_count() < 64 {
        // Too few triangles to simplify meaningfully -- use as-is
        mesh.clone()
    } else {
        let (ratio, lock_border) = if depth >= 3 {
            (0.5, false) // Faster, less aggressive for deep/coarse nodes
        } else {
            (0.25, true) // More aggressive for top-level nodes
        };
        simplify_mesh(&mesh, ratio, lock_border).mesh
    };

    let content = if !content_mesh.is_empty() {
        Some(write_tile_glb_to_disk(
            &content_mesh, materials, texture_config, out_dir, address,
        ))
    } else {
        None
    };
    drop(content_mesh);

    // Split the ORIGINAL mesh spatially into 8 octants
    let sub_meshes = split_mesh(&mesh, bounds);
    drop(mesh);

    // Recurse into non-empty octants in parallel
    let child_tasks: Vec<_> = sub_meshes
        .into_iter()
        .enumerate()
        .filter_map(|(i, sub)| {
            if sub.is_empty() {
                return None;
            }
            let child_addr = if address == "root" {
                format!("{i}")
            } else {
                format!("{address}_{i}")
            };
            let cb = child_bounds(bounds, i);
            Some((child_addr, sub, cb))
        })
        .collect();

    let children: Vec<TileNode> = child_tasks
        .into_par_iter()
        .map(|(child_addr, sub, cb)| {
            build_tile_recursive(
                sub,
                &cb,
                depth + 1,
                max_depth,
                max_tris,
                &child_addr,
                materials,
                texture_config,
                out_dir,
            )
        })
        .collect();

    TileNode {
        address: address.into(),
        level: depth,
        bounds: *bounds,
        geometric_error,
        content,
        children,
    }
}

/// Write the tileset.json to disk.
///
/// GLB files have already been written eagerly during `build_tileset`.
/// Returns the total number of tiles (content nodes).
pub fn write_tileset(
    output: &TilesetOutput,
    transform: &[f64; 16],
    out_dir: &Path,
) -> Result<usize> {
    let tile_count = count_content_nodes(&output.root);

    // Build tileset.json
    let tileset_json = build_tileset_json(&output.root, transform);

    let tileset_path = out_dir.join("tileset.json");
    let json_string = serde_json::to_string_pretty(&tileset_json)
        .map_err(|e| PhotoTilerError::Output(format!("Failed to serialize tileset.json: {e}")))?;

    fs::write(&tileset_path, &json_string)
        .map_err(|e| PhotoTilerError::Output(format!("Failed to write tileset.json: {e}")))?;

    info!(
        tiles = tile_count,
        path = %tileset_path.display(),
        "Wrote tileset.json"
    );

    Ok(tile_count)
}

/// Count nodes that have content (i.e., GLB tiles).
fn count_content_nodes(node: &TileNode) -> usize {
    let self_count = if node.content.is_some() { 1 } else { 0 };
    self_count + node.children.iter().map(count_content_nodes).sum::<usize>()
}

/// Build the tileset.json as a serde_json::Value.
fn build_tileset_json(root: &TileNode, transform: &[f64; 16]) -> serde_json::Value {
    let root_tile = tile_node_to_json(root, Some(transform));

    json!({
        "asset": {
            "version": "1.1",
            "generator": "photo-tiler"
        },
        "geometricError": root.geometric_error,
        "root": root_tile
    })
}

/// Convert a TileNode to its tileset.json representation.
fn tile_node_to_json(node: &TileNode, transform: Option<&[f64; 16]>) -> serde_json::Value {
    let bv = bounding_volume_box(&node.bounds);

    let mut tile = json!({
        "boundingVolume": {
            "box": bv
        },
        "geometricError": node.geometric_error,
        "refine": "REPLACE"
    });

    if let Some(t) = transform {
        tile["transform"] = json!(t);
    }

    if let Some(content) = &node.content {
        tile["content"] = json!({
            "uri": content.uri
        });
    }

    if !node.children.is_empty() {
        let children: Vec<serde_json::Value> = node
            .children
            .iter()
            .map(|c| tile_node_to_json(c, None))
            .collect();
        tile["children"] = json!(children);
    }

    tile
}

/// Convert a BoundingBox to the 12-float `boundingVolume.box` format.
///
/// Format: `[cx, cy, cz, hx, 0, 0, 0, hy, 0, 0, 0, hz]`
/// (center + axis-aligned half-extents as 3 column vectors)
fn bounding_volume_box(bounds: &BoundingBox) -> [f64; 12] {
    let c = bounds.center();
    let he = bounds.half_extents();
    [
        c[0], c[1], c[2], // center
        he[0], 0.0, 0.0, // x half-axis
        0.0, he[1], 0.0, // y half-axis
        0.0, 0.0, he[2], // z half-axis
    ]
}

/// Merge two IndexedMeshes by extending `a` with `b`'s data and offsetting indices.
/// Takes ownership of `a` to avoid cloning it.
fn merge_meshes(mut a: IndexedMesh, b: &IndexedMesh) -> IndexedMesh {
    if a.is_empty() {
        return b.clone();
    }
    if b.is_empty() {
        return a;
    }

    let a_vertex_count = a.vertex_count() as u32;

    a.positions.extend_from_slice(&b.positions);

    if a.has_normals() && b.has_normals() {
        a.normals.extend_from_slice(&b.normals);
    } else {
        a.normals.clear();
    }

    if a.has_uvs() && b.has_uvs() {
        a.uvs.extend_from_slice(&b.uvs);
    } else {
        a.uvs.clear();
    }

    if a.has_colors() && b.has_colors() {
        a.colors.extend_from_slice(&b.colors);
    } else {
        a.colors.clear();
    }

    a.indices.extend(b.indices.iter().map(|&i| i + a_vertex_count));

    if a.material_index.is_none() {
        a.material_index = b.material_index;
    }

    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiling::lod::{LodChain, LodLevel};

    fn unit_bounds() -> BoundingBox {
        BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        }
    }

    fn make_grid_mesh(n: usize) -> IndexedMesh {
        let verts_per_side = n + 1;
        let mut positions = Vec::new();
        for y in 0..verts_per_side {
            for x in 0..verts_per_side {
                let fx = x as f32 / n as f32;
                let fy = y as f32 / n as f32;
                positions.extend_from_slice(&[fx, fy, 0.5]);
            }
        }

        let mut indices = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let tl = (y * verts_per_side + x) as u32;
                let tr = tl + 1;
                let bl = tl + verts_per_side as u32;
                let br = bl + 1;
                indices.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
            }
        }

        IndexedMesh {
            positions,
            indices,
            ..Default::default()
        }
    }

    fn identity() -> [f64; 16] {
        [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ]
    }

    fn tex_config_disabled() -> TextureConfig {
        TextureConfig {
            enabled: false,
            ..Default::default()
        }
    }

    #[test]
    fn build_tileset_single_level() {
        let mesh = make_grid_mesh(4); // 32 triangles
        let chain = LodChain {
            levels: vec![LodLevel {
                level: 0,
                mesh: mesh.clone(),
                geometric_error: 0.0,
            }],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 100,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );
        assert_eq!(output.root.address, "root");
        assert_eq!(output.root.level, 0);
    }

    #[test]
    fn build_tileset_multi_level() {
        let mesh = make_grid_mesh(10); // 200 triangles

        let chain = LodChain {
            levels: vec![
                LodLevel {
                    level: 0,
                    mesh: mesh.clone(),
                    geometric_error: 0.0,
                },
            ],
            bounds: unit_bounds(),
        };

        // Use low max_triangles to force subdivision
        let config = TilingConfig {
            max_triangles_per_tile: 50,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );
        assert_eq!(output.root.address, "root");
        assert!(
            output.root.content.is_some(),
            "root should have content"
        );
        assert!(
            output.root.geometric_error > 0.0,
            "root should have positive geometric error"
        );
        // With subdivision forced, root should have children
        assert!(
            !output.root.children.is_empty(),
            "subdivided tileset root should have children"
        );
    }

    #[test]
    fn build_tileset_four_lods() {
        // With the new unified approach, we only use LOD-0 meshes.
        // Pass a large mesh and force subdivision via low max_triangles.
        let lod0 = make_grid_mesh(16); // 512 tris

        let chain = LodChain {
            levels: vec![
                LodLevel {
                    level: 0,
                    mesh: lod0,
                    geometric_error: 0.0,
                },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 50,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );

        assert_eq!(output.root.address, "root");
        assert!(output.root.content.is_some());

        // Verify hierarchy depth >= 2 (root + at least one level of children)
        fn max_depth(node: &TileNode) -> usize {
            if node.children.is_empty() {
                1
            } else {
                1 + node.children.iter().map(max_depth).max().unwrap_or(0)
            }
        }
        let depth = max_depth(&output.root);
        assert!(
            depth >= 2,
            "subdivided hierarchy should have depth >= 2, got {depth}"
        );
    }

    #[test]
    fn geometric_error_decreasing() {
        let lod0 = make_grid_mesh(16); // 512 tris

        let chain = LodChain {
            levels: vec![
                LodLevel {
                    level: 0,
                    mesh: lod0,
                    geometric_error: 0.0,
                },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 50,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );

        // Root has highest error
        let root_error = output.root.geometric_error;
        assert!(root_error > 0.0, "root should have positive geometric error");

        // Verify errors decrease down the hierarchy
        fn check_decreasing(node: &TileNode, parent_error: f64) {
            assert!(
                node.geometric_error <= parent_error,
                "child error {} should be <= parent error {}",
                node.geometric_error,
                parent_error
            );
            for child in &node.children {
                check_decreasing(child, node.geometric_error);
            }
        }
        for child in &output.root.children {
            check_decreasing(child, root_error);
        }

        // Leaves should have error = 0
        fn check_leaf_zero(node: &TileNode) {
            if node.children.is_empty() {
                assert_eq!(
                    node.geometric_error, 0.0,
                    "leaf tile should have geometric_error = 0"
                );
            }
            for child in &node.children {
                check_leaf_zero(child);
            }
        }
        check_leaf_zero(&output.root);
    }

    #[test]
    fn address_to_uri_mapping() {
        assert_eq!(address_to_uri("root"), "tiles/root.glb");
        assert_eq!(address_to_uri("0"), "tiles/0/tile.glb");
        assert_eq!(address_to_uri("0_3"), "tiles/0/0_3/tile.glb");
        assert_eq!(address_to_uri("0_3_1"), "tiles/0/0_3/0_3_1/tile.glb");
    }

    #[test]
    fn write_tileset_creates_files() {
        let mesh = make_grid_mesh(4);
        let chain = LodChain {
            levels: vec![LodLevel {
                level: 0,
                mesh: mesh.clone(),
                geometric_error: 0.0,
            }],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );

        let transform = identity();
        let tile_count = write_tileset(&output, &transform, tmp.path()).unwrap();

        // Should have tileset.json
        assert!(tmp.path().join("tileset.json").exists());

        // Should have tiles directory (GLBs written eagerly)
        assert!(tmp.path().join("tiles").exists());

        // Should have at least 1 tile
        assert!(tile_count >= 1);
    }

    #[test]
    fn tileset_json_is_valid() {
        let mesh = make_grid_mesh(4);
        let chain = LodChain {
            levels: vec![LodLevel {
                level: 0,
                mesh: mesh.clone(),
                geometric_error: 0.0,
            }],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );

        let transform = identity();
        write_tileset(&output, &transform, tmp.path()).unwrap();

        // Parse tileset.json
        let json_str = fs::read_to_string(tmp.path().join("tileset.json")).unwrap();
        let tileset: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Check required fields
        assert_eq!(tileset["asset"]["version"], "1.1");
        assert_eq!(tileset["asset"]["generator"], "photo-tiler");
        assert!(tileset["root"].is_object());
        assert!(tileset["root"]["boundingVolume"]["box"].is_array());
        assert_eq!(tileset["root"]["refine"], "REPLACE");
    }

    #[test]
    fn tileset_json_has_transform() {
        let mesh = make_grid_mesh(4);
        let chain = LodChain {
            levels: vec![LodLevel {
                level: 0,
                mesh: mesh.clone(),
                geometric_error: 0.0,
            }],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        };
        let _materials = MaterialLibrary::default();

        let transform = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 100.0, 200.0, 300.0,
            1.0,
        ];

        let tmp = tempfile::tempdir().unwrap();
        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &MaterialLibrary::default(),
            &tex_config_disabled(),
            tmp.path(),
        );
        write_tileset(&output, &transform, tmp.path()).unwrap();

        let json_str = fs::read_to_string(tmp.path().join("tileset.json")).unwrap();
        let tileset: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let t = tileset["root"]["transform"].as_array().unwrap();
        assert_eq!(t.len(), 16);
        // Check translation column
        assert_eq!(t[12].as_f64().unwrap(), 100.0);
        assert_eq!(t[13].as_f64().unwrap(), 200.0);
        assert_eq!(t[14].as_f64().unwrap(), 300.0);
    }

    #[test]
    fn bounding_volume_box_format() {
        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [2.0, 4.0, 6.0],
        };
        let bv = bounding_volume_box(&bounds);
        // center = (1, 2, 3), half-extents = (1, 2, 3)
        assert_eq!(bv[0], 1.0); // cx
        assert_eq!(bv[1], 2.0); // cy
        assert_eq!(bv[2], 3.0); // cz
        assert_eq!(bv[3], 1.0); // hx
        assert_eq!(bv[4], 0.0);
        assert_eq!(bv[5], 0.0);
        assert_eq!(bv[6], 0.0);
        assert_eq!(bv[7], 2.0); // hy
        assert_eq!(bv[8], 0.0);
        assert_eq!(bv[9], 0.0);
        assert_eq!(bv[10], 0.0);
        assert_eq!(bv[11], 3.0); // hz
    }

    #[test]
    fn merge_meshes_concatenates() {
        let a = IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let b = IndexedMesh {
            positions: vec![2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 2.0, 1.0, 0.0],
            indices: vec![0, 1, 2],
            ..Default::default()
        };

        let merged = merge_meshes(a, &b);
        assert_eq!(merged.vertex_count(), 6);
        assert_eq!(merged.triangle_count(), 2);
        // Second triangle's indices should be offset by 3
        assert_eq!(merged.indices[3], 3);
        assert_eq!(merged.indices[4], 4);
        assert_eq!(merged.indices[5], 5);
    }

    #[test]
    fn merge_meshes_empty() {
        let empty = IndexedMesh::default();
        let mesh = IndexedMesh {
            positions: vec![0.0, 0.0, 0.0],
            indices: vec![],
            ..Default::default()
        };

        let result = merge_meshes(empty, &mesh);
        assert_eq!(result.positions.len(), mesh.positions.len());

        let result2 = merge_meshes(mesh.clone(), &IndexedMesh::default());
        assert_eq!(result2.positions.len(), mesh.positions.len());
    }

    #[test]
    fn hierarchical_dirs_created() {
        let lod0 = make_grid_mesh(10);

        let chain = LodChain {
            levels: vec![
                LodLevel {
                    level: 0,
                    mesh: lod0,
                    geometric_error: 0.0,
                },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 50,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );

        write_tileset(&output, &identity(), tmp.path()).unwrap();

        // tiles/ directory should exist
        assert!(tmp.path().join("tiles").exists());
        // tileset.json should exist
        assert!(tmp.path().join("tileset.json").exists());
    }

    #[test]
    fn all_uris_match_files() {
        let lod0 = make_grid_mesh(10);

        let chain = LodChain {
            levels: vec![
                LodLevel {
                    level: 0,
                    mesh: lod0,
                    geometric_error: 0.0,
                },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 50,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );

        write_tileset(&output, &identity(), tmp.path()).unwrap();

        // Collect all URIs from the tileset
        fn collect_uris(node: &TileNode, uris: &mut Vec<String>) {
            if let Some(content) = &node.content {
                uris.push(content.uri.clone());
            }
            for child in &node.children {
                collect_uris(child, uris);
            }
        }

        let mut uris = Vec::new();
        collect_uris(&output.root, &mut uris);

        // Every URI should map to an actual file (written eagerly)
        for uri in &uris {
            let path = tmp.path().join(uri);
            assert!(
                path.exists(),
                "URI {uri} should map to existing file at {}",
                path.display()
            );
        }
    }

    #[test]
    fn glb_files_exist_on_disk() {
        let mesh = make_grid_mesh(10); // 200 triangles

        let chain = LodChain {
            levels: vec![
                LodLevel {
                    level: 0,
                    mesh: mesh.clone(),
                    geometric_error: 0.0,
                },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 50,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );

        let tile_count = write_tileset(&output, &identity(), tmp.path()).unwrap();

        assert!(tile_count >= 1, "should have written at least 1 tile");

        // Count GLB files recursively
        fn count_glb_files(dir: &Path) -> usize {
            let mut count = 0;
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_dir() {
                        count += count_glb_files(&path);
                    } else if path.extension().is_some_and(|ext| ext == "glb") {
                        count += 1;
                    }
                }
            }
            count
        }

        let glb_count = count_glb_files(&tmp.path().join("tiles"));
        assert_eq!(
            glb_count, tile_count,
            "GLB file count should match tile_count"
        );
    }

    #[test]
    fn every_internal_node_has_content() {
        let mesh = make_grid_mesh(16); // 512 tris

        let chain = LodChain {
            levels: vec![LodLevel {
                level: 0,
                mesh,
                geometric_error: 0.0,
            }],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 50,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );

        fn check_content(node: &TileNode) {
            if !node.children.is_empty() {
                assert!(
                    node.content.is_some(),
                    "internal node '{}' at level {} must have content",
                    node.address,
                    node.level
                );
            }
            for child in &node.children {
                check_content(child);
            }
        }
        check_content(&output.root);
    }

    #[test]
    fn spatial_subdivision_at_every_level() {
        let mesh = make_grid_mesh(16); // 512 tris

        let chain = LodChain {
            levels: vec![LodLevel {
                level: 0,
                mesh,
                geometric_error: 0.0,
            }],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 50,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );

        fn check_branching(node: &TileNode) {
            if !node.children.is_empty() {
                assert!(
                    node.children.len() >= 2,
                    "internal node '{}' should have branching factor >= 2, got {}",
                    node.address,
                    node.children.len()
                );
            }
            for child in &node.children {
                check_branching(child);
            }
        }
        check_branching(&output.root);
    }

    #[test]
    fn child_bounds_contained_in_parent() {
        let mesh = make_grid_mesh(16); // 512 tris

        let chain = LodChain {
            levels: vec![LodLevel {
                level: 0,
                mesh,
                geometric_error: 0.0,
            }],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 50,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();
        let tmp = tempfile::tempdir().unwrap();

        let output = build_tileset(
            vec![chain],
            &unit_bounds(),
            &config,
            &materials,
            &tex_config_disabled(),
            tmp.path(),
        );

        fn check_containment(node: &TileNode) {
            for child in &node.children {
                // Child bounds should be contained within parent bounds (with epsilon)
                let eps = 1e-6;
                assert!(
                    child.bounds.min[0] >= node.bounds.min[0] - eps
                        && child.bounds.min[1] >= node.bounds.min[1] - eps
                        && child.bounds.min[2] >= node.bounds.min[2] - eps
                        && child.bounds.max[0] <= node.bounds.max[0] + eps
                        && child.bounds.max[1] <= node.bounds.max[1] + eps
                        && child.bounds.max[2] <= node.bounds.max[2] + eps,
                    "child '{}' bounds {:?}-{:?} should be within parent '{}' bounds {:?}-{:?}",
                    child.address,
                    child.bounds.min,
                    child.bounds.max,
                    node.address,
                    node.bounds.min,
                    node.bounds.max
                );
                check_containment(child);
            }
        }
        check_containment(&output.root);
    }
}
