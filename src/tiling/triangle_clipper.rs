use std::collections::HashMap;

use crate::tiling::octree::{child_bounds, octant_index};
use crate::types::{BoundingBox, IndexedMesh};

/// Working vertex for clipping (f64 precision for math, cast to f32 at output).
#[derive(Debug, Clone)]
struct ClipVertex {
    pos: [f64; 3],
    normal: [f64; 3],
    uv: [f64; 2],
    color: [f64; 4],
}

/// Axis-aligned clipping half-plane.
struct ClipPlane {
    axis: usize,  // 0=X, 1=Y, 2=Z
    value: f64,
    positive: bool, // true = keep where pos[axis] >= value
}

/// Quantized position key for deduplication at boundaries (1µm precision).
#[derive(Hash, Eq, PartialEq)]
struct PositionKey([i64; 3]);

impl PositionKey {
    fn from_pos(pos: [f64; 3]) -> Self {
        Self([
            (pos[0] * 1e6).round() as i64,
            (pos[1] * 1e6).round() as i64,
            (pos[2] * 1e6).round() as i64,
        ])
    }
}

/// Extract a ClipVertex from an IndexedMesh at a given vertex index, promoting f32 → f64.
fn extract_clip_vertex(mesh: &IndexedMesh, vertex_index: usize) -> ClipVertex {
    let pos = [
        mesh.positions[vertex_index * 3] as f64,
        mesh.positions[vertex_index * 3 + 1] as f64,
        mesh.positions[vertex_index * 3 + 2] as f64,
    ];

    let normal = if mesh.has_normals() {
        [
            mesh.normals[vertex_index * 3] as f64,
            mesh.normals[vertex_index * 3 + 1] as f64,
            mesh.normals[vertex_index * 3 + 2] as f64,
        ]
    } else {
        [0.0; 3]
    };

    let uv = if mesh.has_uvs() {
        [
            mesh.uvs[vertex_index * 2] as f64,
            mesh.uvs[vertex_index * 2 + 1] as f64,
        ]
    } else {
        [0.0; 2]
    };

    let color = if mesh.has_colors() {
        [
            mesh.colors[vertex_index * 4] as f64,
            mesh.colors[vertex_index * 4 + 1] as f64,
            mesh.colors[vertex_index * 4 + 2] as f64,
            mesh.colors[vertex_index * 4 + 3] as f64,
        ]
    } else {
        [0.0; 4]
    };

    ClipVertex { pos, normal, uv, color }
}

/// Compute parametric intersection of edge (a→b) with a clipping plane, lerp ALL attributes.
fn intersect_edge(a: &ClipVertex, b: &ClipVertex, plane: &ClipPlane) -> ClipVertex {
    let da = a.pos[plane.axis] - plane.value;
    let db = b.pos[plane.axis] - plane.value;
    let denom = da - db;
    let t = if denom.abs() < 1e-15 { 0.5 } else { da / denom };

    let lerp = |a_val: f64, b_val: f64| a_val + t * (b_val - a_val);

    let pos = [
        lerp(a.pos[0], b.pos[0]),
        lerp(a.pos[1], b.pos[1]),
        lerp(a.pos[2], b.pos[2]),
    ];

    let normal = {
        let n = [
            lerp(a.normal[0], b.normal[0]),
            lerp(a.normal[1], b.normal[1]),
            lerp(a.normal[2], b.normal[2]),
        ];
        // Renormalize if non-zero
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 1e-12 {
            [n[0] / len, n[1] / len, n[2] / len]
        } else {
            n
        }
    };

    let uv = [
        lerp(a.uv[0], b.uv[0]),
        lerp(a.uv[1], b.uv[1]),
    ];

    let color = [
        lerp(a.color[0], b.color[0]),
        lerp(a.color[1], b.color[1]),
        lerp(a.color[2], b.color[2]),
        lerp(a.color[3], b.color[3]),
    ];

    ClipVertex { pos, normal, uv, color }
}

/// Sutherland-Hodgman: clip a polygon by a single half-plane.
fn clip_polygon_by_plane(polygon: &[ClipVertex], plane: &ClipPlane) -> Vec<ClipVertex> {
    if polygon.is_empty() {
        return Vec::new();
    }

    let is_inside = |v: &ClipVertex| {
        if plane.positive {
            v.pos[plane.axis] >= plane.value - 1e-10
        } else {
            v.pos[plane.axis] <= plane.value + 1e-10
        }
    };

    let mut output = Vec::new();
    let n = polygon.len();

    for i in 0..n {
        let current = &polygon[i];
        let next = &polygon[(i + 1) % n];
        let cur_in = is_inside(current);
        let nxt_in = is_inside(next);

        match (cur_in, nxt_in) {
            (true, true) => {
                // Both inside: emit next
                output.push(next.clone());
            }
            (true, false) => {
                // Going out: emit intersection
                output.push(intersect_edge(current, next, plane));
            }
            (false, true) => {
                // Coming in: emit intersection + next
                output.push(intersect_edge(current, next, plane));
                output.push(next.clone());
            }
            (false, false) => {
                // Both outside: emit nothing
            }
        }
    }

    output
}

/// Clip a triangle against the 6 AABB planes of one octant.
fn clip_triangle_to_octant(tri: [ClipVertex; 3], octant_bounds: &BoundingBox) -> Vec<ClipVertex> {
    let planes = [
        ClipPlane { axis: 0, value: octant_bounds.min[0], positive: true },
        ClipPlane { axis: 0, value: octant_bounds.max[0], positive: false },
        ClipPlane { axis: 1, value: octant_bounds.min[1], positive: true },
        ClipPlane { axis: 1, value: octant_bounds.max[1], positive: false },
        ClipPlane { axis: 2, value: octant_bounds.min[2], positive: true },
        ClipPlane { axis: 2, value: octant_bounds.max[2], positive: false },
    ];

    let mut polygon: Vec<ClipVertex> = tri.into();

    for plane in &planes {
        polygon = clip_polygon_by_plane(&polygon, plane);
        if polygon.is_empty() {
            return polygon;
        }
    }

    polygon
}

/// Fan-triangulate a convex polygon from vertex 0. Skip degenerate (<3 verts).
fn fan_triangulate(polygon: &[ClipVertex]) -> Vec<[ClipVertex; 3]> {
    if polygon.len() < 3 {
        return Vec::new();
    }

    let mut tris = Vec::with_capacity(polygon.len() - 2);
    for i in 1..polygon.len() - 1 {
        tris.push([
            polygon[0].clone(),
            polygon[i].clone(),
            polygon[i + 1].clone(),
        ]);
    }
    tris
}

/// Accumulator for building an IndexedMesh per octant with vertex deduplication.
struct OctantMeshBuilder {
    positions: Vec<f32>,
    normals: Vec<f32>,
    uvs: Vec<f32>,
    colors: Vec<f32>,
    indices: Vec<u32>,
    dedup: HashMap<PositionKey, u32>,
    has_normals: bool,
    has_uvs: bool,
    has_colors: bool,
}

impl OctantMeshBuilder {
    fn new(has_normals: bool, has_uvs: bool, has_colors: bool) -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            colors: Vec::new(),
            indices: Vec::new(),
            dedup: HashMap::new(),
            has_normals,
            has_uvs,
            has_colors,
        }
    }

    /// Add a vertex (dedup by quantized position), return its index.
    fn add_vertex(&mut self, v: &ClipVertex) -> u32 {
        let key = PositionKey::from_pos(v.pos);
        if let Some(&idx) = self.dedup.get(&key) {
            return idx;
        }

        let idx = (self.positions.len() / 3) as u32;
        self.positions.extend_from_slice(&[v.pos[0] as f32, v.pos[1] as f32, v.pos[2] as f32]);

        if self.has_normals {
            self.normals.extend_from_slice(&[v.normal[0] as f32, v.normal[1] as f32, v.normal[2] as f32]);
        }
        if self.has_uvs {
            self.uvs.extend_from_slice(&[v.uv[0] as f32, v.uv[1] as f32]);
        }
        if self.has_colors {
            self.colors.extend_from_slice(&[v.color[0] as f32, v.color[1] as f32, v.color[2] as f32, v.color[3] as f32]);
        }

        self.dedup.insert(key, idx);
        idx
    }

    /// Add a triangle from 3 ClipVertices. Skips degenerate (collapsed indices).
    fn add_triangle(&mut self, a: &ClipVertex, b: &ClipVertex, c: &ClipVertex) {
        let ia = self.add_vertex(a);
        let ib = self.add_vertex(b);
        let ic = self.add_vertex(c);
        // Skip degenerate triangles
        if ia != ib && ib != ic && ia != ic {
            self.indices.extend_from_slice(&[ia, ib, ic]);
        }
    }

    /// Build the final IndexedMesh.
    fn build(self, material_index: Option<usize>) -> IndexedMesh {
        IndexedMesh {
            positions: self.positions,
            normals: self.normals,
            uvs: self.uvs,
            colors: self.colors,
            indices: self.indices,
            material_index,
        }
    }
}

/// Split a mesh into 8 octant sub-meshes using Sutherland-Hodgman clipping.
///
/// Triangles straddling octant boundaries are clipped and the resulting
/// sub-polygons are fan-triangulated into the appropriate octant. Interior
/// triangles (all 3 vertices in the same octant) take a fast path that skips
/// clipping entirely.
pub fn split_mesh_clipping(mesh: &IndexedMesh, bounds: &BoundingBox) -> [IndexedMesh; 8] {
    let center = bounds.center();
    let child_boxes: [BoundingBox; 8] = std::array::from_fn(|i| child_bounds(bounds, i));

    let mut builders: [OctantMeshBuilder; 8] = std::array::from_fn(|_| {
        OctantMeshBuilder::new(mesh.has_normals(), mesh.has_uvs(), mesh.has_colors())
    });

    for tri in mesh.indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;

        let p0 = [
            mesh.positions[i0 * 3] as f64,
            mesh.positions[i0 * 3 + 1] as f64,
            mesh.positions[i0 * 3 + 2] as f64,
        ];
        let p1 = [
            mesh.positions[i1 * 3] as f64,
            mesh.positions[i1 * 3 + 1] as f64,
            mesh.positions[i1 * 3 + 2] as f64,
        ];
        let p2 = [
            mesh.positions[i2 * 3] as f64,
            mesh.positions[i2 * 3 + 1] as f64,
            mesh.positions[i2 * 3 + 2] as f64,
        ];

        let oct0 = octant_index(center, p0);
        let oct1 = octant_index(center, p1);
        let oct2 = octant_index(center, p2);

        if oct0 == oct1 && oct1 == oct2 {
            // Fast path: all vertices in same octant — no clipping needed
            let v0 = extract_clip_vertex(mesh, i0);
            let v1 = extract_clip_vertex(mesh, i1);
            let v2 = extract_clip_vertex(mesh, i2);
            builders[oct0].add_triangle(&v0, &v1, &v2);
        } else {
            // Slow path: triangle straddles boundary — clip against each candidate octant
            let v0 = extract_clip_vertex(mesh, i0);
            let v1 = extract_clip_vertex(mesh, i1);
            let v2 = extract_clip_vertex(mesh, i2);

            // Only clip against octants that the triangle might touch.
            // The triangle can only be in octants covered by its vertices' octant indices.
            // For simplicity and correctness, test all 8 octants for boundary triangles.
            for (oct_idx, cb) in child_boxes.iter().enumerate() {
                let clipped = clip_triangle_to_octant(
                    [v0.clone(), v1.clone(), v2.clone()],
                    cb,
                );
                let sub_tris = fan_triangulate(&clipped);
                for sub_tri in &sub_tris {
                    builders[oct_idx].add_triangle(&sub_tri[0], &sub_tri[1], &sub_tri[2]);
                }
            }
        }
    }

    let material_index = mesh.material_index;
    std::array::from_fn(|i| {
        std::mem::replace(
            &mut builders[i],
            OctantMeshBuilder::new(false, false, false),
        )
        .build(material_index)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_polygon_fully_inside() {
        let polygon = vec![
            ClipVertex { pos: [0.2, 0.2, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
            ClipVertex { pos: [0.4, 0.2, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
            ClipVertex { pos: [0.3, 0.4, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
        ];
        let plane = ClipPlane { axis: 0, value: 0.0, positive: true };
        let result = clip_polygon_by_plane(&polygon, &plane);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn clip_polygon_fully_outside() {
        let polygon = vec![
            ClipVertex { pos: [-0.5, 0.2, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
            ClipVertex { pos: [-0.3, 0.2, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
            ClipVertex { pos: [-0.4, 0.4, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
        ];
        let plane = ClipPlane { axis: 0, value: 0.0, positive: true };
        let result = clip_polygon_by_plane(&polygon, &plane);
        assert!(result.is_empty());
    }

    #[test]
    fn clip_polygon_one_vertex_out() {
        // Triangle with 2 verts inside (x >= 0) and 1 outside
        let polygon = vec![
            ClipVertex { pos: [0.5, 0.0, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
            ClipVertex { pos: [0.5, 1.0, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
            ClipVertex { pos: [-0.5, 0.5, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
        ];
        let plane = ClipPlane { axis: 0, value: 0.0, positive: true };
        let result = clip_polygon_by_plane(&polygon, &plane);
        assert_eq!(result.len(), 4, "clipping one vertex out should produce a quad");
    }

    #[test]
    fn clip_polygon_two_vertices_out() {
        // Triangle with 1 vert inside (x >= 0.5) and 2 outside
        let polygon = vec![
            ClipVertex { pos: [1.0, 0.5, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
            ClipVertex { pos: [0.0, 0.0, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
            ClipVertex { pos: [0.0, 1.0, 0.0], normal: [0.0; 3], uv: [0.0; 2], color: [0.0; 4] },
        ];
        let plane = ClipPlane { axis: 0, value: 0.5, positive: true };
        let result = clip_polygon_by_plane(&polygon, &plane);
        assert_eq!(result.len(), 3, "clipping two vertices out should produce a triangle");
    }

    #[test]
    fn intersect_edge_midpoint() {
        let a = ClipVertex {
            pos: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            color: [1.0, 0.0, 0.0, 1.0],
        };
        let b = ClipVertex {
            pos: [1.0, 1.0, 1.0],
            normal: [1.0, 0.0, 0.0],
            uv: [1.0, 1.0],
            color: [0.0, 1.0, 0.0, 1.0],
        };
        let plane = ClipPlane { axis: 0, value: 0.5, positive: true };
        let v = intersect_edge(&a, &b, &plane);

        // Position at midpoint
        assert!((v.pos[0] - 0.5).abs() < 1e-10);
        assert!((v.pos[1] - 0.5).abs() < 1e-10);
        assert!((v.pos[2] - 0.5).abs() < 1e-10);

        // UV at midpoint
        assert!((v.uv[0] - 0.5).abs() < 1e-10);
        assert!((v.uv[1] - 0.5).abs() < 1e-10);

        // Color at midpoint
        assert!((v.color[0] - 0.5).abs() < 1e-10);
        assert!((v.color[1] - 0.5).abs() < 1e-10);

        // Normal lerped and renormalized
        let nlen = (v.normal[0] * v.normal[0] + v.normal[1] * v.normal[1] + v.normal[2] * v.normal[2]).sqrt();
        assert!((nlen - 1.0).abs() < 1e-10, "normal should be unit length after renormalize");
    }

    #[test]
    fn fan_triangulate_pentagon() {
        let pentagon: Vec<ClipVertex> = (0..5)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / 5.0;
                ClipVertex {
                    pos: [angle.cos(), angle.sin(), 0.0],
                    normal: [0.0; 3],
                    uv: [0.0; 2],
                    color: [0.0; 4],
                }
            })
            .collect();

        let tris = fan_triangulate(&pentagon);
        assert_eq!(tris.len(), 3, "pentagon → 3 triangles");
    }

    #[test]
    fn split_mesh_all_in_one_octant() {
        // Tiny triangle in octant 0 (all coords < 0.5 in unit box)
        let mesh = IndexedMesh {
            positions: vec![0.1, 0.1, 0.1, 0.2, 0.1, 0.1, 0.1, 0.2, 0.1],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let children = split_mesh_clipping(&mesh, &bounds);
        let non_empty: Vec<usize> = children.iter().enumerate()
            .filter(|(_, m)| !m.is_empty())
            .map(|(i, _)| i)
            .collect();

        assert_eq!(non_empty.len(), 1, "should be in exactly 1 octant");
        assert_eq!(non_empty[0], 0, "should be in octant 0");
        assert_eq!(children[0].triangle_count(), 1);
    }

    #[test]
    fn split_mesh_boundary_triangle() {
        // Triangle straddling the center of the bounding box
        let mesh = IndexedMesh {
            positions: vec![0.25, 0.25, 0.25, 0.75, 0.25, 0.25, 0.25, 0.75, 0.25],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let children = split_mesh_clipping(&mesh, &bounds);
        let non_empty_count = children.iter().filter(|m| !m.is_empty()).count();
        assert!(non_empty_count >= 2, "boundary triangle should appear in ≥2 octants, got {non_empty_count}");

        // Total triangles should be >= 1 (clipping produces more)
        let total_tris: usize = children.iter().map(|m| m.triangle_count()).sum();
        assert!(total_tris >= 1);
    }

    #[test]
    fn split_mesh_preserves_attributes() {
        let mesh = IndexedMesh {
            positions: vec![0.1, 0.1, 0.1, 0.2, 0.1, 0.1, 0.1, 0.2, 0.1],
            normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            colors: vec![1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0],
            indices: vec![0, 1, 2],
            material_index: Some(2),
        };
        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let children = split_mesh_clipping(&mesh, &bounds);
        for child in &children {
            if child.is_empty() {
                continue;
            }
            assert!(child.has_normals(), "normals should be preserved");
            assert!(child.has_uvs(), "UVs should be preserved");
            assert!(child.has_colors(), "colors should be preserved");
            assert_eq!(child.material_index, Some(2));
            assert_eq!(child.normals.len(), child.positions.len());
            assert_eq!(child.uvs.len(), child.vertex_count() * 2);
            assert_eq!(child.colors.len(), child.vertex_count() * 4);
        }
    }

    #[test]
    fn split_mesh_total_area_conserved() {
        // Triangle straddling center
        let mesh = IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let original_area = triangle_area_f32(&mesh.positions, 0, 1, 2);

        let children = split_mesh_clipping(&mesh, &bounds);
        let mut total_area = 0.0_f64;
        for child in &children {
            for tri in child.indices.chunks_exact(3) {
                total_area += triangle_area_f32(&child.positions, tri[0] as usize, tri[1] as usize, tri[2] as usize);
            }
        }

        let rel_error = (total_area - original_area).abs() / original_area;
        assert!(rel_error < 1e-4, "area should be conserved within ε, got relative error {rel_error}");
    }

    #[test]
    fn split_mesh_boundary_vertex_shared() {
        // Triangle straddling the X midpoint
        let mesh = IndexedMesh {
            positions: vec![0.25, 0.25, 0.25, 0.75, 0.25, 0.25, 0.5, 0.4, 0.25],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let bounds = BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 1.0],
        };

        let children = split_mesh_clipping(&mesh, &bounds);

        // Collect all vertex positions from non-empty octants
        let mut boundary_positions = Vec::new();
        for child in &children {
            if child.is_empty() {
                continue;
            }
            for vi in 0..child.vertex_count() {
                let x = child.positions[vi * 3];
                // Check for vertices at the boundary (x ≈ 0.5)
                if (x - 0.5).abs() < 1e-4 {
                    boundary_positions.push([
                        child.positions[vi * 3],
                        child.positions[vi * 3 + 1],
                        child.positions[vi * 3 + 2],
                    ]);
                }
            }
        }

        // Adjacent octants should share vertex positions at the boundary
        assert!(boundary_positions.len() >= 2, "boundary vertices should appear in multiple octants");
    }

    /// Helper: compute area of a triangle from a flat f32 positions array.
    fn triangle_area_f32(positions: &[f32], i0: usize, i1: usize, i2: usize) -> f64 {
        let ax = positions[i0 * 3] as f64;
        let ay = positions[i0 * 3 + 1] as f64;
        let az = positions[i0 * 3 + 2] as f64;
        let bx = positions[i1 * 3] as f64;
        let by = positions[i1 * 3 + 1] as f64;
        let bz = positions[i1 * 3 + 2] as f64;
        let cx = positions[i2 * 3] as f64;
        let cy = positions[i2 * 3 + 1] as f64;
        let cz = positions[i2 * 3 + 2] as f64;

        let ux = bx - ax;
        let uy = by - ay;
        let uz = bz - az;
        let vx = cx - ax;
        let vy = cy - ay;
        let vz = cz - az;

        let cross_x = uy * vz - uz * vy;
        let cross_y = uz * vx - ux * vz;
        let cross_z = ux * vy - uy * vx;

        0.5 * (cross_x * cross_x + cross_y * cross_y + cross_z * cross_z).sqrt()
    }
}
