use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use ply_rs::parser::Parser;
use ply_rs::ply::{DefaultElement, Property};
use tracing::debug;

use crate::error::{PhotoTilerError, Result};
use crate::types::IndexedMesh;

/// Load a PLY file into an `IndexedMesh`.
pub fn load_ply(path: &Path) -> Result<IndexedMesh> {
    let file = File::open(path)
        .map_err(|e| PhotoTilerError::Input(format!("Failed to open PLY: {e}")))?;
    let mut reader = BufReader::new(file);

    let parser = Parser::<DefaultElement>::new();
    let ply = parser.read_ply(&mut reader).map_err(|e| {
        PhotoTilerError::Input(format!("Failed to parse PLY: {e}"))
    })?;

    let vertices = ply.payload.get("vertex").ok_or_else(|| {
        PhotoTilerError::Input("PLY file missing 'vertex' element".into())
    })?;

    debug!(vertex_count = vertices.len(), "Parsing PLY vertices");

    let mut positions = Vec::with_capacity(vertices.len() * 3);
    let mut normals = Vec::new();
    let mut colors = Vec::new();

    let has_normals = vertices
        .first()
        .map(|v| v.contains_key("nx"))
        .unwrap_or(false);
    let has_colors = vertices.first().map(|v| {
        v.contains_key("red") || v.contains_key("r")
    }).unwrap_or(false);

    if has_normals {
        normals.reserve(vertices.len() * 3);
    }
    if has_colors {
        colors.reserve(vertices.len() * 4);
    }

    for vertex in vertices {
        positions.push(get_float_property(vertex, "x")?);
        positions.push(get_float_property(vertex, "y")?);
        positions.push(get_float_property(vertex, "z")?);

        if has_normals {
            normals.push(get_float_property(vertex, "nx")?);
            normals.push(get_float_property(vertex, "ny")?);
            normals.push(get_float_property(vertex, "nz")?);
        }

        if has_colors {
            let r = get_color_property(vertex)?;
            colors.push(r.0);
            colors.push(r.1);
            colors.push(r.2);
            colors.push(1.0); // alpha
        }
    }

    // Parse faces
    let mut indices = Vec::new();
    if let Some(faces) = ply.payload.get("face") {
        debug!(face_count = faces.len(), "Parsing PLY faces");
        for face in faces {
            let face_indices = get_index_list(face)?;
            // Fan-triangulate polygons with >3 vertices
            if face_indices.len() >= 3 {
                for i in 1..face_indices.len() - 1 {
                    indices.push(face_indices[0]);
                    indices.push(face_indices[i]);
                    indices.push(face_indices[i + 1]);
                }
            }
        }
    }

    Ok(IndexedMesh {
        positions,
        normals,
        uvs: Vec::new(), // PLY typically lacks UVs
        colors,
        indices,
        material_index: None,
    })
}

/// Extract a float property, handling Float/Double/Int/Short types.
fn get_float_property(element: &DefaultElement, key: &str) -> Result<f32> {
    let prop = element.get(key).ok_or_else(|| {
        PhotoTilerError::Input(format!("PLY vertex missing property '{key}'"))
    })?;

    match prop {
        Property::Float(v) => Ok(*v),
        Property::Double(v) => Ok(*v as f32),
        Property::Int(v) => Ok(*v as f32),
        Property::Short(v) => Ok(*v as f32),
        Property::UInt(v) => Ok(*v as f32),
        Property::UShort(v) => Ok(*v as f32),
        Property::Char(v) => Ok(*v as f32),
        Property::UChar(v) => Ok(*v as f32),
        _ => Err(PhotoTilerError::Input(format!(
            "PLY property '{key}' has unsupported type"
        ))),
    }
}

/// Extract RGB color from a vertex, normalizing UChar 0-255 to f32 0.0-1.0.
fn get_color_property(element: &DefaultElement) -> Result<(f32, f32, f32)> {
    // Try "red"/"green"/"blue" first, then "r"/"g"/"b"
    let r_key = if element.contains_key("red") { "red" } else { "r" };
    let g_key = if element.contains_key("green") { "green" } else { "g" };
    let b_key = if element.contains_key("blue") { "blue" } else { "b" };

    let r = normalize_color_value(element, r_key)?;
    let g = normalize_color_value(element, g_key)?;
    let b = normalize_color_value(element, b_key)?;

    Ok((r, g, b))
}

/// Normalize a single color channel: UChar 0-255 -> 0.0-1.0, Float stays as-is.
fn normalize_color_value(element: &DefaultElement, key: &str) -> Result<f32> {
    let prop = element.get(key).ok_or_else(|| {
        PhotoTilerError::Input(format!("PLY vertex missing color property '{key}'"))
    })?;

    match prop {
        Property::UChar(v) => Ok(*v as f32 / 255.0),
        Property::Float(v) => Ok(*v),
        Property::Double(v) => Ok(*v as f32),
        Property::Short(v) => Ok(*v as f32 / 255.0),
        Property::UShort(v) => Ok(*v as f32 / 255.0),
        Property::Int(v) => Ok(*v as f32 / 255.0),
        Property::UInt(v) => Ok(*v as f32 / 255.0),
        _ => Err(PhotoTilerError::Input(format!(
            "PLY color property '{key}' has unsupported type"
        ))),
    }
}

/// Extract the index list from a face element.
fn get_index_list(face: &DefaultElement) -> Result<Vec<u32>> {
    // Try "vertex_indices" first, then "vertex_index"
    let key = if face.contains_key("vertex_indices") {
        "vertex_indices"
    } else {
        "vertex_index"
    };

    let prop = face.get(key).ok_or_else(|| {
        PhotoTilerError::Input("PLY face missing vertex_indices property".into())
    })?;

    match prop {
        Property::ListInt(v) => Ok(v.iter().map(|&i| i as u32).collect()),
        Property::ListUInt(v) => Ok(v.clone()),
        Property::ListUChar(v) => Ok(v.iter().map(|&i| i as u32).collect()),
        Property::ListShort(v) => Ok(v.iter().map(|&i| i as u32).collect()),
        Property::ListUShort(v) => Ok(v.iter().map(|&i| i as u32).collect()),
        _ => Err(PhotoTilerError::Input(
            "PLY face vertex_indices has unsupported type".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_ascii_ply(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn load_ascii_ply_basic() {
        let ply_content = "\
ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element face 1
property list uchar int vertex_indices
end_header
0.0 0.0 0.0
1.0 0.0 0.0
0.0 1.0 0.0
3 0 1 2
";
        let file = write_ascii_ply(ply_content);
        let mesh = load_ply(file.path()).unwrap();

        assert_eq!(mesh.vertex_count(), 3);
        assert_eq!(mesh.triangle_count(), 1);
        assert!(!mesh.has_normals());
        assert!(!mesh.has_uvs());
        assert!(!mesh.has_colors());
    }

    #[test]
    fn load_ascii_ply_with_colors() {
        let ply_content = "\
ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
property uchar red
property uchar green
property uchar blue
element face 1
property list uchar int vertex_indices
end_header
0.0 0.0 0.0 255 0 0
1.0 0.0 0.0 0 255 0
0.0 1.0 0.0 0 0 255
3 0 1 2
";
        let file = write_ascii_ply(ply_content);
        let mesh = load_ply(file.path()).unwrap();

        assert!(mesh.has_colors());
        assert_eq!(mesh.colors.len(), 12); // 3 verts * 4 (RGBA)
        // First vertex: red = 255 -> 1.0
        assert!((mesh.colors[0] - 1.0).abs() < 1e-3);
        assert!((mesh.colors[1] - 0.0).abs() < 1e-3);
        assert!((mesh.colors[2] - 0.0).abs() < 1e-3);
        assert!((mesh.colors[3] - 1.0).abs() < 1e-3); // alpha
    }

    #[test]
    fn polygon_triangulation() {
        let ply_content = "\
ply
format ascii 1.0
element vertex 4
property float x
property float y
property float z
element face 1
property list uchar int vertex_indices
end_header
0.0 0.0 0.0
1.0 0.0 0.0
1.0 1.0 0.0
0.0 1.0 0.0
4 0 1 2 3
";
        let file = write_ascii_ply(ply_content);
        let mesh = load_ply(file.path()).unwrap();

        // Quad -> 2 triangles
        assert_eq!(mesh.triangle_count(), 2);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn color_normalization_uchar() {
        let mut element = DefaultElement::new();
        element.insert("red".to_string(), Property::UChar(128));
        element.insert("green".to_string(), Property::UChar(0));
        element.insert("blue".to_string(), Property::UChar(255));

        let (r, g, b) = get_color_property(&element).unwrap();
        assert!((r - 128.0 / 255.0).abs() < 1e-3);
        assert!((g - 0.0).abs() < 1e-3);
        assert!((b - 1.0).abs() < 1e-3);
    }
}
