use anyhow::Context;
use clap::Parser;
use tracing::error;
use tracing_subscriber::EnvFilter;

use photo_tiler::config::{CliArgs, PipelineConfig};
use photo_tiler::pipeline::Pipeline;

fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();

    // Init tracing
    let filter = if args.verbose {
        EnvFilter::new("photo_tiler=debug")
    } else {
        EnvFilter::new("photo_tiler=info")
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let config: PipelineConfig = args.into();

    // Configure rayon thread pool
    if let Some(threads) = config.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .context("Failed to configure rayon thread pool")?;
    }

    match Pipeline::run(&config) {
        Ok(result) => {
            println!(
                "Done: {} tiles generated in {:.2}s",
                result.tile_count,
                result.duration.as_secs_f64()
            );
            Ok(())
        }
        Err(e) => {
            error!(%e, "Pipeline failed");
            Err(anyhow::anyhow!(e)).context("photo-tiler pipeline failed")
        }
    }
}
