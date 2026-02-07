/// Raw texture image data.
#[derive(Debug, Clone)]
pub struct TextureData {
    pub data: Vec<u8>,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
}

/// PBR metallic-roughness material.
#[derive(Debug, Clone)]
pub struct PBRMaterial {
    pub name: String,
    /// Base color factor [r, g, b, a].
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    /// Index into `MaterialLibrary::textures`.
    pub base_color_texture: Option<usize>,
}

impl Default for PBRMaterial {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_color: [1.0, 1.0, 1.0, 1.0],
            metallic: 0.0,
            roughness: 1.0,
            base_color_texture: None,
        }
    }
}

/// Collection of materials and their associated textures.
#[derive(Debug, Clone, Default)]
pub struct MaterialLibrary {
    pub materials: Vec<PBRMaterial>,
    pub textures: Vec<TextureData>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbr_material_defaults() {
        let mat = PBRMaterial::default();
        assert_eq!(mat.name, "");
        assert_eq!(mat.base_color, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(mat.metallic, 0.0);
        assert_eq!(mat.roughness, 1.0);
        assert_eq!(mat.base_color_texture, None);
    }

    #[test]
    fn material_library_construction() {
        let mut lib = MaterialLibrary::default();
        assert!(lib.materials.is_empty());
        assert!(lib.textures.is_empty());

        lib.textures.push(TextureData {
            data: vec![0xFF; 4],
            mime_type: "image/png".into(),
            width: 1,
            height: 1,
        });

        lib.materials.push(PBRMaterial {
            name: "brick".into(),
            base_color_texture: Some(0),
            ..Default::default()
        });

        assert_eq!(lib.materials.len(), 1);
        assert_eq!(lib.textures.len(), 1);
        assert_eq!(lib.materials[0].name, "brick");
        assert_eq!(lib.materials[0].base_color_texture, Some(0));
    }
}
