use std::path::Path;

use tracing::{debug, warn};

use crate::config::PipelineConfig;
use crate::error::{PhotoTilerError, Result};
use crate::types::{IndexedMesh, MaterialLibrary, PBRMaterial, TextureData};

/// Load an OBJ file (+ associated MTL and textures) into our internal types.
pub fn load_obj(path: &Path, config: &PipelineConfig) -> Result<(Vec<IndexedMesh>, MaterialLibrary)> {
    let (models, materials_result) = tobj::load_obj(path, &tobj::GPU_LOAD_OPTIONS)
        .map_err(|e| PhotoTilerError::Input(format!("Failed to load OBJ: {e}")))?;

    debug!(model_count = models.len(), "Loaded OBJ models");

    let obj_dir = path.parent().unwrap_or_else(|| Path::new("."));

    let tobj_materials = match materials_result {
        Ok(mats) => mats,
        Err(e) => {
            warn!("Failed to load MTL: {e}");
            Vec::new()
        }
    };

    let material_lib = convert_materials(&tobj_materials, obj_dir, config)?;

    let meshes: Vec<IndexedMesh> = models
        .into_iter()
        .map(|model| convert_mesh(model.mesh))
        .collect();

    Ok((meshes, material_lib))
}

/// Convert a `tobj::Mesh` into our `IndexedMesh`.
fn convert_mesh(mesh: tobj::Mesh) -> IndexedMesh {
    let positions = mesh.positions;
    let normals = mesh.normals;

    // UV V-flip: OBJ uses bottom-left origin, glTF uses top-left
    let uvs: Vec<f32> = mesh
        .texcoords
        .chunks_exact(2)
        .flat_map(|uv| [uv[0], 1.0 - uv[1]])
        .collect();

    // Vertex colors: expand RGB (3 components) to RGBA (4 components, alpha=1.0)
    let colors: Vec<f32> = mesh
        .vertex_color
        .chunks_exact(3)
        .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 1.0])
        .collect();

    let material_index = mesh.material_id;

    IndexedMesh {
        positions,
        normals,
        uvs,
        colors,
        indices: mesh.indices,
        material_index,
    }
}

/// Convert tobj materials into our `MaterialLibrary`.
fn convert_materials(
    tobj_mats: &[tobj::Material],
    obj_dir: &Path,
    config: &PipelineConfig,
) -> Result<MaterialLibrary> {
    let mut lib = MaterialLibrary::default();

    for mat in tobj_mats {
        let mut pbr = PBRMaterial {
            name: mat.name.clone(),
            metallic: 0.0,
            roughness: 1.0,
            ..Default::default()
        };

        // Kd -> base_color
        if let Some(diffuse) = mat.diffuse {
            pbr.base_color = [
                diffuse[0],
                diffuse[1],
                diffuse[2],
                mat.dissolve.unwrap_or(1.0),
            ];
        }

        // Load diffuse texture (map_Kd)
        if config.texture.enabled {
            if let Some(ref tex_name) = mat.diffuse_texture {
                let tex_path = obj_dir.join(tex_name);
                match load_texture(&tex_path) {
                    Ok(tex) => {
                        let tex_idx = lib.textures.len();
                        lib.textures.push(tex);
                        pbr.base_color_texture = Some(tex_idx);
                    }
                    Err(e) => {
                        warn!(texture = %tex_name, "Failed to load texture: {e}");
                    }
                }
            }
        }

        lib.materials.push(pbr);
    }

    Ok(lib)
}

/// Load a texture file: read raw bytes and decode for width/height.
fn load_texture(path: &Path) -> Result<TextureData> {
    let data = std::fs::read(path).map_err(|e| {
        PhotoTilerError::Input(format!("Failed to read texture {}: {e}", path.display()))
    })?;

    let img = image::load_from_memory(&data).map_err(|e| {
        PhotoTilerError::Input(format!(
            "Failed to decode texture {}: {e}",
            path.display()
        ))
    })?;

    let mime_type = match path.extension().and_then(|e| e.to_str()) {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    };

    debug!(
        path = %path.display(),
        width = img.width(),
        height = img.height(),
        "Loaded texture"
    );

    Ok(TextureData {
        data,
        mime_type: mime_type.to_string(),
        width: img.width(),
        height: img.height(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_mesh_basic() {
        let mesh = tobj::Mesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            texcoords: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            vertex_color: vec![],
            face_arities: vec![],
            texcoord_indices: vec![],
            normal_indices: vec![],
            material_id: Some(0),
        };

        let indexed = convert_mesh(mesh);
        assert_eq!(indexed.vertex_count(), 3);
        assert_eq!(indexed.triangle_count(), 1);
        assert!(indexed.has_normals());
        assert!(indexed.has_uvs());
        assert!(!indexed.has_colors());
        assert_eq!(indexed.material_index, Some(0));
    }

    #[test]
    fn convert_mesh_uv_vflip() {
        let mesh = tobj::Mesh {
            positions: vec![0.0; 9],
            normals: vec![],
            texcoords: vec![0.0, 0.0, 1.0, 0.3, 0.5, 1.0],
            indices: vec![0, 1, 2],
            vertex_color: vec![],
            face_arities: vec![],
            texcoord_indices: vec![],
            normal_indices: vec![],
            material_id: None,
        };

        let indexed = convert_mesh(mesh);
        // V-flip: v = 1.0 - v
        // Original UVs: (0.0,0.0), (1.0,0.3), (0.5,1.0)
        // Flipped UVs:  (0.0,1.0), (1.0,0.7), (0.5,0.0)
        assert!((indexed.uvs[1] - 1.0).abs() < f32::EPSILON);
        assert!((indexed.uvs[3] - 0.7).abs() < 1e-6);
        assert!((indexed.uvs[5] - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn convert_mesh_vertex_color_rgb_to_rgba() {
        let mesh = tobj::Mesh {
            positions: vec![0.0; 9],
            normals: vec![],
            texcoords: vec![],
            indices: vec![0, 1, 2],
            vertex_color: vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            face_arities: vec![],
            texcoord_indices: vec![],
            normal_indices: vec![],
            material_id: None,
        };

        let indexed = convert_mesh(mesh);
        assert!(indexed.has_colors());
        // 3 vertices * 4 components = 12 floats
        assert_eq!(indexed.colors.len(), 12);
        // First vertex: R=1, G=0, B=0, A=1
        assert!((indexed.colors[0] - 1.0).abs() < f32::EPSILON);
        assert!((indexed.colors[1] - 0.0).abs() < f32::EPSILON);
        assert!((indexed.colors[2] - 0.0).abs() < f32::EPSILON);
        assert!((indexed.colors[3] - 1.0).abs() < f32::EPSILON);
        // Third vertex: R=0, G=0, B=1, A=1
        assert!((indexed.colors[8] - 0.0).abs() < f32::EPSILON);
        assert!((indexed.colors[9] - 0.0).abs() < f32::EPSILON);
        assert!((indexed.colors[10] - 1.0).abs() < f32::EPSILON);
        assert!((indexed.colors[11] - 1.0).abs() < f32::EPSILON);
    }
}
