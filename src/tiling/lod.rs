use tracing::info;

use crate::types::{BoundingBox, IndexedMesh};

use super::simplifier::simplify_mesh;

/// A single level of detail.
#[derive(Debug, Clone)]
pub struct LodLevel {
    pub level: u32,
    pub mesh: IndexedMesh,
    pub geometric_error: f64,
}

/// A chain of LOD levels ordered finest (LOD 0 = original) to coarsest.
///
/// In 3D Tiles, the coarsest level becomes the root tile (highest
/// `geometricError`) and the finest becomes a leaf (error = 0).
#[derive(Debug, Clone)]
pub struct LodChain {
    pub levels: Vec<LodLevel>,
    pub bounds: BoundingBox,
}

/// Minimum triangle count before we stop generating coarser LODs.
const MIN_TRIANGLE_COUNT: usize = 1000;

/// Generate a chain of LOD levels by repeatedly simplifying the mesh.
///
/// LOD 0 = original mesh (geometric_error = 0, finest detail).
/// LOD N = simplified at ratio `0.25^N` of the original index count.
///
/// `geometric_error` is derived from meshopt's achieved simplification
/// error (relative) scaled by the bounding-box diagonal to produce a
/// value in the same units as the mesh (meters after transform).
/// This matches the 3D Tiles spec where `geometricError` is the metric
/// error introduced by rendering this LOD instead of a finer one.
///
/// Stops when `max_levels` is reached, triangle count drops below 1000,
/// or simplification can't reduce further.
pub fn generate_lod_chain(
    mesh: IndexedMesh,
    bounds: &BoundingBox,
    max_levels: u32,
) -> LodChain {
    let diagonal = bounds.diagonal();
    let mut levels = Vec::new();

    // LOD 0: original mesh (finest detail → zero geometric error)
    // Takes ownership -- no clone needed.
    levels.push(LodLevel {
        level: 0,
        mesh,
        geometric_error: 0.0,
    });

    if levels[0].mesh.is_empty() || max_levels <= 1 {
        return LodChain {
            levels,
            bounds: *bounds,
        };
    }

    let mut prev_triangle_count = levels[0].mesh.triangle_count();
    let mut cumulative_error = 0.0_f64;

    for n in 1..max_levels {
        // Cascade: simplify from previous level (not from LOD 0)
        let ratio = 0.25_f32;

        let prev_level = &levels[n as usize - 1];
        info!(
            level = n,
            ratio,
            source_triangles = prev_level.mesh.triangle_count(),
            target_triangles = (prev_level.mesh.indices.len() as f64 * ratio as f64 / 3.0) as usize,
            "Generating LOD level (cascaded)"
        );

        let simplified = simplify_mesh(&prev_level.mesh, ratio, true);

        // Stop if simplification couldn't reduce meaningfully (< 5% reduction)
        let new_triangle_count = simplified.mesh.triangle_count();
        if new_triangle_count == 0 {
            break;
        }
        if new_triangle_count >= prev_triangle_count * 95 / 100 {
            info!(
                level = n,
                triangles = new_triangle_count,
                "Simplification stalled, stopping LOD chain"
            );
            break;
        }

        // Compound error: each level accumulates error from all previous
        // simplification steps.
        let measured_error = simplified.achieved_error as f64 * diagonal;
        cumulative_error += measured_error;
        // Heuristic minimum based on overall reduction from the original
        let overall_ratio = 0.25_f64.powi(n as i32);
        let min_heuristic_error = diagonal * (1.0 - overall_ratio) * 0.5;
        let geometric_error = cumulative_error.max(min_heuristic_error);

        levels.push(LodLevel {
            level: n,
            mesh: simplified.mesh,
            geometric_error,
        });

        // Stop if we've reached the minimum triangle count
        if new_triangle_count < MIN_TRIANGLE_COUNT {
            info!(
                level = n,
                triangles = new_triangle_count,
                "Below minimum triangle threshold, stopping LOD chain"
            );
            break;
        }

        prev_triangle_count = new_triangle_count;
    }

    LodChain {
        levels,
        bounds: *bounds,
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

        for y in 0..verts_per_side {
            for x in 0..verts_per_side {
                let fx = x as f32 / n as f32;
                let fy = y as f32 / n as f32;
                positions.extend_from_slice(&[fx, fy, 0.0]);
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
            indices,
            ..Default::default()
        }
    }

    fn unit_bounds() -> BoundingBox {
        BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [1.0, 1.0, 0.0],
        }
    }

    #[test]
    fn lod_chain_levels_decrease_in_triangles() {
        let mesh = make_grid(100); // 20000 triangles
        let bounds = unit_bounds();
        let chain = generate_lod_chain(mesh, &bounds, 4);

        assert!(chain.levels.len() >= 2, "Should produce at least 2 LOD levels");

        for i in 1..chain.levels.len() {
            assert!(
                chain.levels[i].mesh.triangle_count()
                    < chain.levels[i - 1].mesh.triangle_count(),
                "LOD {} ({} tris) should have fewer triangles than LOD {} ({} tris)",
                i,
                chain.levels[i].mesh.triangle_count(),
                i - 1,
                chain.levels[i - 1].mesh.triangle_count(),
            );
        }
    }

    #[test]
    fn lod_chain_geometric_error_increases() {
        let mesh = make_grid(100);
        let bounds = unit_bounds();
        let chain = generate_lod_chain(mesh, &bounds, 4);

        for i in 1..chain.levels.len() {
            assert!(
                chain.levels[i].geometric_error > chain.levels[i - 1].geometric_error,
                "LOD {} error ({}) should be greater than LOD {} error ({})",
                i,
                chain.levels[i].geometric_error,
                i - 1,
                chain.levels[i - 1].geometric_error,
            );
        }
    }

    #[test]
    fn lod_chain_lod0_is_original() {
        let mesh = make_grid(20);
        let tris = mesh.triangle_count();
        let bounds = unit_bounds();
        let chain = generate_lod_chain(mesh, &bounds, 4);

        assert_eq!(chain.levels[0].level, 0);
        assert_eq!(chain.levels[0].mesh.triangle_count(), tris);
    }

    #[test]
    fn lod_chain_empty_mesh() {
        let mesh = IndexedMesh::default();
        let bounds = BoundingBox {
            min: [0.0; 3],
            max: [0.0; 3],
        };
        let chain = generate_lod_chain(mesh, &bounds, 4);
        assert_eq!(chain.levels.len(), 1); // Only LOD 0
    }

    #[test]
    fn lod_chain_respects_max_levels() {
        let mesh = make_grid(100);
        let bounds = unit_bounds();
        let chain = generate_lod_chain(mesh, &bounds, 2);
        assert!(chain.levels.len() <= 2);
    }

    #[test]
    fn lod_chain_bounds_preserved() {
        let bounds = unit_bounds();
        let mesh = make_grid(20);
        let chain = generate_lod_chain(mesh, &bounds, 4);
        assert_eq!(chain.bounds, bounds);
    }
}
