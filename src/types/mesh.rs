/// The fundamental geometry container.
///
/// All buffers are contiguous `Vec<f32>` / `Vec<u32>` for zero-copy interop
/// with meshoptimizer and glTF writers.
#[derive(Debug, Clone, Default)]
pub struct IndexedMesh {
    /// Interleaved positions: [x, y, z, x, y, z, ...]
    pub positions: Vec<f32>,
    /// Interleaved normals: [nx, ny, nz, ...] or empty
    pub normals: Vec<f32>,
    /// Interleaved UVs: [u, v, u, v, ...] or empty
    pub uvs: Vec<f32>,
    /// Interleaved vertex colors: [r, g, b, a, ...] or empty
    pub colors: Vec<f32>,
    /// Triangle indices into the vertex buffers
    pub indices: Vec<u32>,
    /// Index into the associated `MaterialLibrary`
    pub material_index: Option<usize>,
}

impl IndexedMesh {
    /// Number of vertices (positions / 3).
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Number of triangles (indices / 3).
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Whether normals are present.
    pub fn has_normals(&self) -> bool {
        !self.normals.is_empty()
    }

    /// Whether UV coordinates are present.
    pub fn has_uvs(&self) -> bool {
        !self.uvs.is_empty()
    }

    /// Whether vertex colors are present.
    pub fn has_colors(&self) -> bool {
        !self.colors.is_empty()
    }

    /// Whether the mesh contains no geometry.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_mesh() {
        let mesh = IndexedMesh::default();
        assert!(mesh.is_empty());
        assert_eq!(mesh.vertex_count(), 0);
        assert_eq!(mesh.triangle_count(), 0);
        assert!(!mesh.has_normals());
        assert!(!mesh.has_uvs());
        assert!(!mesh.has_colors());
        assert_eq!(mesh.material_index, None);
    }

    #[test]
    fn single_triangle() {
        let mesh = IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            colors: vec![],
            indices: vec![0, 1, 2],
            material_index: Some(0),
        };

        assert!(!mesh.is_empty());
        assert_eq!(mesh.vertex_count(), 3);
        assert_eq!(mesh.triangle_count(), 1);
        assert!(mesh.has_normals());
        assert!(mesh.has_uvs());
        assert!(!mesh.has_colors());
        assert_eq!(mesh.material_index, Some(0));
    }

    #[test]
    fn quad_two_triangles() {
        let mesh = IndexedMesh {
            positions: vec![
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0,
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..Default::default()
        };

        assert_eq!(mesh.vertex_count(), 4);
        assert_eq!(mesh.triangle_count(), 2);
    }
}
