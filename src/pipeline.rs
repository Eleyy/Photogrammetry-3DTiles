use std::time::{Duration, Instant};

use tracing::info;

use crate::config::PipelineConfig;
use crate::error::Result;
use crate::ingestion::{self, IngestionResult};
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
        let tile_count = Self::tile(config, &transform_result)?;

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

    fn tile(_config: &PipelineConfig, _transform: &TransformResult) -> Result<usize> {
        todo!("Milestone 4: tiling stage")
    }

    fn validate(_config: &PipelineConfig) -> Result<()> {
        todo!("Milestone 5: validation stage")
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
