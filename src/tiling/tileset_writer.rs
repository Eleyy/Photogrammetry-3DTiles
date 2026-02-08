use std::fs;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde_json::json;
use tracing::info;

use crate::config::{TextureConfig, TilingConfig};
use crate::error::{PhotoTilerError, Result};
use crate::tiling::atlas_repacker;
use crate::tiling::glb_writer::write_glb;
use crate::tiling::lod::LodChain;
use crate::tiling::octree::{build_octree, OctreeNode};
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

/// Write a tile's GLB, using atlas repacking when textures are enabled.
fn write_tile_glb(
    mesh: &IndexedMesh,
    materials: &MaterialLibrary,
    texture_config: &TextureConfig,
) -> Vec<u8> {
    if texture_config.enabled && mesh.has_uvs() {
        if let Some(result) = atlas_repacker::repack_atlas(mesh, materials, texture_config) {
            return write_glb(&result.mesh, materials, Some(&result.atlas_texture));
        }
    }
    write_glb(mesh, materials, None)
}

/// Build a tile hierarchy from LOD chains.
///
/// Produces a proper N-level hierarchy matching the architecture spec:
/// ```text
/// Root (LOD N, coarsest) → children at LOD N-1 → ... → leaves at LOD 0
/// ```
///
/// Each LOD level maps to one tier of the tile tree. The coarsest LOD becomes
/// the root tile content, intermediate LODs become intermediate tile nodes,
/// and the finest LOD (LOD 0) octree-splits into leaf tiles.
pub fn build_tileset(
    lod_chains: &[LodChain],
    bounds: &BoundingBox,
    config: &TilingConfig,
    materials: &MaterialLibrary,
    texture_config: &TextureConfig,
) -> TilesetOutput {
    // Collect all LOD levels, merged per level across chains
    let max_lod = lod_chains
        .iter()
        .flat_map(|c| c.levels.iter())
        .map(|l| l.level)
        .max()
        .unwrap_or(0);

    // Merge meshes at each LOD level
    let mut level_meshes: Vec<(u32, IndexedMesh, f64)> = Vec::new();
    for lod in 0..=max_lod {
        let mut merged = IndexedMesh::default();
        let mut max_error = 0.0_f64;
        for chain in lod_chains {
            if let Some(level) = chain.levels.iter().find(|l| l.level == lod) {
                merged = merge_meshes(&merged, &level.mesh);
                if level.geometric_error > max_error {
                    max_error = level.geometric_error;
                }
            }
        }
        if !merged.is_empty() {
            level_meshes.push((lod, merged, max_error));
        }
    }

    // Sort by LOD level: finest (0) first, coarsest last
    level_meshes.sort_by_key(|(lod, _, _)| *lod);

    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    if level_meshes.len() <= 1 {
        // Single-level: octree split the mesh
        let mesh = if level_meshes.is_empty() {
            IndexedMesh::default()
        } else {
            level_meshes.remove(0).1
        };

        let tree = build_octree(&mesh, bounds, config.max_depth, config.max_triangles_per_tile);
        let root = octree_to_tile_node(&tree, "root", 0, bounds, 0.0, materials, texture_config);

        return TilesetOutput {
            root,
            root_transform: identity,
        };
    }

    // Multi-level hierarchy: build from coarsest (root) down to finest (leaves)
    let root = build_lod_hierarchy(&level_meshes, bounds, config, materials, texture_config);

    TilesetOutput {
        root,
        root_transform: identity,
    }
}

/// Build a multi-level LOD hierarchy.
///
/// Coarsest LOD → root tile content, each successively finer LOD becomes
/// children. Finest LOD (LOD 0) → octree-split into leaf tiles.
fn build_lod_hierarchy(
    level_meshes: &[(u32, IndexedMesh, f64)],
    bounds: &BoundingBox,
    config: &TilingConfig,
    materials: &MaterialLibrary,
    texture_config: &TextureConfig,
) -> TileNode {
    // level_meshes is sorted finest-first: [LOD0, LOD1, ..., LOD_N]
    // We want to build: root = LOD_N (coarsest), children = LOD_N-1, ..., leaves = LOD0
    let num_levels = level_meshes.len();

    // Start from the coarsest level (last in array)
    let coarsest_idx = num_levels - 1;
    let (_, ref coarsest_mesh, coarsest_error) = level_meshes[coarsest_idx];

    // Root tile: coarsest LOD
    let root_glb = write_tile_glb(coarsest_mesh, materials, texture_config);
    let root_uri = address_to_uri("root");

    // Build children recursively from the next-finer level
    let children = if num_levels >= 2 {
        build_lod_children(level_meshes, coarsest_idx - 1, bounds, config, materials, texture_config, "")
    } else {
        vec![]
    };

    TileNode {
        address: "root".into(),
        level: 0,
        bounds: *bounds,
        geometric_error: coarsest_error,
        content: Some(TileContent {
            glb_data: root_glb,
            uri: root_uri,
        }),
        children,
    }
}

/// Recursively build children for a LOD level.
///
/// For the finest level (LOD 0), octree-split into leaf tiles.
/// For intermediate levels, create a single tile with the level's mesh as content,
/// with children from the next finer level.
fn build_lod_children(
    level_meshes: &[(u32, IndexedMesh, f64)],
    current_idx: usize,
    bounds: &BoundingBox,
    config: &TilingConfig,
    materials: &MaterialLibrary,
    texture_config: &TextureConfig,
    parent_addr: &str,
) -> Vec<TileNode> {
    let (lod_level, ref mesh, geometric_error) = level_meshes[current_idx];

    if current_idx == 0 {
        // Finest LOD: octree-split into leaf tiles
        let tree = build_octree(mesh, bounds, config.max_depth, config.max_triangles_per_tile);
        return octree_children_to_tiles(&tree, bounds, 0, materials, texture_config);
    }

    // Intermediate LOD: single tile with content, children from next finer level
    let address = if parent_addr.is_empty() {
        format!("{lod_level}")
    } else {
        format!("{parent_addr}_{lod_level}")
    };

    let glb_data = write_tile_glb(mesh, materials, texture_config);
    let uri = address_to_uri(&address);

    let children = build_lod_children(
        level_meshes,
        current_idx - 1,
        bounds,
        config,
        materials,
        texture_config,
        &address,
    );

    vec![TileNode {
        address,
        level: lod_level,
        bounds: *bounds,
        geometric_error,
        content: Some(TileContent { glb_data, uri }),
        children,
    }]
}

/// Convert an octree into tile nodes for the leaf level of the LOD hierarchy.
fn octree_children_to_tiles(
    node: &OctreeNode,
    _bounds: &BoundingBox,
    child_counter: usize,
    materials: &MaterialLibrary,
    texture_config: &TextureConfig,
) -> Vec<TileNode> {
    if node.is_leaf() {
        if node.mesh.is_empty() {
            return vec![];
        }
        let address = format!("{child_counter}");
        let glb_data = write_tile_glb(&node.mesh, materials, texture_config);
        let uri = address_to_uri(&address);
        return vec![TileNode {
            address,
            level: 0,
            bounds: node.bounds,
            geometric_error: 0.0,
            content: Some(TileContent { glb_data, uri }),
            children: vec![],
        }];
    }

    // Internal octree node: recurse into children
    let mut tiles = Vec::new();
    let mut counter = child_counter;
    for child in &node.children {
        if let Some(c) = child.as_ref() {
            let sub = octree_to_tile_node_recursive(c, &mut counter, materials, texture_config);
            tiles.push(sub);
        }
    }
    tiles
}

/// Recursively convert an OctreeNode into a TileNode with proper addressing.
fn octree_to_tile_node_recursive(
    node: &OctreeNode,
    counter: &mut usize,
    materials: &MaterialLibrary,
    texture_config: &TextureConfig,
) -> TileNode {
    let address = format!("{counter}");
    *counter += 1;

    if node.is_leaf() {
        let content = if !node.mesh.is_empty() {
            let glb_data = write_tile_glb(&node.mesh, materials, texture_config);
            let uri = address_to_uri(&address);
            Some(TileContent { glb_data, uri })
        } else {
            None
        };

        return TileNode {
            address,
            level: 0,
            bounds: node.bounds,
            geometric_error: 0.0,
            content,
            children: vec![],
        };
    }

    // Internal octree node
    let geometric_error = node.bounds.diagonal() * 0.1;
    let mut children = Vec::new();
    for child in &node.children {
        if let Some(c) = child.as_ref() {
            children.push(octree_to_tile_node_recursive(c, counter, materials, texture_config));
        }
    }

    TileNode {
        address,
        level: 0,
        bounds: node.bounds,
        geometric_error,
        content: None,
        children,
    }
}

/// Write the tileset to disk: `tileset.json` + hierarchical `tiles/` directory.
///
/// Returns the total number of tiles written.
pub fn write_tileset(
    output: &TilesetOutput,
    transform: &[f64; 16],
    out_dir: &Path,
) -> Result<usize> {
    // Write all GLB tile files using parallel I/O
    let tile_count = write_tile_glbs_parallel(&output.root, out_dir)?;

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

/// Collect all (path, data) pairs from the tile tree.
fn collect_glb_pairs<'a>(node: &'a TileNode, out_dir: &Path, pairs: &mut Vec<(PathBuf, &'a [u8])>) {
    if let Some(content) = &node.content {
        let glb_path = out_dir.join(&content.uri);
        pairs.push((glb_path, &content.glb_data));
    }
    for child in &node.children {
        collect_glb_pairs(child, out_dir, pairs);
    }
}

/// Write GLB files in parallel using rayon.
fn write_tile_glbs_parallel(node: &TileNode, out_dir: &Path) -> Result<usize> {
    let mut pairs: Vec<(PathBuf, &[u8])> = Vec::new();
    collect_glb_pairs(node, out_dir, &mut pairs);

    // Create directories (sequential — fast and must happen before writes)
    for (path, _) in &pairs {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                PhotoTilerError::Output(format!(
                    "Failed to create dir {}: {e}",
                    parent.display()
                ))
            })?;
        }
    }

    // Write files in parallel
    pairs.par_iter().try_for_each(|(path, data)| {
        fs::write(path, data).map_err(|e| {
            PhotoTilerError::Output(format!("Failed to write {}: {e}", path.display()))
        })
    })?;

    Ok(pairs.len())
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

/// Convert an OctreeNode into a TileNode (used for single-level tilesets).
fn octree_to_tile_node(
    node: &OctreeNode,
    address: &str,
    level: u32,
    bounds: &BoundingBox,
    _parent_error: f64,
    materials: &MaterialLibrary,
    texture_config: &TextureConfig,
) -> TileNode {
    let geometric_error = if node.is_leaf() {
        0.0
    } else {
        // Internal nodes: error proportional to bounds diagonal
        bounds.diagonal() * 0.5_f64.powi(level as i32)
    };

    let content = if !node.mesh.is_empty() {
        let glb_data = write_tile_glb(&node.mesh, materials, texture_config);
        let uri = address_to_uri(address);
        Some(TileContent { glb_data, uri })
    } else {
        None
    };

    let children: Vec<TileNode> = node
        .children
        .iter()
        .enumerate()
        .filter_map(|(i, child)| {
            child.as_ref().map(|c| {
                let child_addr = format!("{address}_{i}");
                let child_bounds = &c.bounds;
                octree_to_tile_node(
                    c,
                    &child_addr,
                    level + 1,
                    child_bounds,
                    geometric_error,
                    materials,
                    texture_config,
                )
            })
        })
        .collect();

    TileNode {
        address: address.into(),
        level,
        bounds: *bounds,
        geometric_error,
        content,
        children,
    }
}

/// Merge two IndexedMeshes by concatenating their buffers and offsetting indices.
fn merge_meshes(a: &IndexedMesh, b: &IndexedMesh) -> IndexedMesh {
    if a.is_empty() {
        return b.clone();
    }
    if b.is_empty() {
        return a.clone();
    }

    let a_vertex_count = a.vertex_count() as u32;

    let mut positions = a.positions.clone();
    positions.extend_from_slice(&b.positions);

    let normals = if a.has_normals() && b.has_normals() {
        let mut n = a.normals.clone();
        n.extend_from_slice(&b.normals);
        n
    } else {
        vec![]
    };

    let uvs = if a.has_uvs() && b.has_uvs() {
        let mut u = a.uvs.clone();
        u.extend_from_slice(&b.uvs);
        u
    } else {
        vec![]
    };

    let colors = if a.has_colors() && b.has_colors() {
        let mut c = a.colors.clone();
        c.extend_from_slice(&b.colors);
        c
    } else {
        vec![]
    };

    let mut indices = a.indices.clone();
    indices.extend(b.indices.iter().map(|&i| i + a_vertex_count));

    IndexedMesh {
        positions,
        normals,
        uvs,
        colors,
        indices,
        material_index: a.material_index.or(b.material_index),
    }
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

        let tex_config = TextureConfig { enabled: false, ..Default::default() };
        let output = build_tileset(&[chain], &unit_bounds(), &config, &materials, &tex_config);
        assert_eq!(output.root.address, "root");
        assert_eq!(output.root.level, 0);
    }

    #[test]
    fn build_tileset_multi_level() {
        let mesh = make_grid_mesh(10); // 200 triangles
        let simplified = make_grid_mesh(4); // 32 triangles

        let chain = LodChain {
            levels: vec![
                LodLevel {
                    level: 0,
                    mesh: mesh.clone(),
                    geometric_error: 0.0,
                },
                LodLevel {
                    level: 1,
                    mesh: simplified.clone(),
                    geometric_error: 0.5,
                },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();

        let tex_config = TextureConfig { enabled: false, ..Default::default() };
        let output = build_tileset(&[chain], &unit_bounds(), &config, &materials, &tex_config);
        assert_eq!(output.root.address, "root");
        assert!(
            output.root.content.is_some(),
            "root should have content (coarsest LOD)"
        );
        assert!(
            output.root.geometric_error > 0.0,
            "root should have positive geometric error"
        );
        // Multi-level: root should have children (intermediate or leaf)
        assert!(
            !output.root.children.is_empty(),
            "multi-level tileset root should have children"
        );
    }

    #[test]
    fn build_tileset_four_lods() {
        let lod0 = make_grid_mesh(16); // 512 tris
        let lod1 = make_grid_mesh(8);  // 128 tris
        let lod2 = make_grid_mesh(4);  // 32 tris
        let lod3 = make_grid_mesh(2);  // 8 tris

        let chain = LodChain {
            levels: vec![
                LodLevel { level: 0, mesh: lod0, geometric_error: 0.0 },
                LodLevel { level: 1, mesh: lod1, geometric_error: 0.2 },
                LodLevel { level: 2, mesh: lod2, geometric_error: 0.5 },
                LodLevel { level: 3, mesh: lod3, geometric_error: 1.0 },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();

        let tex_config = TextureConfig { enabled: false, ..Default::default() };
        let output = build_tileset(&[chain], &unit_bounds(), &config, &materials, &tex_config);

        // Root should be coarsest (LOD 3)
        assert_eq!(output.root.address, "root");
        assert!(output.root.content.is_some());

        // Verify hierarchy depth >= 2 (root + at least intermediate + leaves)
        fn max_depth(node: &TileNode) -> usize {
            if node.children.is_empty() {
                1
            } else {
                1 + node.children.iter().map(max_depth).max().unwrap_or(0)
            }
        }
        let depth = max_depth(&output.root);
        assert!(depth >= 3, "4-LOD hierarchy should have depth >= 3, got {depth}");
    }

    #[test]
    fn geometric_error_decreasing() {
        let lod0 = make_grid_mesh(10);
        let lod1 = make_grid_mesh(4);
        let lod2 = make_grid_mesh(2);

        let chain = LodChain {
            levels: vec![
                LodLevel { level: 0, mesh: lod0, geometric_error: 0.0 },
                LodLevel { level: 1, mesh: lod1, geometric_error: 0.5 },
                LodLevel { level: 2, mesh: lod2, geometric_error: 1.0 },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();

        let tex_config = TextureConfig { enabled: false, ..Default::default() };
        let output = build_tileset(&[chain], &unit_bounds(), &config, &materials, &tex_config);

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

        let tex_config = TextureConfig { enabled: false, ..Default::default() };
        let output = build_tileset(&[chain], &unit_bounds(), &config, &materials, &tex_config);

        let tmp = tempfile::tempdir().unwrap();
        let transform = identity();
        let tile_count = write_tileset(&output, &transform, tmp.path()).unwrap();

        // Should have tileset.json
        assert!(tmp.path().join("tileset.json").exists());

        // Should have tiles directory
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

        let tex_config = TextureConfig { enabled: false, ..Default::default() };
        let output = build_tileset(&[chain], &unit_bounds(), &config, &materials, &tex_config);

        let tmp = tempfile::tempdir().unwrap();
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
        let materials = MaterialLibrary::default();

        let tex_config = TextureConfig { enabled: false, ..Default::default() };
        let output = build_tileset(&[chain], &unit_bounds(), &config, &materials, &tex_config);

        let transform = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 100.0, 200.0, 300.0,
            1.0,
        ];

        let tmp = tempfile::tempdir().unwrap();
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

        let merged = merge_meshes(&a, &b);
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

        let result = merge_meshes(&empty, &mesh);
        assert_eq!(result.positions.len(), mesh.positions.len());

        let result2 = merge_meshes(&mesh, &empty);
        assert_eq!(result2.positions.len(), mesh.positions.len());
    }

    #[test]
    fn hierarchical_dirs_created() {
        let lod0 = make_grid_mesh(10);
        let lod1 = make_grid_mesh(4);

        let chain = LodChain {
            levels: vec![
                LodLevel { level: 0, mesh: lod0, geometric_error: 0.0 },
                LodLevel { level: 1, mesh: lod1, geometric_error: 0.5 },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();

        let tex_config = TextureConfig { enabled: false, ..Default::default() };
        let output = build_tileset(&[chain], &unit_bounds(), &config, &materials, &tex_config);

        let tmp = tempfile::tempdir().unwrap();
        write_tileset(&output, &identity(), tmp.path()).unwrap();

        // tiles/ directory should exist
        assert!(tmp.path().join("tiles").exists());
        // tileset.json should exist
        assert!(tmp.path().join("tileset.json").exists());
    }

    #[test]
    fn all_uris_match_files() {
        let lod0 = make_grid_mesh(10);
        let lod1 = make_grid_mesh(4);

        let chain = LodChain {
            levels: vec![
                LodLevel { level: 0, mesh: lod0, geometric_error: 0.0 },
                LodLevel { level: 1, mesh: lod1, geometric_error: 0.5 },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();

        let tex_config = TextureConfig { enabled: false, ..Default::default() };
        let output = build_tileset(&[chain], &unit_bounds(), &config, &materials, &tex_config);

        let tmp = tempfile::tempdir().unwrap();
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

        // Every URI should map to an actual file
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
        let simplified = make_grid_mesh(4);

        let chain = LodChain {
            levels: vec![
                LodLevel {
                    level: 0,
                    mesh: mesh.clone(),
                    geometric_error: 0.0,
                },
                LodLevel {
                    level: 1,
                    mesh: simplified.clone(),
                    geometric_error: 0.5,
                },
            ],
            bounds: unit_bounds(),
        };

        let config = TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        };
        let materials = MaterialLibrary::default();

        let tex_config = TextureConfig { enabled: false, ..Default::default() };
        let output = build_tileset(&[chain], &unit_bounds(), &config, &materials, &tex_config);

        let tmp = tempfile::tempdir().unwrap();
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
        assert_eq!(glb_count, tile_count, "GLB file count should match tile_count");
    }
}
