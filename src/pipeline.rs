use std::time::{Duration, Instant};

use tracing::info;

use crate::config::PipelineConfig;
use crate::error::Result;
use crate::ingestion::{self, IngestionResult};

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
            info!("--dry-run: scanning input only");
            let result = ingestion::ingest(config)?;
            print_dry_run_summary(&result);
            return Ok(ProcessingResult {
                tile_count: 0,
                duration: start.elapsed(),
            });
        }

        // Full pipeline
        info!("Stage 1/4: Ingestion");
        let ingestion_result = ingestion::ingest(config)?;

        info!("Stage 2/4: Transform");
        Self::transform(config, &ingestion_result)?;

        info!("Stage 3/4: Tiling");
        let tile_count = Self::tile(config)?;

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

    fn transform(_config: &PipelineConfig, _ingestion: &IngestionResult) -> Result<()> {
        todo!("Milestone 3: transform stage")
    }

    fn tile(_config: &PipelineConfig) -> Result<usize> {
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

/// Print dry-run summary with mesh stats and georeferencing.
fn print_dry_run_summary(result: &IngestionResult) {
    let stats = &result.stats;
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
    print_georef(result);
}
