use meshopt::{self, SimplifyOptions, VertexDataAdapter};

use crate::types::IndexedMesh;

/// Result of mesh simplification: new mesh + achieved error.
#[derive(Debug, Clone)]
pub struct SimplifiedMesh {
    pub mesh: IndexedMesh,
    pub achieved_error: f32,
}

/// Simplify a mesh to `target_ratio` of its original index count.
///
/// Only indices change; vertex attribute arrays are compacted to remove
/// unreferenced vertices via `compact_mesh`.
pub fn simplify_mesh(mesh: &IndexedMesh, target_ratio: f32, lock_border: bool) -> SimplifiedMesh {
    if mesh.is_empty() {
        return SimplifiedMesh {
            mesh: IndexedMesh::default(),
            achieved_error: 0.0,
        };
    }

    let positions_bytes = meshopt::typed_to_bytes(&mesh.positions);
    let adapter = VertexDataAdapter::new(positions_bytes, 12, 0)
        .expect("positions buffer should be valid for VertexDataAdapter");

    let target_count = (mesh.indices.len() as f64 * target_ratio as f64) as usize;
    // Ensure target_count is a multiple of 3 (whole triangles)
    let target_count = (target_count / 3) * 3;
    let target_error: f32 = 0.01;

    let options = if lock_border {
        SimplifyOptions::LockBorder
    } else {
        SimplifyOptions::None
    };

    let mut result_error: f32 = 0.0;
    let new_indices = meshopt::simplify(
        &mesh.indices,
        &adapter,
        target_count,
        target_error,
        options,
        Some(&mut result_error),
    );

    // Optimize for GPU: vertex cache then compact unused vertices
    let new_indices = meshopt::optimize_vertex_cache(&new_indices, mesh.vertex_count());

    let compacted = compact_mesh(new_indices, mesh);

    SimplifiedMesh {
        mesh: compacted,
        achieved_error: result_error,
    }
}

/// Remap indices to remove unreferenced vertices and rebuild attribute arrays.
///
/// Scans the index buffer to find referenced vertices, builds a compact remap,
/// then rebuilds positions/normals/uvs/colors with only referenced vertices.
pub fn compact_mesh(indices: Vec<u32>, source: &IndexedMesh) -> IndexedMesh {
    if indices.is_empty() {
        return IndexedMesh {
            material_index: source.material_index,
            ..Default::default()
        };
    }

    let vertex_count = source.vertex_count();

    // Build remap: old_index -> new_index (u32::MAX if unreferenced)
    let mut remap = vec![u32::MAX; vertex_count];
    let mut next_vertex: u32 = 0;
    for &idx in &indices {
        let i = idx as usize;
        if remap[i] == u32::MAX {
            remap[i] = next_vertex;
            next_vertex += 1;
        }
    }
    let new_vertex_count = next_vertex as usize;

    // Remap indices
    let new_indices: Vec<u32> = indices.iter().map(|&i| remap[i as usize]).collect();

    // Rebuild attribute arrays
    let mut new_positions = vec![0.0f32; new_vertex_count * 3];
    let mut new_normals = if source.has_normals() {
        vec![0.0f32; new_vertex_count * 3]
    } else {
        vec![]
    };
    let mut new_uvs = if source.has_uvs() {
        vec![0.0f32; new_vertex_count * 2]
    } else {
        vec![]
    };
    let mut new_colors = if source.has_colors() {
        vec![0.0f32; new_vertex_count * 4]
    } else {
        vec![]
    };

    for (old_idx, &new_idx) in remap.iter().enumerate() {
        if new_idx == u32::MAX {
            continue;
        }
        let ni = new_idx as usize;

        // Positions (stride 3)
        new_positions[ni * 3] = source.positions[old_idx * 3];
        new_positions[ni * 3 + 1] = source.positions[old_idx * 3 + 1];
        new_positions[ni * 3 + 2] = source.positions[old_idx * 3 + 2];

        // Normals (stride 3)
        if source.has_normals() {
            new_normals[ni * 3] = source.normals[old_idx * 3];
            new_normals[ni * 3 + 1] = source.normals[old_idx * 3 + 1];
            new_normals[ni * 3 + 2] = source.normals[old_idx * 3 + 2];
        }

        // UVs (stride 2)
        if source.has_uvs() {
            new_uvs[ni * 2] = source.uvs[old_idx * 2];
            new_uvs[ni * 2 + 1] = source.uvs[old_idx * 2 + 1];
        }

        // Colors (stride 4)
        if source.has_colors() {
            new_colors[ni * 4] = source.colors[old_idx * 4];
            new_colors[ni * 4 + 1] = source.colors[old_idx * 4 + 1];
            new_colors[ni * 4 + 2] = source.colors[old_idx * 4 + 2];
            new_colors[ni * 4 + 3] = source.colors[old_idx * 4 + 3];
        }
    }

    IndexedMesh {
        positions: new_positions,
        normals: new_normals,
        uvs: new_uvs,
        colors: new_colors,
        indices: new_indices,
        material_index: source.material_index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a flat grid mesh with `n x n` quads (2 triangles each).
    fn make_grid(n: usize) -> IndexedMesh {
        let verts_per_side = n + 1;
        let vertex_count = verts_per_side * verts_per_side;
        let mut positions = Vec::with_capacity(vertex_count * 3);
        let mut normals = Vec::with_capacity(vertex_count * 3);
        let mut uvs = Vec::with_capacity(vertex_count * 2);

        for y in 0..verts_per_side {
            for x in 0..verts_per_side {
                let fx = x as f32 / n as f32;
                let fy = y as f32 / n as f32;
                positions.extend_from_slice(&[fx, fy, 0.0]);
                normals.extend_from_slice(&[0.0, 0.0, 1.0]);
                uvs.extend_from_slice(&[fx, fy]);
            }
        }

        let mut indices = Vec::with_capacity(n * n * 6);
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
            normals,
            uvs,
            colors: vec![],
            indices,
            material_index: None,
        }
    }

    #[test]
    fn simplify_reduces_triangle_count() {
        let mesh = make_grid(50); // 50x50 = 2500 quads = 5000 triangles
        assert_eq!(mesh.triangle_count(), 5000);

        let result = simplify_mesh(&mesh, 0.5, false);
        // Should have meaningfully fewer triangles
        assert!(result.mesh.triangle_count() < mesh.triangle_count());
        assert!(result.mesh.triangle_count() > 0);
    }

    #[test]
    fn simplify_preserves_attributes() {
        let mesh = make_grid(20);
        let result = simplify_mesh(&mesh, 0.5, false);

        // Simplified mesh should still have normals and UVs
        assert!(result.mesh.has_normals());
        assert!(result.mesh.has_uvs());
        // Vertex count should match attribute array sizes
        assert_eq!(result.mesh.normals.len(), result.mesh.positions.len());
        assert_eq!(
            result.mesh.uvs.len(),
            result.mesh.vertex_count() * 2
        );
    }

    #[test]
    fn simplify_empty_mesh() {
        let mesh = IndexedMesh::default();
        let result = simplify_mesh(&mesh, 0.5, false);
        assert!(result.mesh.is_empty());
        assert_eq!(result.achieved_error, 0.0);
    }

    #[test]
    fn simplify_with_lock_border() {
        let mesh = make_grid(30);
        let result = simplify_mesh(&mesh, 0.25, true);
        assert!(result.mesh.triangle_count() < mesh.triangle_count());
        assert!(result.mesh.triangle_count() > 0);
    }

    #[test]
    fn compact_mesh_removes_unreferenced() {
        // Create a mesh with 4 vertices but only use 3 (one triangle)
        let source = IndexedMesh {
            positions: vec![
                0.0, 0.0, 0.0, // v0
                1.0, 0.0, 0.0, // v1
                0.0, 1.0, 0.0, // v2
                9.0, 9.0, 9.0, // v3 -- unreferenced
            ],
            normals: vec![
                0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0,
            ],
            uvs: vec![],
            colors: vec![],
            indices: vec![0, 1, 2],
            material_index: Some(0),
        };

        let compacted = compact_mesh(vec![0, 1, 2], &source);
        assert_eq!(compacted.vertex_count(), 3);
        assert_eq!(compacted.triangle_count(), 1);
        assert!(compacted.has_normals());
        assert_eq!(compacted.material_index, Some(0));
    }

    #[test]
    fn simplify_aggressive_ratio() {
        let mesh = make_grid(100); // 10000 quads = 20000 triangles
        let result = simplify_mesh(&mesh, 0.01, false);
        // Even at 1% target, should produce valid geometry
        assert!(result.mesh.triangle_count() > 0);
        assert!(result.mesh.triangle_count() < mesh.triangle_count());
    }
}
