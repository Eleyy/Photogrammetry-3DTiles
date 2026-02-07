use std::path::Path;

use tracing::debug;

use crate::error::{PhotoTilerError, Result};
use crate::types::{IndexedMesh, MaterialLibrary, PBRMaterial, TextureData};

/// Load a glTF or GLB file into our internal types.
pub fn load_gltf(path: &Path) -> Result<(Vec<IndexedMesh>, MaterialLibrary)> {
    let (document, buffers, images) = gltf::import(path)
        .map_err(|e| PhotoTilerError::Input(format!("Failed to load glTF: {e}")))?;

    debug!(
        meshes = document.meshes().len(),
        materials = document.materials().len(),
        "Loaded glTF document"
    );

    let mut meshes = Vec::new();

    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            match extract_primitive(&primitive, &buffers) {
                Ok(mut indexed) => {
                    indexed.material_index = primitive.material().index();
                    meshes.push(indexed);
                }
                Err(e) => {
                    tracing::warn!(mesh = ?mesh.name(), "Skipping primitive: {e}");
                }
            }
        }
    }

    let mut lib = MaterialLibrary::default();

    // Convert materials
    for material in document.materials() {
        lib.materials.push(convert_gltf_material(&material));
    }

    // Convert images/textures
    for image_data in &images {
        lib.textures.push(convert_gltf_image(image_data));
    }

    Ok((meshes, lib))
}

/// Extract geometry from a single glTF primitive.
fn extract_primitive(
    primitive: &gltf::Primitive<'_>,
    buffers: &[gltf::buffer::Data],
) -> Result<IndexedMesh> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    // Positions (required)
    let positions: Vec<f32> = reader
        .read_positions()
        .ok_or_else(|| PhotoTilerError::Input("Primitive missing positions".into()))?
        .flatten()
        .collect();

    // Normals (optional)
    let normals: Vec<f32> = reader
        .read_normals()
        .map(|iter| iter.flatten().collect())
        .unwrap_or_default();

    // UVs (optional, no V-flip needed for glTF)
    let uvs: Vec<f32> = reader
        .read_tex_coords(0)
        .map(|iter| iter.into_f32().flatten().collect())
        .unwrap_or_default();

    // Vertex colors (optional)
    let colors: Vec<f32> = reader
        .read_colors(0)
        .map(|iter| iter.into_rgba_f32().flatten().collect())
        .unwrap_or_default();

    // Indices (required for indexed geometry)
    let indices: Vec<u32> = reader
        .read_indices()
        .ok_or_else(|| PhotoTilerError::Input("Primitive missing indices".into()))?
        .into_u32()
        .collect();

    Ok(IndexedMesh {
        positions,
        normals,
        uvs,
        colors,
        indices,
        material_index: None, // Set by caller
    })
}

/// Convert a glTF material to our PBR material type.
fn convert_gltf_material(material: &gltf::Material<'_>) -> PBRMaterial {
    let pbr = material.pbr_metallic_roughness();
    let color = pbr.base_color_factor();

    let base_color_texture = pbr
        .base_color_texture()
        .map(|info| info.texture().source().index());

    PBRMaterial {
        name: material.name().unwrap_or("").to_string(),
        base_color: color,
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        base_color_texture,
    }
}

/// Convert glTF image data to our TextureData type.
fn convert_gltf_image(image_data: &gltf::image::Data) -> TextureData {
    let mime_type = match image_data.format {
        gltf::image::Format::R8 | gltf::image::Format::R8G8 => "image/png",
        gltf::image::Format::R8G8B8 | gltf::image::Format::R8G8B8A8 => "image/png",
        gltf::image::Format::R16 | gltf::image::Format::R16G16 => "image/png",
        gltf::image::Format::R16G16B16 | gltf::image::Format::R16G16B16A16 => "image/png",
        gltf::image::Format::R32G32B32FLOAT | gltf::image::Format::R32G32B32A32FLOAT => {
            "image/png"
        }
    };

    TextureData {
        data: image_data.pixels.clone(),
        mime_type: mime_type.to_string(),
        width: image_data.width,
        height: image_data.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gltf_material_conversion_defaults() {
        // We can't easily construct a gltf::Material, so we test
        // the PBRMaterial defaults that convert_gltf_material would produce
        let mat = PBRMaterial::default();
        assert_eq!(mat.base_color, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(mat.metallic, 0.0);
        assert_eq!(mat.roughness, 1.0);
        assert_eq!(mat.base_color_texture, None);
    }

    #[test]
    fn gltf_image_conversion() {
        let image_data = gltf::image::Data {
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 255],
            format: gltf::image::Format::R8G8B8A8,
            width: 2,
            height: 1,
        };

        let tex = convert_gltf_image(&image_data);
        assert_eq!(tex.width, 2);
        assert_eq!(tex.height, 1);
        assert_eq!(tex.mime_type, "image/png");
        assert_eq!(tex.data.len(), 8);
    }
}
