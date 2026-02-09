use std::borrow::Cow;
use std::collections::BTreeMap;

use gltf::binary::Glb;
use gltf_json::accessor::{ComponentType, GenericComponentType, Type as AccessorType};
use gltf_json::buffer::Target;
use gltf_json::mesh::{Mode, Primitive, Semantic};
use gltf_json::validation::{Checked, USize64};
use gltf_json::Index;

use crate::types::{IndexedMesh, MaterialLibrary, TextureData};

/// Serialize an `IndexedMesh` into a binary GLB (glTF 2.0) byte buffer.
///
/// Produces a valid, self-contained GLB with:
/// - 1 buffer (positions + optional normals/UVs/colors + indices + optional texture)
/// - BufferViews and Accessors for each attribute present
/// - 1 Mesh with 1 Primitive (mode = Triangles)
/// - 1 Node → 1 Scene
/// - Material if `material_index` is set and present in `materials`
/// - Texture if `atlas_texture` is provided
///
/// Colors are stored as u8 normalized (4 bytes/vertex instead of 16).
/// Indices use u16 when vertex_count <= 65535.
pub fn write_glb(
    mesh: &IndexedMesh,
    materials: &MaterialLibrary,
    atlas_texture: Option<&TextureData>,
) -> Vec<u8> {
    write_glb_impl(mesh, materials, atlas_texture, false)
}

/// Serialize an `IndexedMesh` into a compressed GLB with EXT_meshopt_compression.
///
/// Same as `write_glb` but applies meshopt buffer encoding to vertex attribute
/// and index buffers. Viewers must support EXT_meshopt_compression to load these.
/// Achieves 50-70% size reduction compared to uncompressed GLB.
pub fn write_glb_compressed(
    mesh: &IndexedMesh,
    materials: &MaterialLibrary,
    atlas_texture: Option<&TextureData>,
) -> Vec<u8> {
    write_glb_impl(mesh, materials, atlas_texture, true)
}

fn write_glb_impl(
    mesh: &IndexedMesh,
    materials: &MaterialLibrary,
    atlas_texture: Option<&TextureData>,
    compress: bool,
) -> Vec<u8> {
    if mesh.is_empty() {
        return write_empty_glb();
    }

    let mut root = gltf_json::Root {
        asset: gltf_json::Asset {
            version: "2.0".into(),
            generator: Some("photo-tiler".into()),
            ..Default::default()
        },
        ..Default::default()
    };

    // Build binary buffer data
    let mut bin_data: Vec<u8> = Vec::new();
    let mut attributes = BTreeMap::new();

    let buffer_idx = Index::new(0); // will push buffer at end

    // --- Positions (required) ---
    let (pos_min, pos_max) = compute_position_bounds(&mesh.positions);
    let pos_encoded = if compress {
        encode_f32x3(&mesh.positions)
    } else {
        None
    };
    let pos_view = write_vertex_attribute_view(
        &mut root,
        &mut bin_data,
        buffer_idx,
        bytemuck::cast_slice(&mesh.positions),
        12, // stride: 3 * f32
        mesh.vertex_count(),
        pos_encoded,
    );

    let pos_accessor = root.push(gltf_json::Accessor {
        buffer_view: Some(pos_view),
        byte_offset: Some(USize64(0)),
        count: USize64::from(mesh.vertex_count()),
        component_type: Checked::Valid(GenericComponentType(ComponentType::F32)),
        type_: Checked::Valid(AccessorType::Vec3),
        min: Some(serde_json::json!(pos_min)),
        max: Some(serde_json::json!(pos_max)),
        name: None,
        normalized: false,
        sparse: None,
        extensions: Default::default(),
        extras: Default::default(),
    });
    attributes.insert(Checked::Valid(Semantic::Positions), pos_accessor);

    // --- Normals (optional) ---
    if mesh.has_normals() {
        let normals_encoded = if compress {
            encode_f32x3(&mesh.normals)
        } else {
            None
        };
        let view = write_vertex_attribute_view(
            &mut root,
            &mut bin_data,
            buffer_idx,
            bytemuck::cast_slice(&mesh.normals),
            12, // stride: 3 * f32
            mesh.vertex_count(),
            normals_encoded,
        );

        let accessor = root.push(gltf_json::Accessor {
            buffer_view: Some(view),
            byte_offset: Some(USize64(0)),
            count: USize64::from(mesh.vertex_count()),
            component_type: Checked::Valid(GenericComponentType(ComponentType::F32)),
            type_: Checked::Valid(AccessorType::Vec3),
            min: None,
            max: None,
            name: None,
            normalized: false,
            sparse: None,
            extensions: Default::default(),
            extras: Default::default(),
        });
        attributes.insert(Checked::Valid(Semantic::Normals), accessor);
    }

    // --- UVs (optional) ---
    if mesh.has_uvs() {
        let uvs_encoded = if compress {
            encode_f32x2(&mesh.uvs)
        } else {
            None
        };
        let view = write_vertex_attribute_view(
            &mut root,
            &mut bin_data,
            buffer_idx,
            bytemuck::cast_slice(&mesh.uvs),
            8, // stride: 2 * f32
            mesh.vertex_count(),
            uvs_encoded,
        );

        let accessor = root.push(gltf_json::Accessor {
            buffer_view: Some(view),
            byte_offset: Some(USize64(0)),
            count: USize64::from(mesh.vertex_count()),
            component_type: Checked::Valid(GenericComponentType(ComponentType::F32)),
            type_: Checked::Valid(AccessorType::Vec2),
            min: None,
            max: None,
            name: None,
            normalized: false,
            sparse: None,
            extensions: Default::default(),
            extras: Default::default(),
        });
        attributes.insert(Checked::Valid(Semantic::TexCoords(0)), accessor);
    }

    // --- Colors (optional, stored as u8 normalized) ---
    if mesh.has_colors() {
        // Convert f32 colors to u8 (4 bytes per vertex instead of 16)
        let color_u8: Vec<u8> = mesh
            .colors
            .iter()
            .map(|&c| (c * 255.0).round().clamp(0.0, 255.0) as u8)
            .collect();

        let colors_encoded = if compress {
            encode_u8x4(&color_u8)
        } else {
            None
        };
        let view = write_vertex_attribute_view(
            &mut root,
            &mut bin_data,
            buffer_idx,
            &color_u8,
            4, // stride: 4 * u8
            mesh.vertex_count(),
            colors_encoded,
        );

        let accessor = root.push(gltf_json::Accessor {
            buffer_view: Some(view),
            byte_offset: Some(USize64(0)),
            count: USize64::from(mesh.vertex_count()),
            component_type: Checked::Valid(GenericComponentType(ComponentType::U8)),
            type_: Checked::Valid(AccessorType::Vec4),
            min: None,
            max: None,
            name: None,
            normalized: true,
            sparse: None,
            extensions: Default::default(),
            extras: Default::default(),
        });
        attributes.insert(Checked::Valid(Semantic::Colors(0)), accessor);
    }

    // --- Indices (u16 when vertex_count <= 65535, else u32) ---
    let use_u16_indices = mesh.vertex_count() <= 65535;
    let idx_encoded = if compress {
        meshopt::encode_index_buffer(&mesh.indices, mesh.vertex_count()).ok()
    } else {
        None
    };
    let idx_view = write_index_view(
        &mut root,
        &mut bin_data,
        buffer_idx,
        &mesh.indices,
        mesh.vertex_count(),
        use_u16_indices,
        idx_encoded,
    );

    let idx_component_type = if use_u16_indices {
        ComponentType::U16
    } else {
        ComponentType::U32
    };

    let idx_accessor = root.push(gltf_json::Accessor {
        buffer_view: Some(idx_view),
        byte_offset: Some(USize64(0)),
        count: USize64::from(mesh.indices.len()),
        component_type: Checked::Valid(GenericComponentType(idx_component_type)),
        type_: Checked::Valid(AccessorType::Scalar),
        min: None,
        max: None,
        name: None,
        normalized: false,
        sparse: None,
        extensions: Default::default(),
        extras: Default::default(),
    });

    // --- Texture (optional) ---
    let texture_index = if let Some(tex) = atlas_texture {
        // Pad to 4-byte alignment before texture data
        while bin_data.len() % 4 != 0 {
            bin_data.push(0);
        }
        let tex_byte_offset = bin_data.len();
        bin_data.extend_from_slice(&tex.data);
        let tex_byte_length = tex.data.len();

        let tex_view = root.push(gltf_json::buffer::View {
            buffer: buffer_idx,
            byte_length: USize64::from(tex_byte_length),
            byte_offset: Some(USize64::from(tex_byte_offset)),
            byte_stride: None,
            name: None,
            target: None, // no target for image buffer views
            extensions: Default::default(),
            extras: Default::default(),
        });

        let image_idx = root.push(gltf_json::Image {
            buffer_view: Some(tex_view),
            mime_type: Some(gltf_json::image::MimeType(tex.mime_type.clone())),
            uri: None,
            name: None,
            extensions: Default::default(),
            extras: Default::default(),
        });

        let sampler_idx = root.push(gltf_json::texture::Sampler {
            mag_filter: Some(Checked::Valid(gltf_json::texture::MagFilter::Linear)),
            min_filter: Some(Checked::Valid(gltf_json::texture::MinFilter::LinearMipmapLinear)),
            wrap_s: Checked::Valid(gltf_json::texture::WrappingMode::ClampToEdge),
            wrap_t: Checked::Valid(gltf_json::texture::WrappingMode::ClampToEdge),
            name: None,
            extensions: Default::default(),
            extras: Default::default(),
        });

        let tex_idx = root.push(gltf_json::Texture {
            sampler: Some(sampler_idx),
            source: image_idx,
            name: None,
            extensions: Default::default(),
            extras: Default::default(),
        });

        Some(tex_idx)
    } else {
        None
    };

    // --- Material (optional) ---
    let material_index = build_material(&mut root, mesh.material_index, materials, texture_index);

    // --- Mesh ---
    let primitive = Primitive {
        attributes,
        indices: Some(idx_accessor),
        material: material_index,
        mode: Checked::Valid(Mode::Triangles),
        targets: None,
        extensions: Default::default(),
        extras: Default::default(),
    };

    let mesh_idx = root.push(gltf_json::Mesh {
        primitives: vec![primitive],
        weights: None,
        name: None,
        extensions: Default::default(),
        extras: Default::default(),
    });

    // --- Node ---
    let node_idx = root.push(gltf_json::Node {
        mesh: Some(mesh_idx),
        ..Default::default()
    });

    // --- Scene ---
    let scene_idx = root.push(gltf_json::Scene {
        nodes: vec![node_idx],
        name: None,
        extensions: Default::default(),
        extras: Default::default(),
    });
    root.scene = Some(scene_idx);

    // --- Extensions used/required (when compressed) ---
    if compress {
        let ext = "EXT_meshopt_compression".to_string();
        root.extensions_used.push(ext.clone());
        root.extensions_required.push(ext);
    }

    // KHR_texture_basisu when atlas texture is KTX2/Basis
    if let Some(tex) = atlas_texture {
        if tex.mime_type == "image/ktx2" {
            let ext = "KHR_texture_basisu".to_string();
            root.extensions_used.push(ext.clone());
            root.extensions_required.push(ext);
        }
    }

    // --- Buffer (the one buffer holding all data) ---
    // Pad binary data to 4-byte alignment
    while bin_data.len() % 4 != 0 {
        bin_data.push(0);
    }

    root.push(gltf_json::Buffer {
        byte_length: USize64::from(bin_data.len()),
        uri: None,
        name: None,
        extensions: Default::default(),
        extras: Default::default(),
    });

    // --- Assemble GLB ---
    let json_string = gltf_json::serialize::to_string(&root).expect("gltf-json serialization");
    let mut json_bytes = json_string.into_bytes();
    // Pad JSON to 4-byte alignment with spaces (per GLB spec)
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }

    let glb = Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: (12 + 8 + json_bytes.len() + 8 + bin_data.len()) as u32,
        },
        json: Cow::Owned(json_bytes),
        bin: Some(Cow::Owned(bin_data)),
    };

    glb.to_vec().expect("GLB serialization")
}

/// Encode a flat f32 array as [f32; 3] vertex data using meshopt.
fn encode_f32x3(data: &[f32]) -> Option<Vec<u8>> {
    let vertices: &[[f32; 3]] = bytemuck::cast_slice(data);
    meshopt::encode_vertex_buffer(vertices).ok()
}

/// Encode a flat f32 array as [f32; 2] vertex data using meshopt.
fn encode_f32x2(data: &[f32]) -> Option<Vec<u8>> {
    let vertices: &[[f32; 2]] = bytemuck::cast_slice(data);
    meshopt::encode_vertex_buffer(vertices).ok()
}

/// Encode a flat u8 array as [u8; 4] vertex data using meshopt.
fn encode_u8x4(data: &[u8]) -> Option<Vec<u8>> {
    let vertices: &[[u8; 4]] = bytemuck::cast_slice(data);
    meshopt::encode_vertex_buffer(vertices).ok()
}

/// Write a vertex attribute buffer view, optionally with meshopt compression.
///
/// Returns the buffer view index. When compressed, the buffer view has the
/// EXT_meshopt_compression extension with mode = ATTRIBUTES.
///
/// `encoded_data` should be `Some(encoded_bytes)` when compressing, `None` otherwise.
/// This allows the caller to use the correct typed encoding function.
fn write_vertex_attribute_view(
    root: &mut gltf_json::Root,
    bin_data: &mut Vec<u8>,
    buffer_idx: Index<gltf_json::Buffer>,
    raw_bytes: &[u8],
    stride: usize,
    vertex_count: usize,
    encoded_data: Option<Vec<u8>>,
) -> Index<gltf_json::buffer::View> {
    // Pad to 4-byte alignment
    while bin_data.len() % 4 != 0 {
        bin_data.push(0);
    }

    if let Some(encoded) = encoded_data {
        let byte_offset = bin_data.len();
        bin_data.extend_from_slice(&encoded);
        let byte_length = encoded.len();

        // Build the EXT_meshopt_compression extension data
        let mut ext_map = serde_json::Map::new();
        ext_map.insert(
            "EXT_meshopt_compression".into(),
            serde_json::json!({
                "buffer": 0,
                "byteOffset": byte_offset,
                "byteLength": byte_length,
                "byteStride": stride,
                "count": vertex_count,
                "mode": "ATTRIBUTES"
            }),
        );

        root.push(gltf_json::buffer::View {
            buffer: buffer_idx,
            byte_length: USize64::from(byte_length),
            byte_offset: Some(USize64::from(byte_offset)),
            byte_stride: None, // no stride on compressed views
            name: None,
            target: None, // no target on compressed views
            extensions: Some(gltf_json::extensions::buffer::View { others: ext_map }),
            extras: Default::default(),
        })
    } else {
        let byte_offset = bin_data.len();
        bin_data.extend_from_slice(raw_bytes);
        let byte_length = raw_bytes.len();

        root.push(gltf_json::buffer::View {
            buffer: buffer_idx,
            byte_length: USize64::from(byte_length),
            byte_offset: Some(USize64::from(byte_offset)),
            byte_stride: None,
            name: None,
            target: Some(Checked::Valid(Target::ArrayBuffer)),
            extensions: Default::default(),
            extras: Default::default(),
        })
    }
}

/// Write an index buffer view, optionally with meshopt compression.
///
/// `encoded_data` should be `Some(encoded_bytes)` when compressing, `None` otherwise.
fn write_index_view(
    root: &mut gltf_json::Root,
    bin_data: &mut Vec<u8>,
    buffer_idx: Index<gltf_json::Buffer>,
    indices: &[u32],
    _vertex_count: usize,
    use_u16: bool,
    encoded_data: Option<Vec<u8>>,
) -> Index<gltf_json::buffer::View> {
    // Pad to 4-byte alignment before indices
    while bin_data.len() % 4 != 0 {
        bin_data.push(0);
    }

    if let Some(encoded) = encoded_data {
        let byte_offset = bin_data.len();
        bin_data.extend_from_slice(&encoded);
        let byte_length = encoded.len();

        let index_byte_stride = if use_u16 { 2 } else { 4 };

        let mut ext_map = serde_json::Map::new();
        ext_map.insert(
            "EXT_meshopt_compression".into(),
            serde_json::json!({
                "buffer": 0,
                "byteOffset": byte_offset,
                "byteLength": byte_length,
                "byteStride": index_byte_stride,
                "count": indices.len(),
                "mode": "TRIANGLES"
            }),
        );

        root.push(gltf_json::buffer::View {
            buffer: buffer_idx,
            byte_length: USize64::from(byte_length),
            byte_offset: Some(USize64::from(byte_offset)),
            byte_stride: None,
            name: None,
            target: None,
            extensions: Some(gltf_json::extensions::buffer::View { others: ext_map }),
            extras: Default::default(),
        })
    } else {
        let byte_offset = bin_data.len();
        if use_u16 {
            let idx_u16: Vec<u16> = indices.iter().map(|&i| i as u16).collect();
            let idx_bytes: &[u8] = bytemuck::cast_slice(&idx_u16);
            bin_data.extend_from_slice(idx_bytes);
        } else {
            let idx_bytes: &[u8] = bytemuck::cast_slice(indices);
            bin_data.extend_from_slice(idx_bytes);
        }
        let byte_length = if use_u16 {
            indices.len() * 2
        } else {
            indices.len() * 4
        };

        root.push(gltf_json::buffer::View {
            buffer: buffer_idx,
            byte_length: USize64::from(byte_length),
            byte_offset: Some(USize64::from(byte_offset)),
            byte_stride: None,
            name: None,
            target: Some(Checked::Valid(Target::ElementArrayBuffer)),
            extensions: Default::default(),
            extras: Default::default(),
        })
    }
}

/// Produce a minimal valid empty GLB.
fn write_empty_glb() -> Vec<u8> {
    let mut root = gltf_json::Root {
        asset: gltf_json::Asset {
            version: "2.0".into(),
            generator: Some("photo-tiler".into()),
            ..Default::default()
        },
        ..Default::default()
    };

    let node_idx = root.push(gltf_json::Node::default());
    let scene_idx = root.push(gltf_json::Scene {
        nodes: vec![node_idx],
        name: None,
        extensions: Default::default(),
        extras: Default::default(),
    });
    root.scene = Some(scene_idx);

    let json_string = gltf_json::serialize::to_string(&root).expect("gltf-json serialization");
    let mut json_bytes = json_string.into_bytes();
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }

    let glb = Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: (12 + 8 + json_bytes.len()) as u32,
        },
        json: Cow::Owned(json_bytes),
        bin: None,
    };

    glb.to_vec().expect("GLB serialization")
}

/// Build a gltf-json Material if the mesh references one in the library.
fn build_material(
    root: &mut gltf_json::Root,
    material_index: Option<usize>,
    materials: &MaterialLibrary,
    texture_index: Option<Index<gltf_json::Texture>>,
) -> Option<Index<gltf_json::Material>> {
    let mat_idx = material_index?;
    let mat = materials.materials.get(mat_idx)?;

    let base_color_texture = texture_index.map(|idx| gltf_json::texture::Info {
        index: idx,
        tex_coord: 0,
        extensions: Default::default(),
        extras: Default::default(),
    });

    let pbr = gltf_json::material::PbrMetallicRoughness {
        base_color_factor: gltf_json::material::PbrBaseColorFactor(mat.base_color),
        metallic_factor: gltf_json::material::StrengthFactor(mat.metallic),
        roughness_factor: gltf_json::material::StrengthFactor(mat.roughness),
        base_color_texture,
        metallic_roughness_texture: None,
        extensions: Default::default(),
        extras: Default::default(),
    };

    let gltf_mat = gltf_json::Material {
        pbr_metallic_roughness: pbr,
        alpha_mode: Checked::Valid(gltf_json::material::AlphaMode::Opaque),
        alpha_cutoff: None,
        double_sided: false,
        normal_texture: None,
        occlusion_texture: None,
        emissive_texture: None,
        emissive_factor: gltf_json::material::EmissiveFactor([0.0, 0.0, 0.0]),
        name: None,
        extensions: Default::default(),
        extras: Default::default(),
    };

    Some(root.push(gltf_mat))
}

/// Compute min/max for a flat positions array (stride 3).
fn compute_position_bounds(positions: &[f32]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for chunk in positions.chunks_exact(3) {
        for i in 0..3 {
            min[i] = min[i].min(chunk[i]);
            max[i] = max[i].max(chunk[i]);
        }
    }

    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PBRMaterial;

    fn make_triangle() -> IndexedMesh {
        IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            colors: vec![],
            indices: vec![0, 1, 2],
            material_index: None,
        }
    }

    fn make_colored_triangle() -> IndexedMesh {
        IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            normals: vec![],
            uvs: vec![],
            colors: vec![
                1.0, 0.0, 0.0, 1.0, // red
                0.0, 1.0, 0.0, 1.0, // green
                0.0, 0.0, 1.0, 1.0, // blue
            ],
            indices: vec![0, 1, 2],
            material_index: None,
        }
    }

    #[test]
    fn glb_magic_bytes() {
        let mesh = make_triangle();
        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        assert!(bytes.len() >= 4);
        assert_eq!(&bytes[0..4], b"glTF", "GLB magic should be 'glTF'");
    }

    #[test]
    fn glb_version_2() {
        let mesh = make_triangle();
        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        assert_eq!(version, 2, "GLB version should be 2");
    }

    #[test]
    fn glb_roundtrip_parseable() {
        let mesh = make_triangle();
        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        let glb = Glb::from_slice(&bytes).expect("GLB should be parseable");
        assert_eq!(&glb.header.magic, b"glTF");
        assert_eq!(glb.header.version, 2);
        assert!(glb.bin.is_some());
    }

    #[test]
    fn glb_roundtrip_vertex_count() {
        let mesh = make_triangle();
        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        let (doc, _buffers, _images) =
            gltf::import_slice(&bytes).expect("GLB should import cleanly");

        let gltf_mesh = doc.meshes().next().expect("should have 1 mesh");
        let prim = gltf_mesh.primitives().next().expect("should have 1 primitive");

        let pos_accessor = prim
            .get(&Semantic::Positions)
            .expect("should have positions");
        assert_eq!(pos_accessor.count(), 3, "should have 3 vertices");
    }

    #[test]
    fn glb_roundtrip_triangle_count() {
        let mesh = make_triangle();
        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        let (doc, _buffers, _images) = gltf::import_slice(&bytes).unwrap();
        let gltf_mesh = doc.meshes().next().unwrap();
        let prim = gltf_mesh.primitives().next().unwrap();

        let idx_accessor = prim.indices().expect("should have indices");
        assert_eq!(idx_accessor.count(), 3, "1 triangle = 3 indices");
    }

    #[test]
    fn glb_roundtrip_with_normals_and_uvs() {
        let mesh = make_triangle();
        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        let (doc, _buffers, _images) = gltf::import_slice(&bytes).unwrap();
        let prim = doc.meshes().next().unwrap().primitives().next().unwrap();

        assert!(
            prim.get(&Semantic::Normals).is_some(),
            "should have normals"
        );
        assert!(
            prim.get(&Semantic::TexCoords(0)).is_some(),
            "should have UVs"
        );
    }

    #[test]
    fn glb_roundtrip_with_colors() {
        let mesh = make_colored_triangle();
        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        let (doc, _buffers, _images) = gltf::import_slice(&bytes).unwrap();
        let prim = doc.meshes().next().unwrap().primitives().next().unwrap();

        assert!(
            prim.get(&Semantic::Colors(0)).is_some(),
            "should have vertex colors"
        );
    }

    #[test]
    fn glb_u8_colors_smaller_than_f32() {
        let mesh = make_colored_triangle();
        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        let (doc, _buffers, _images) = gltf::import_slice(&bytes).unwrap();
        let prim = doc.meshes().next().unwrap().primitives().next().unwrap();
        let color_accessor = prim.get(&Semantic::Colors(0)).unwrap();

        // Colors should be u8 normalized
        assert_eq!(
            color_accessor.data_type(),
            gltf::accessor::DataType::U8,
            "colors should be stored as u8"
        );
        assert!(color_accessor.normalized(), "u8 colors should be normalized");
    }

    #[test]
    fn glb_u16_indices_for_small_mesh() {
        let mesh = make_triangle(); // 3 vertices < 65535
        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        let (doc, _buffers, _images) = gltf::import_slice(&bytes).unwrap();
        let prim = doc.meshes().next().unwrap().primitives().next().unwrap();
        let idx_accessor = prim.indices().unwrap();

        assert_eq!(
            idx_accessor.data_type(),
            gltf::accessor::DataType::U16,
            "small mesh should use u16 indices"
        );
    }

    #[test]
    fn glb_empty_mesh() {
        let mesh = IndexedMesh::default();
        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        assert_eq!(&bytes[0..4], b"glTF");
        let glb = Glb::from_slice(&bytes).expect("empty GLB should be parseable");
        assert_eq!(glb.header.version, 2);
    }

    #[test]
    fn glb_with_material() {
        let mesh = IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: vec![0, 1, 2],
            material_index: Some(0),
            ..Default::default()
        };
        let mut materials = MaterialLibrary::default();
        materials.materials.push(PBRMaterial {
            name: "test".into(),
            base_color: [0.8, 0.2, 0.1, 1.0],
            metallic: 0.5,
            roughness: 0.7,
            base_color_texture: None,
        });

        let bytes = write_glb(&mesh, &materials, None);

        let (doc, _buffers, _images) = gltf::import_slice(&bytes).unwrap();
        let mat = doc.materials().next().expect("should have material");
        let pbr = mat.pbr_metallic_roughness();
        let color = pbr.base_color_factor();
        assert!((color[0] - 0.8).abs() < 1e-3);
        assert!((color[1] - 0.2).abs() < 1e-3);
        assert!((pbr.metallic_factor() - 0.5).abs() < 1e-3);
        assert!((pbr.roughness_factor() - 0.7).abs() < 1e-3);
    }

    #[test]
    fn glb_larger_mesh_roundtrip() {
        let n = 10;
        let verts_per_side = n + 1;
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();

        for y in 0..verts_per_side {
            for x in 0..verts_per_side {
                let fx = x as f32 / n as f32;
                let fy = y as f32 / n as f32;
                positions.extend_from_slice(&[fx, fy, 0.0]);
                normals.extend_from_slice(&[0.0, 0.0, 1.0]);
                uvs.extend_from_slice(&[fx, fy]);
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

        let mesh = IndexedMesh {
            positions,
            normals,
            uvs,
            colors: vec![],
            indices,
            material_index: None,
        };

        let materials = MaterialLibrary::default();
        let bytes = write_glb(&mesh, &materials, None);

        let (doc, buffers, _images) = gltf::import_slice(&bytes).unwrap();
        let gltf_mesh = doc.meshes().next().unwrap();
        let prim = gltf_mesh.primitives().next().unwrap();
        let reader = prim.reader(|buf| Some(&buffers[buf.index()]));

        let pos_count = reader.read_positions().unwrap().count();
        assert_eq!(pos_count, verts_per_side * verts_per_side);

        let idx_count = reader.read_indices().unwrap().into_u32().count();
        assert_eq!(idx_count, n * n * 6);
        assert_eq!(idx_count / 3, 200);
    }

    #[test]
    fn position_bounds_correct() {
        let positions = vec![
            -1.0, 0.0, 2.0, //
            3.0, -4.0, 5.0, //
            0.0, 1.0, -3.0, //
        ];
        let (min, max) = compute_position_bounds(&positions);
        assert_eq!(min, [-1.0, -4.0, -3.0]);
        assert_eq!(max, [3.0, 1.0, 5.0]);
    }

    #[test]
    fn glb_with_texture_roundtrip() {
        let mesh = IndexedMesh {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            material_index: Some(0),
            ..Default::default()
        };
        let mut materials = MaterialLibrary::default();
        materials.materials.push(PBRMaterial {
            name: "textured".into(),
            base_color_texture: Some(0),
            ..Default::default()
        });

        // Create a small PNG texture
        let img = image::RgbaImage::from_fn(4, 4, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 255, 0, 255])
            }
        });
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let atlas = TextureData {
            data: buf.into_inner(),
            mime_type: "image/png".into(),
            width: 4,
            height: 4,
        };

        let bytes = write_glb(&mesh, &materials, Some(&atlas));

        let (doc, _buffers, images) = gltf::import_slice(&bytes).unwrap();

        // Should have a texture
        assert_eq!(doc.textures().count(), 1, "should have 1 texture");
        assert_eq!(doc.images().count(), 1, "should have 1 image");
        assert_eq!(doc.samplers().count(), 1, "should have 1 sampler");

        // Material should reference the texture
        let mat = doc.materials().next().expect("should have material");
        let pbr = mat.pbr_metallic_roughness();
        assert!(
            pbr.base_color_texture().is_some(),
            "material should have base color texture"
        );

        // Image data should be present
        assert!(!images.is_empty(), "should have image data");
        assert_eq!(images[0].width, 4);
        assert_eq!(images[0].height, 4);
    }

    #[test]
    fn glb_compressed_parseable() {
        let mesh = make_triangle();
        let materials = MaterialLibrary::default();
        let bytes = write_glb_compressed(&mesh, &materials, None);

        // Should be a valid GLB container
        assert_eq!(&bytes[0..4], b"glTF");
        let glb = Glb::from_slice(&bytes).expect("compressed GLB should be parseable");
        assert_eq!(glb.header.version, 2);
        assert!(glb.bin.is_some());

        // JSON should contain the extension
        let json_str = std::str::from_utf8(&glb.json).unwrap();
        assert!(
            json_str.contains("EXT_meshopt_compression"),
            "should declare EXT_meshopt_compression extension"
        );
    }

    #[test]
    fn glb_compressed_smaller_than_uncompressed() {
        // Need a larger mesh to see compression benefits
        let n = 20;
        let verts_per_side = n + 1;
        let mut positions = Vec::new();
        let mut normals = Vec::new();

        for y in 0..verts_per_side {
            for x in 0..verts_per_side {
                let fx = x as f32 / n as f32;
                let fy = y as f32 / n as f32;
                positions.extend_from_slice(&[fx, fy, 0.0]);
                normals.extend_from_slice(&[0.0, 0.0, 1.0]);
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

        let mesh = IndexedMesh {
            positions,
            normals,
            uvs: vec![],
            colors: vec![],
            indices,
            material_index: None,
        };

        let materials = MaterialLibrary::default();
        let uncompressed = write_glb(&mesh, &materials, None);
        let compressed = write_glb_compressed(&mesh, &materials, None);

        assert!(
            compressed.len() < uncompressed.len(),
            "compressed ({}) should be smaller than uncompressed ({})",
            compressed.len(),
            uncompressed.len()
        );
    }

    #[test]
    fn glb_compressed_with_colors() {
        let mesh = make_colored_triangle();
        let materials = MaterialLibrary::default();
        let bytes = write_glb_compressed(&mesh, &materials, None);

        assert_eq!(&bytes[0..4], b"glTF");
        let glb = Glb::from_slice(&bytes).expect("compressed GLB with colors should be parseable");
        assert!(glb.bin.is_some());
    }
}
