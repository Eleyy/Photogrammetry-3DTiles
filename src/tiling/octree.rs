use rayon::prelude::*;

use crate::types::{BoundingBox, IndexedMesh};

/// A node in the octree spatial hierarchy.
#[derive(Debug, Clone)]
pub struct OctreeNode {
    pub bounds: BoundingBox,
    pub mesh: IndexedMesh,
    pub children: [Option<Box<OctreeNode>>; 8],
}

impl OctreeNode {
    /// Whether this node is a leaf (no children).
    pub fn is_leaf(&self) -> bool {
        self.children.iter().all(|c| c.is_none())
    }

    /// Count total nodes in the subtree (including self).
    pub fn node_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .filter_map(|c| c.as_ref())
            .map(|c| c.node_count())
            .sum::<usize>()
    }

    /// Count total triangles in the subtree.
    pub fn total_triangles(&self) -> usize {
        self.mesh.triangle_count()
            + self
                .children
                .iter()
                .filter_map(|c| c.as_ref())
                .map(|c| c.total_triangles())
                .sum::<usize>()
    }
}

/// Compute the octant index (0..7) for a point relative to the center of a bounding box.
///
/// Octant layout (bit pattern: z_hi | y_hi | x_hi):
///   0 = (lo, lo, lo), 1 = (hi, lo, lo), 2 = (lo, hi, lo), 3 = (hi, hi, lo)
///   4 = (lo, lo, hi), 5 = (hi, lo, hi), 6 = (lo, hi, hi), 7 = (hi, hi, hi)
pub(crate) fn octant_index(center: [f64; 3], point: [f64; 3]) -> usize {
    let mut idx = 0;
    if point[0] >= center[0] {
        idx |= 1;
    }
    if point[1] >= center[1] {
        idx |= 2;
    }
    if point[2] >= center[2] {
        idx |= 4;
    }
    idx
}

/// Compute the child bounding box for a given octant index.
pub(crate) fn child_bounds(parent: &BoundingBox, octant: usize) -> BoundingBox {
    let c = parent.center();
    let min_x = if octant & 1 != 0 { c[0] } else { parent.min[0] };
    let max_x = if octant & 1 != 0 { parent.max[0] } else { c[0] };
    let min_y = if octant & 2 != 0 { c[1] } else { parent.min[1] };
    let max_y = if octant & 2 != 0 { parent.max[1] } else { c[1] };
    let min_z = if octant & 4 != 0 { c[2] } else { parent.min[2] };
    let max_z = if octant & 4 != 0 { parent.max[2] } else { c[2] };

    BoundingBox {
        min: [min_x, min_y, min_z],
        max: [max_x, max_y, max_z],
    }
}

/// Split a mesh into 8 octant sub-meshes using Sutherland-Hodgman clipping.
///
/// Triangles straddling octant boundaries are clipped at the boundary planes
/// and the resulting sub-polygons are fan-triangulated into the appropriate
/// octant. Interior triangles (all vertices in one octant) take a fast path.
pub fn split_mesh(mesh: &IndexedMesh, bounds: &BoundingBox) -> [IndexedMesh; 8] {
    crate::tiling::triangle_clipper::split_mesh_clipping(mesh, bounds)
}

/// Recursively build an octree from a mesh.
///
/// Takes ownership of the mesh to avoid unnecessary clones of large buffers.
/// Subdivides if `triangle_count > max_triangles` AND `depth < max_depth`.
/// Otherwise the node becomes a leaf containing its mesh.
pub fn build_octree(
    mesh: IndexedMesh,
    bounds: &BoundingBox,
    max_depth: u32,
    max_triangles: usize,
) -> OctreeNode {
    build_octree_recursive(mesh, bounds, 0, max_depth, max_triangles)
}

fn build_octree_recursive(
    mesh: IndexedMesh,
    bounds: &BoundingBox,
    depth: u32,
    max_depth: u32,
    max_triangles: usize,
) -> OctreeNode {
    // Leaf condition: few enough triangles or at max depth
    if mesh.triangle_count() <= max_triangles || depth >= max_depth {
        return OctreeNode {
            bounds: *bounds,
            mesh, // move, no clone
            children: Default::default(),
        };
    }

    let sub_meshes = split_mesh(&mesh, bounds);
    drop(mesh); // free parent mesh before recursing into children

    // Convert [IndexedMesh; 8] to Vec of (index, mesh) pairs for parallel processing
    let bounds_copy = *bounds;
    let child_vec: Vec<Option<Box<OctreeNode>>> = sub_meshes
        .into_iter()
        .enumerate()
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|(i, sub)| {
            if sub.is_empty() {
                None
            } else {
                let cb = child_bounds(&bounds_copy, i);
                Some(Box::new(build_octree_recursive(
                    sub,
                    &cb,
                    depth + 1,
                    max_depth,
                    max_triangles,
                )))
            }
        })
        .collect();

    let children: [Option<Box<OctreeNode>>; 8] = child_vec
        .try_into()
        .expect("parallel octree should produce exactly 8 children");

    OctreeNode {
        bounds: *bounds,
        mesh: IndexedMesh::default(), // internal nodes have no mesh
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a 3D grid mesh spanning [0,1]^3.
    /// Creates `n x n x n` cubes, each face triangulated as 2 triangles.
    /// For simpler tests, we use a flat XY grid at varying Z.
    fn make_3d_grid(n: usize) -> (IndexedMesh, BoundingBox) {
        let verts_per_side = n + 1;
        let total_verts = verts_per_side * verts_per_side * verts_per_side;
        let mut positions = Vec::with_capacity(total_verts * 3);

        for z in 0..verts_per_side {
            for y in 0..verts_per_side {
                for x in 0..verts_per_side {
                    let fx = x as f32 / n as f32;
                    let fy = y as f32 / n as f32;
                    let fz = z as f32 / n as f32;
                    positions.extend_from_slice(&[fx, fy, fz]);
                }
            }
        }

        // Create triangles from the XY faces at each Z layer
        let mut indices = Vec::new();
        for z in 0..verts_per_side {
            for y in 0..n {
                for x in 0..n {
                    let v = |x: usize, y: usize, z: usize| -> u32 {
                        (z * verts_per_side * verts_per_side + y * verts_per_side + x) as u32
                    };
                    let tl = v(x, y, z);
                    let tr = v(x + 1, y, z);
                    let bl = v(x, y + 1, z);
                    let br = v(x + 1, y + 1, z);
                    indices.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
                }
            }
        }

        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let mesh = IndexedMesh {
            positions,
            indices,
            ..Default::default()
        };

        (mesh, bounds)
    }

    /// Generate a flat grid on XY at z=0.5 for simpler 2D-like tests.
    fn make_flat_grid(n: usize) -> (IndexedMesh, BoundingBox) {
        let verts_per_side = n + 1;
        let mut positions = Vec::with_capacity(verts_per_side * verts_per_side * 3);

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

        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let mesh = IndexedMesh {
            positions,
            indices,
            ..Default::default()
        };

        (mesh, bounds)
    }

    #[test]
    fn split_mesh_preserves_all_triangles() {
        let (mesh, bounds) = make_3d_grid(4);
        let original_tris = mesh.triangle_count();
        assert!(original_tris > 0);

        let children = split_mesh(&mesh, &bounds);
        let total: usize = children.iter().map(|m| m.triangle_count()).sum();
        // Clipping can produce MORE triangles than original (boundary splits)
        assert!(total >= original_tris, "clipped output ({total}) must have >= original ({original_tris}) triangles");
    }

    #[test]
    fn split_mesh_clipping_no_gaps() {
        // Every original vertex position should appear in the output
        let mesh = IndexedMesh {
            positions: vec![0.25, 0.25, 0.25, 0.75, 0.25, 0.25, 0.5, 0.75, 0.25],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let children = split_mesh(&mesh, &bounds);

        // Collect all output vertex positions
        let mut all_output_positions = Vec::new();
        for child in &children {
            for vi in 0..child.vertex_count() {
                all_output_positions.push([
                    child.positions[vi * 3],
                    child.positions[vi * 3 + 1],
                    child.positions[vi * 3 + 2],
                ]);
            }
        }

        // Each original vertex should appear in output (within epsilon)
        for vi in 0..mesh.vertex_count() {
            let orig = [
                mesh.positions[vi * 3],
                mesh.positions[vi * 3 + 1],
                mesh.positions[vi * 3 + 2],
            ];
            let found = all_output_positions.iter().any(|p| {
                (p[0] - orig[0]).abs() < 1e-4
                    && (p[1] - orig[1]).abs() < 1e-4
                    && (p[2] - orig[2]).abs() < 1e-4
            });
            assert!(found, "original vertex {orig:?} should appear in clipped output");
        }
    }

    #[test]
    fn split_mesh_empty_input() {
        let mesh = IndexedMesh::default();
        let bounds = BoundingBox {
            min: [0.0; 3],
            max: [1.0; 3],
        };
        let children = split_mesh(&mesh, &bounds);
        for child in &children {
            assert!(child.is_empty());
        }
    }

    #[test]
    fn split_mesh_single_triangle_interior() {
        // Triangle fully within octant 0 (all coords < 0.5 in unit box)
        let mesh = IndexedMesh {
            positions: vec![0.1, 0.1, 0.1, 0.3, 0.1, 0.1, 0.1, 0.3, 0.1],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let children = split_mesh(&mesh, &bounds);
        let total: usize = children.iter().map(|m| m.triangle_count()).sum();
        assert_eq!(total, 1, "interior triangle stays as 1 triangle");
    }

    #[test]
    fn split_mesh_single_triangle_boundary() {
        // Triangle spanning from (0,0,0) to (1,0,0) to (0,1,0) crosses boundaries
        let mesh = IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let children = split_mesh(&mesh, &bounds);
        let total: usize = children.iter().map(|m| m.triangle_count()).sum();
        // Clipping produces more triangles from boundary splits
        assert!(total >= 1, "boundary triangle should produce ≥1 total triangles, got {total}");
        let non_empty = children.iter().filter(|m| !m.is_empty()).count();
        assert!(non_empty >= 2, "boundary triangle should span ≥2 octants, got {non_empty}");
    }

    #[test]
    fn split_distributes_across_octants_3d() {
        let (mesh, bounds) = make_3d_grid(4);
        let children = split_mesh(&mesh, &bounds);

        // With a 3D grid spanning the full box, triangles should land in multiple octants
        let non_empty = children.iter().filter(|m| !m.is_empty()).count();
        assert!(
            non_empty >= 4,
            "3D grid should distribute across at least 4 octants, got {non_empty}"
        );
    }

    #[test]
    fn octant_bounds_correct() {
        let parent = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [2.0, 4.0, 6.0],
        };
        let center = parent.center(); // [1.0, 2.0, 3.0]

        // Octant 0: (lo, lo, lo) → min=[0,0,0], max=[1,2,3]
        let b0 = child_bounds(&parent, 0);
        assert_eq!(b0.min, [0.0, 0.0, 0.0]);
        assert_eq!(b0.max, center);

        // Octant 7: (hi, hi, hi) → min=[1,2,3], max=[2,4,6]
        let b7 = child_bounds(&parent, 7);
        assert_eq!(b7.min, center);
        assert_eq!(b7.max, [2.0, 4.0, 6.0]);

        // Octant 1: (hi, lo, lo) → x=[1,2], y=[0,2], z=[0,3]
        let b1 = child_bounds(&parent, 1);
        assert_eq!(b1.min, [1.0, 0.0, 0.0]);
        assert_eq!(b1.max, [2.0, 2.0, 3.0]);
    }

    #[test]
    fn build_octree_leaf_when_few_triangles() {
        let (mesh, bounds) = make_flat_grid(4); // 32 triangles
        let tree = build_octree(mesh, &bounds, 6, 100);

        // 32 < 100 → should be a leaf
        assert!(tree.is_leaf());
        assert_eq!(tree.mesh.triangle_count(), 32);
    }

    #[test]
    fn build_octree_leaf_at_max_depth() {
        let (mesh, bounds) = make_3d_grid(4);
        let tris = mesh.triangle_count();
        let tree = build_octree(mesh, &bounds, 0, 1); // max_depth=0 → immediate leaf

        assert!(tree.is_leaf());
        assert_eq!(tree.mesh.triangle_count(), tris);
    }

    #[test]
    fn build_octree_subdivides_large_mesh() {
        let (mesh, bounds) = make_3d_grid(8);
        let original_tris = mesh.triangle_count();

        // Set max_triangles low enough to force splitting
        let tree = build_octree(mesh, &bounds, 4, 50);

        assert!(!tree.is_leaf(), "large mesh should be subdivided");
        assert!(tree.node_count() > 1);

        // All triangles should be in leaf nodes (clipping may produce more)
        assert!(tree.total_triangles() >= original_tris);
    }

    #[test]
    fn build_octree_preserves_attributes() {
        let n = 4;
        let verts_per_side = n + 1;
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();

        for z in 0..verts_per_side {
            for y in 0..verts_per_side {
                for x in 0..verts_per_side {
                    let fx = x as f32 / n as f32;
                    let fy = y as f32 / n as f32;
                    let fz = z as f32 / n as f32;
                    positions.extend_from_slice(&[fx, fy, fz]);
                    normals.extend_from_slice(&[0.0, 0.0, 1.0]);
                    uvs.extend_from_slice(&[fx, fy]);
                }
            }
        }

        let mut indices = Vec::new();
        for z in 0..verts_per_side {
            for y in 0..n {
                for x in 0..n {
                    let v = |x: usize, y: usize, z: usize| -> u32 {
                        (z * verts_per_side * verts_per_side + y * verts_per_side + x) as u32
                    };
                    let tl = v(x, y, z);
                    let tr = v(x + 1, y, z);
                    let bl = v(x, y + 1, z);
                    let br = v(x + 1, y + 1, z);
                    indices.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
                }
            }
        }

        let mesh = IndexedMesh {
            positions,
            normals,
            uvs,
            colors: vec![],
            indices,
            material_index: Some(0),
        };

        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let children = split_mesh(&mesh, &bounds);
        for child in &children {
            if child.is_empty() {
                continue;
            }
            assert!(child.has_normals(), "normals should be preserved");
            assert!(child.has_uvs(), "UVs should be preserved");
            assert_eq!(child.normals.len(), child.positions.len());
            assert_eq!(child.uvs.len(), child.vertex_count() * 2);
            assert_eq!(child.material_index, Some(0));
        }
    }
}
