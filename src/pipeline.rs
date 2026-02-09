use std::fs;
use std::time::{Duration, Instant};

use gltf::binary::Glb;
use tracing::{info, warn};

use crate::config::PipelineConfig;
use crate::error::{PhotoTilerError, Result};
use crate::ingestion::{self, IngestionResult};
use crate::tiling::{lod, tileset_writer};
use crate::transform::{self, TransformResult};

/// Summary of a completed pipeline run.
#[derive(Debug)]
pub struct ProcessingResult {
    pub tile_count: usize,
    pub duration: Duration,
}

/// Pipeline orchestrator -- drives the four conversion stages.
pub struct Pipeline;

impl Pipeline {
    /// Run the full conversion pipeline.
    pub fn run(config: &PipelineConfig) -> Result<ProcessingResult> {
        let start = Instant::now();

        info!(input = %config.input.display(), "Starting pipeline");

        // Early exits
        if config.show_georef {
            info!("--show-georef: detecting georeferencing information");
            let result = ingestion::ingest(config)?;
            print_georef(&result);
            return Ok(ProcessingResult {
                tile_count: 0,
                duration: start.elapsed(),
            });
        }

        if config.dry_run {
            info!("--dry-run: scanning input and computing transforms");
            let ingestion_result = ingestion::ingest(config)?;
            let transform_result = transform::transform(config, &ingestion_result)?;
            print_dry_run_summary(&ingestion_result, &transform_result);
            return Ok(ProcessingResult {
                tile_count: 0,
                duration: start.elapsed(),
            });
        }

        // Full pipeline
        info!("Stage 1/4: Ingestion");
        let ingestion_result = ingestion::ingest(config)?;

        info!("Stage 2/4: Transform");
        let transform_result = transform::transform(config, &ingestion_result)?;
        print_transform_summary(&transform_result);

        info!("Stage 3/4: Tiling");
        fs::create_dir_all(&config.output).map_err(|e| {
            PhotoTilerError::Output(format!(
                "Failed to create output directory {}: {e}",
                config.output.display()
            ))
        })?;
        let tile_count = Self::tile(config, transform_result)?;

        if config.validate {
            info!("Stage 4/4: Validation");
            Self::validate(config)?;
        }

        let duration = start.elapsed();
        info!(tiles = tile_count, elapsed = ?duration, "Pipeline complete");

        Ok(ProcessingResult {
            tile_count,
            duration,
        })
    }

    fn tile(config: &PipelineConfig, transform_result: TransformResult) -> Result<usize> {
        let max_lod_levels = 1;

        // Destructure to take ownership of fields individually
        let TransformResult {
            meshes,
            bounds,
            materials,
            root_transform,
        } = transform_result;

        let mesh_count = meshes.len();

        // Move meshes into LOD generation (no extra copies)
        let lod_chains: Vec<_> = meshes
            .into_iter()
            .enumerate()
            .map(|(i, mesh)| {
                info!(
                    mesh = i,
                    vertices = mesh.vertex_count(),
                    triangles = mesh.triangle_count(),
                    "Generating LOD chain"
                );

                let chain = lod::generate_lod_chain(mesh, &bounds, max_lod_levels);

                for level in &chain.levels {
                    info!(
                        mesh = i,
                        lod = level.level,
                        triangles = level.mesh.triangle_count(),
                        geometric_error = level.geometric_error,
                        "LOD level"
                    );
                }

                chain
            })
            .collect();

        let total_lod_levels: usize = lod_chains.iter().map(|c| c.levels.len()).sum();
        info!(
            meshes = mesh_count,
            total_lod_levels,
            "LOD generation complete"
        );

        // Build tile hierarchy and write GLBs eagerly to disk
        info!("Building tile hierarchy");
        let tileset_output = tileset_writer::build_tileset(
            lod_chains,
            &bounds,
            &config.tiling,
            &materials,
            &config.texture,
            &config.output,
        );

        // Write tileset.json (GLBs already on disk)
        info!(output = %config.output.display(), "Writing tileset.json");
        let tile_count =
            tileset_writer::write_tileset(&tileset_output, &root_transform, &config.output)?;

        Ok(tile_count)
    }

    fn validate(config: &PipelineConfig) -> Result<()> {
        let out_dir = &config.output;

        // 1. tileset.json must exist and be valid JSON
        let tileset_path = out_dir.join("tileset.json");
        let json_str = fs::read_to_string(&tileset_path).map_err(|e| {
            PhotoTilerError::Validation(format!(
                "Cannot read tileset.json at {}: {e}",
                tileset_path.display()
            ))
        })?;

        let tileset: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
            PhotoTilerError::Validation(format!("tileset.json is not valid JSON: {e}"))
        })?;

        // 2. Required top-level fields
        let asset = tileset
            .get("asset")
            .ok_or_else(|| PhotoTilerError::Validation("Missing 'asset' field".into()))?;
        let version = asset
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if version != "1.1" {
            return Err(PhotoTilerError::Validation(format!(
                "Expected asset.version '1.1', got '{version}'"
            )));
        }

        let root = tileset
            .get("root")
            .ok_or_else(|| PhotoTilerError::Validation("Missing 'root' tile".into()))?;

        // 3. Walk tile tree: validate each tile
        let mut tile_count = 0;
        let mut glb_count = 0;
        let mut errors = Vec::new();
        validate_tile(root, out_dir, None, &mut tile_count, &mut glb_count, &mut errors);

        for err in &errors {
            warn!("Validation: {err}");
        }

        if errors.is_empty() {
            info!(tiles = tile_count, glbs = glb_count, "Validation passed");
        } else {
            return Err(PhotoTilerError::Validation(format!(
                "{} issues found: {}",
                errors.len(),
                errors.first().unwrap()
            )));
        }

        Ok(())
    }
}

/// Recursively validate a tile node from tileset.json.
fn validate_tile(
    tile: &serde_json::Value,
    out_dir: &std::path::Path,
    parent_error: Option<f64>,
    tile_count: &mut usize,
    glb_count: &mut usize,
    errors: &mut Vec<String>,
) {
    *tile_count += 1;

    // Bounding volume must exist
    if tile.get("boundingVolume").is_none() {
        errors.push(format!("Tile {tile_count}: missing boundingVolume"));
    }

    // Geometric error must be non-negative
    let geo_error = tile
        .get("geometricError")
        .and_then(|v| v.as_f64())
        .unwrap_or(-1.0);
    if geo_error < 0.0 {
        errors.push(format!("Tile {tile_count}: invalid geometricError {geo_error}"));
    }

    // Geometric error should not exceed parent's
    if let Some(parent_err) = parent_error {
        if geo_error > parent_err + 1e-6 {
            errors.push(format!(
                "Tile {tile_count}: geometricError {geo_error} > parent {parent_err}"
            ));
        }
    }

    // If tile has content, verify the GLB file
    if let Some(content) = tile.get("content") {
        if let Some(uri) = content.get("uri").and_then(|u| u.as_str()) {
            let glb_path = out_dir.join(uri);
            if !glb_path.exists() {
                errors.push(format!("Tile {tile_count}: GLB not found: {uri}"));
            } else {
                *glb_count += 1;
                // Try to parse the GLB
                match fs::read(&glb_path) {
                    Ok(data) => {
                        if Glb::from_slice(&data).is_err() {
                            errors.push(format!("Tile {tile_count}: GLB not parseable: {uri}"));
                        }
                    }
                    Err(e) => {
                        errors.push(format!("Tile {tile_count}: cannot read {uri}: {e}"));
                    }
                }
            }
        }
    }

    // Recurse into children
    if let Some(children) = tile.get("children").and_then(|c| c.as_array()) {
        for child in children {
            validate_tile(child, out_dir, Some(geo_error), tile_count, glb_count, errors);
        }
    }
}

/// Print georeferencing information and exit.
fn print_georef(result: &IngestionResult) {
    println!("=== Georeferencing ===");
    match &result.georeference {
        Some(geo) => {
            println!("  EPSG:      {}", geo.epsg);
            println!("  Easting:   {:.3}", geo.easting);
            println!("  Northing:  {:.3}", geo.northing);
            println!("  Elevation: {:.3}", geo.elevation);
            println!("  True North:{:.1}°", geo.true_north);
        }
        None => {
            println!("  No georeference detected.");
            println!("  Use --epsg, --offset-file, or --metadata-xml to specify.");
        }
    }
}

/// Print transform summary: bounding box and root transform.
fn print_transform_summary(result: &TransformResult) {
    let bb = &result.bounds;
    println!("=== Transform ===");
    println!(
        "  Bounding box: ({:.3}, {:.3}, {:.3}) → ({:.3}, {:.3}, {:.3})",
        bb.min[0], bb.min[1], bb.min[2], bb.max[0], bb.max[1], bb.max[2]
    );
    println!("  Diagonal:     {:.3} m", bb.diagonal());

    let rt = &result.root_transform;
    let is_identity = rt[0] == 1.0
        && rt[5] == 1.0
        && rt[10] == 1.0
        && rt[15] == 1.0
        && rt[12] == 0.0
        && rt[13] == 0.0
        && rt[14] == 0.0;

    if is_identity {
        println!("  Root transform: identity (local coordinates)");
    } else {
        println!(
            "  Root transform: ECEF ({:.1}, {:.1}, {:.1})",
            rt[12], rt[13], rt[14]
        );
    }
}

/// Print dry-run summary with mesh stats, georeferencing, and transform info.
fn print_dry_run_summary(ingestion: &IngestionResult, transform: &TransformResult) {
    let stats = &ingestion.stats;
    println!("=== Dry Run Summary ===");
    println!("  Format:    {}", stats.input_format);
    println!("  Meshes:    {}", stats.total_meshes);
    println!("  Vertices:  {}", stats.total_vertices);
    println!("  Triangles: {}", stats.total_triangles);
    println!("  Normals:   {}", if stats.has_normals { "yes" } else { "no" });
    println!("  UVs:       {}", if stats.has_uvs { "yes" } else { "no" });
    println!("  Colors:    {}", if stats.has_colors { "yes" } else { "no" });
    println!("  Materials: {}", stats.material_count);
    println!("  Textures:  {}", stats.texture_count);
    println!();
    print_georef(ingestion);
    println!();
    print_transform_summary(transform);
}
