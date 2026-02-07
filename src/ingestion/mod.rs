pub mod georef;
pub mod gltf_loader;
pub mod obj_loader;
pub mod ply_loader;

use std::path::Path;

use tracing::{debug, info};

use crate::config::{Georeference, PipelineConfig};
use crate::error::{PhotoTilerError, Result};
use crate::types::{IndexedMesh, MaterialLibrary};

/// Result of the ingestion stage.
#[derive(Debug)]
pub struct IngestionResult {
    pub meshes: Vec<IndexedMesh>,
    pub materials: MaterialLibrary,
    pub georeference: Option<Georeference>,
    pub stats: IngestionStats,
}

/// Statistics about the ingested data.
#[derive(Debug)]
pub struct IngestionStats {
    pub total_vertices: usize,
    pub total_triangles: usize,
    pub total_meshes: usize,
    pub has_normals: bool,
    pub has_uvs: bool,
    pub has_colors: bool,
    pub texture_count: usize,
    pub material_count: usize,
    pub input_format: String,
}

/// Supported input formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    Obj,
    Gltf,
    Glb,
    Ply,
}

impl InputFormat {
    /// Detect format from file extension (case-insensitive).
    pub fn from_path(path: &Path) -> Result<Self> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        match ext.as_str() {
            "obj" => Ok(InputFormat::Obj),
            "gltf" => Ok(InputFormat::Gltf),
            "glb" => Ok(InputFormat::Glb),
            "ply" => Ok(InputFormat::Ply),
            _ => Err(PhotoTilerError::Input(format!(
                "Unsupported file format: .{ext}"
            ))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            InputFormat::Obj => "OBJ",
            InputFormat::Gltf => "glTF",
            InputFormat::Glb => "GLB",
            InputFormat::Ply => "PLY",
        }
    }
}

impl std::fmt::Display for InputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Run the full ingestion stage.
pub fn ingest(config: &PipelineConfig) -> Result<IngestionResult> {
    // 1. Validate input exists
    if !config.input.exists() {
        return Err(PhotoTilerError::Input(format!(
            "Input file not found: {}",
            config.input.display()
        )));
    }

    // 2. Detect format
    let format = InputFormat::from_path(&config.input)?;
    info!(format = %format, path = %config.input.display(), "Detected input format");

    // 3. Dispatch to loader
    let (meshes, materials) = match format {
        InputFormat::Obj => obj_loader::load_obj(&config.input, config)?,
        InputFormat::Gltf | InputFormat::Glb => gltf_loader::load_gltf(&config.input)?,
        InputFormat::Ply => {
            let mesh = ply_loader::load_ply(&config.input)?;
            (vec![mesh], MaterialLibrary::default())
        }
    };

    // 4. Compute stats
    let stats = compute_stats(&meshes, &materials, format);
    debug!(
        vertices = stats.total_vertices,
        triangles = stats.total_triangles,
        meshes = stats.total_meshes,
        "Ingestion stats"
    );

    // 5. Detect georeferencing
    let georeference = georef::detect_georeference(config)?;
    if let Some(ref geo) = georeference {
        info!(
            epsg = geo.epsg,
            easting = geo.easting,
            northing = geo.northing,
            elevation = geo.elevation,
            "Detected georeference"
        );
    }

    Ok(IngestionResult {
        meshes,
        materials,
        georeference,
        stats,
    })
}

/// Compute summary statistics from the ingested meshes and materials.
pub fn compute_stats(
    meshes: &[IndexedMesh],
    materials: &MaterialLibrary,
    format: InputFormat,
) -> IngestionStats {
    let total_vertices: usize = meshes.iter().map(|m| m.vertex_count()).sum();
    let total_triangles: usize = meshes.iter().map(|m| m.triangle_count()).sum();
    let has_normals = meshes.iter().any(|m| m.has_normals());
    let has_uvs = meshes.iter().any(|m| m.has_uvs());
    let has_colors = meshes.iter().any(|m| m.has_colors());

    IngestionStats {
        total_vertices,
        total_triangles,
        total_meshes: meshes.len(),
        has_normals,
        has_uvs,
        has_colors,
        texture_count: materials.textures.len(),
        material_count: materials.materials.len(),
        input_format: format.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PBRMaterial;

    #[test]
    fn format_detection_obj() {
        assert_eq!(
            InputFormat::from_path(Path::new("model.obj")).unwrap(),
            InputFormat::Obj
        );
    }

    #[test]
    fn format_detection_gltf() {
        assert_eq!(
            InputFormat::from_path(Path::new("scene.gltf")).unwrap(),
            InputFormat::Gltf
        );
    }

    #[test]
    fn format_detection_glb() {
        assert_eq!(
            InputFormat::from_path(Path::new("scene.glb")).unwrap(),
            InputFormat::Glb
        );
    }

    #[test]
    fn format_detection_ply() {
        assert_eq!(
            InputFormat::from_path(Path::new("cloud.ply")).unwrap(),
            InputFormat::Ply
        );
    }

    #[test]
    fn format_detection_case_insensitive() {
        assert_eq!(
            InputFormat::from_path(Path::new("Model.OBJ")).unwrap(),
            InputFormat::Obj
        );
        assert_eq!(
            InputFormat::from_path(Path::new("Scene.GLTF")).unwrap(),
            InputFormat::Gltf
        );
    }

    #[test]
    fn format_detection_unsupported() {
        assert!(InputFormat::from_path(Path::new("file.fbx")).is_err());
    }

    #[test]
    fn compute_stats_basic() {
        let meshes = vec![
            IndexedMesh {
                positions: vec![0.0; 9],
                normals: vec![0.0; 9],
                uvs: vec![0.0; 6],
                colors: vec![],
                indices: vec![0, 1, 2],
                material_index: Some(0),
            },
            IndexedMesh {
                positions: vec![0.0; 12],
                normals: vec![],
                uvs: vec![],
                colors: vec![0.0; 16],
                indices: vec![0, 1, 2, 0, 2, 3],
                material_index: None,
            },
        ];

        let mut lib = MaterialLibrary::default();
        lib.materials.push(PBRMaterial::default());

        let stats = compute_stats(&meshes, &lib, InputFormat::Obj);

        assert_eq!(stats.total_vertices, 7); // 3 + 4
        assert_eq!(stats.total_triangles, 3); // 1 + 2
        assert_eq!(stats.total_meshes, 2);
        assert!(stats.has_normals);
        assert!(stats.has_uvs);
        assert!(stats.has_colors);
        assert_eq!(stats.material_count, 1);
        assert_eq!(stats.texture_count, 0);
        assert_eq!(stats.input_format, "OBJ");
    }

    #[test]
    fn ingest_missing_file() {
        let config = PipelineConfig {
            input: std::path::PathBuf::from("/nonexistent/file.obj"),
            ..Default::default()
        };
        let err = ingest(&config).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }
}
