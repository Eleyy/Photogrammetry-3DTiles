use std::time::{Duration, Instant};

use tracing::info;

use crate::config::PipelineConfig;
use crate::error::Result;

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
            // TODO: detect and print georef, then return
            info!("Georeferencing detection not yet implemented");
            return Ok(ProcessingResult {
                tile_count: 0,
                duration: start.elapsed(),
            });
        }

        if config.dry_run {
            info!("--dry-run: scanning input only");
            Self::ingest(config)?;
            info!("Dry-run complete");
            return Ok(ProcessingResult {
                tile_count: 0,
                duration: start.elapsed(),
            });
        }

        // Full pipeline
        info!("Stage 1/4: Ingestion");
        Self::ingest(config)?;

        info!("Stage 2/4: Transform");
        Self::transform(config)?;

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

    fn ingest(_config: &PipelineConfig) -> Result<()> {
        todo!("Milestone 2: ingestion stage")
    }

    fn transform(_config: &PipelineConfig) -> Result<()> {
        todo!("Milestone 3: transform stage")
    }

    fn tile(_config: &PipelineConfig) -> Result<usize> {
        todo!("Milestone 4: tiling stage")
    }

    fn validate(_config: &PipelineConfig) -> Result<()> {
        todo!("Milestone 5: validation stage")
    }
}
