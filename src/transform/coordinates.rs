use crate::config::Units;
use crate::types::{BoundingBox, IndexedMesh};

/// Return the multiplier to convert the given units to metres.
pub fn unit_scale_factor(units: Units) -> f64 {
    match units {
        Units::Millimeters => 0.001,
        Units::Centimeters => 0.01,
        Units::Meters => 1.0,
        Units::Feet => 0.3048,
        Units::Inches => 0.0254,
    }
}

/// Scale all vertex positions in-place (f64 math, write back f32).
pub fn apply_unit_scaling(meshes: &mut [IndexedMesh], factor: f64) {
    for mesh in meshes.iter_mut() {
        for pos in mesh.positions.iter_mut() {
            *pos = ((*pos as f64) * factor) as f32;
        }
    }
}

/// Convert from right-handed Y-up (OBJ/glTF) to right-handed Z-up (3D Tiles).
///
/// Transform: `(x, y, z)` → `(x, z, -y)`
pub fn swap_y_up_to_z_up(meshes: &mut [IndexedMesh]) {
    for mesh in meshes.iter_mut() {
        for tri in mesh.positions.chunks_exact_mut(3) {
            let y = tri[1];
            let z = tri[2];
            tri[1] = z;
            tri[2] = -y;
        }
        // Normals follow the same rotation
        for tri in mesh.normals.chunks_exact_mut(3) {
            let y = tri[1];
            let z = tri[2];
            tri[1] = z;
            tri[2] = -y;
        }
    }
}

/// Rotate all vertex positions about the Z axis by the given angle in degrees.
pub fn apply_true_north_rotation(meshes: &mut [IndexedMesh], degrees: f64) {
    let radians = degrees.to_radians();
    let cos_a = radians.cos();
    let sin_a = radians.sin();

    for mesh in meshes.iter_mut() {
        for tri in mesh.positions.chunks_exact_mut(3) {
            let x = tri[0] as f64;
            let y = tri[1] as f64;
            tri[0] = (x * cos_a - y * sin_a) as f32;
            tri[1] = (x * sin_a + y * cos_a) as f32;
        }
        for tri in mesh.normals.chunks_exact_mut(3) {
            let x = tri[0] as f64;
            let y = tri[1] as f64;
            tri[0] = (x * cos_a - y * sin_a) as f32;
            tri[1] = (x * sin_a + y * cos_a) as f32;
        }
    }
}

/// Compute the centroid of all vertices, subtract it from every position,
/// and return the centroid offset `[cx, cy, cz]`.
pub fn center_meshes(meshes: &mut [IndexedMesh]) -> [f64; 3] {
    // Accumulate in f64
    let mut sum = [0.0_f64; 3];
    let mut count: usize = 0;

    for mesh in meshes.iter() {
        for tri in mesh.positions.chunks_exact(3) {
            sum[0] += tri[0] as f64;
            sum[1] += tri[1] as f64;
            sum[2] += tri[2] as f64;
            count += 1;
        }
    }

    if count == 0 {
        return [0.0; 3];
    }

    let centroid = [
        sum[0] / count as f64,
        sum[1] / count as f64,
        sum[2] / count as f64,
    ];

    for mesh in meshes.iter_mut() {
        for tri in mesh.positions.chunks_exact_mut(3) {
            tri[0] = ((tri[0] as f64) - centroid[0]) as f32;
            tri[1] = ((tri[1] as f64) - centroid[1]) as f32;
            tri[2] = ((tri[2] as f64) - centroid[2]) as f32;
        }
    }

    centroid
}

/// Scan all vertex positions and return the axis-aligned bounding box.
pub fn compute_bounding_box(meshes: &[IndexedMesh]) -> BoundingBox {
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];

    for mesh in meshes {
        for tri in mesh.positions.chunks_exact(3) {
            let x = tri[0] as f64;
            let y = tri[1] as f64;
            let z = tri[2] as f64;
            if x < min[0] {
                min[0] = x;
            }
            if y < min[1] {
                min[1] = y;
            }
            if z < min[2] {
                min[2] = z;
            }
            if x > max[0] {
                max[0] = x;
            }
            if y > max[1] {
                max[1] = y;
            }
            if z > max[2] {
                max[2] = z;
            }
        }
    }

    // If no vertices, return a zero-size box at origin
    if min[0] == f64::INFINITY {
        return BoundingBox {
            min: [0.0; 3],
            max: [0.0; 3],
        };
    }

    BoundingBox { min, max }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_triangle(x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32, x2: f32, y2: f32, z2: f32) -> IndexedMesh {
        IndexedMesh {
            positions: vec![x0, y0, z0, x1, y1, z1, x2, y2, z2],
            normals: vec![0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0],
            uvs: vec![],
            colors: vec![],
            indices: vec![0, 1, 2],
            material_index: None,
        }
    }

    #[test]
    fn unit_scale_factors() {
        assert!((unit_scale_factor(Units::Millimeters) - 0.001).abs() < f64::EPSILON);
        assert!((unit_scale_factor(Units::Centimeters) - 0.01).abs() < f64::EPSILON);
        assert!((unit_scale_factor(Units::Meters) - 1.0).abs() < f64::EPSILON);
        assert!((unit_scale_factor(Units::Feet) - 0.3048).abs() < f64::EPSILON);
        assert!((unit_scale_factor(Units::Inches) - 0.0254).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_unit_scaling_doubles_positions() {
        let mut meshes = vec![IndexedMesh {
            positions: vec![1.0, 2.0, 3.0],
            ..Default::default()
        }];
        apply_unit_scaling(&mut meshes, 2.0);
        assert!((meshes[0].positions[0] - 2.0).abs() < 1e-5);
        assert!((meshes[0].positions[1] - 4.0).abs() < 1e-5);
        assert!((meshes[0].positions[2] - 6.0).abs() < 1e-5);
    }

    #[test]
    fn swap_y_up_to_z_up_known_triangle() {
        // Y-up: vertex at (1, 2, 3) → Z-up: (1, 3, -2)
        let mut meshes = vec![make_triangle(1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)];
        swap_y_up_to_z_up(&mut meshes);
        let p = &meshes[0].positions;
        assert!((p[0] - 1.0).abs() < 1e-6);  // x unchanged
        assert!((p[1] - 3.0).abs() < 1e-6);  // new y = old z
        assert!((p[2] - (-2.0)).abs() < 1e-6); // new z = -old y

        // Normal (0,1,0) → (0,0,-1)
        let n = &meshes[0].normals;
        assert!((n[0] - 0.0).abs() < 1e-6);
        assert!((n[1] - 0.0).abs() < 1e-6);
        assert!((n[2] - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn true_north_rotation_90_degrees() {
        // Point (1, 0, 0) rotated 90° about Z → (0, 1, 0)
        let mut meshes = vec![IndexedMesh {
            positions: vec![1.0, 0.0, 5.0],
            normals: vec![],
            ..Default::default()
        }];
        apply_true_north_rotation(&mut meshes, 90.0);
        let p = &meshes[0].positions;
        assert!((p[0] - 0.0).abs() < 1e-5);
        assert!((p[1] - 1.0).abs() < 1e-5);
        assert!((p[2] - 5.0).abs() < 1e-5); // z unchanged
    }

    #[test]
    fn centering_returns_correct_offset() {
        let mut meshes = vec![IndexedMesh {
            positions: vec![
                10.0, 20.0, 30.0,
                20.0, 40.0, 60.0,
            ],
            ..Default::default()
        }];
        let offset = center_meshes(&mut meshes);
        // Centroid = (15, 30, 45)
        assert!((offset[0] - 15.0).abs() < 1e-6);
        assert!((offset[1] - 30.0).abs() < 1e-6);
        assert!((offset[2] - 45.0).abs() < 1e-6);

        // After centering, vertices should be symmetric around origin
        let p = &meshes[0].positions;
        assert!((p[0] - (-5.0)).abs() < 1e-3);
        assert!((p[1] - (-10.0)).abs() < 1e-3);
        assert!((p[2] - (-15.0)).abs() < 1e-3);
        assert!((p[3] - 5.0).abs() < 1e-3);
        assert!((p[4] - 10.0).abs() < 1e-3);
        assert!((p[5] - 15.0).abs() < 1e-3);
    }

    #[test]
    fn centering_empty_meshes() {
        let mut meshes: Vec<IndexedMesh> = vec![];
        let offset = center_meshes(&mut meshes);
        assert_eq!(offset, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn bounding_box_computation() {
        let meshes = vec![
            IndexedMesh {
                positions: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
                ..Default::default()
            },
            IndexedMesh {
                positions: vec![-1.0, -2.0, -3.0],
                ..Default::default()
            },
        ];
        let bb = compute_bounding_box(&meshes);
        assert!((bb.min[0] - (-1.0)).abs() < 1e-6);
        assert!((bb.min[1] - (-2.0)).abs() < 1e-6);
        assert!((bb.min[2] - (-3.0)).abs() < 1e-6);
        assert!((bb.max[0] - 4.0).abs() < 1e-6);
        assert!((bb.max[1] - 5.0).abs() < 1e-6);
        assert!((bb.max[2] - 6.0).abs() < 1e-6);
    }

    #[test]
    fn bounding_box_empty() {
        let meshes: Vec<IndexedMesh> = vec![];
        let bb = compute_bounding_box(&meshes);
        assert_eq!(bb.min, [0.0; 3]);
        assert_eq!(bb.max, [0.0; 3]);
    }
}
