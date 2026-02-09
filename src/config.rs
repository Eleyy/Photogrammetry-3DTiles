use std::path::PathBuf;

use clap::Parser;

/// Input coordinate units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Units {
    #[value(name = "mm")]
    Millimeters,
    #[value(name = "cm")]
    Centimeters,
    #[value(name = "m")]
    Meters,
    #[value(name = "ft")]
    Feet,
    #[value(name = "in")]
    Inches,
}

impl std::fmt::Display for Units {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Units::Millimeters => write!(f, "mm"),
            Units::Centimeters => write!(f, "cm"),
            Units::Meters => write!(f, "m"),
            Units::Feet => write!(f, "ft"),
            Units::Inches => write!(f, "in"),
        }
    }
}

/// Output texture format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TextureFormat {
    #[value(name = "webp")]
    WebP,
    #[value(name = "ktx2")]
    Ktx2,
    #[value(name = "original")]
    Original,
}

impl std::fmt::Display for TextureFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextureFormat::WebP => write!(f, "webp"),
            TextureFormat::Ktx2 => write!(f, "ktx2"),
            TextureFormat::Original => write!(f, "original"),
        }
    }
}

/// Georeferencing parameters.
#[derive(Debug, Clone, Default)]
pub struct Georeference {
    pub epsg: u32,
    pub easting: f64,
    pub northing: f64,
    pub elevation: f64,
    pub true_north: f64,
}

/// Tiling parameters.
#[derive(Debug, Clone)]
pub struct TilingConfig {
    pub max_triangles_per_tile: usize,
    pub max_depth: u32,
}

impl Default for TilingConfig {
    fn default() -> Self {
        Self {
            max_triangles_per_tile: 65_000,
            max_depth: 6,
        }
    }
}

/// Texture processing parameters.
#[derive(Debug, Clone)]
pub struct TextureConfig {
    pub format: TextureFormat,
    pub quality: u8,
    pub max_size: u32,
    pub enabled: bool,
}

impl Default for TextureConfig {
    fn default() -> Self {
        Self {
            format: TextureFormat::WebP,
            quality: 85,
            max_size: 2048,
            enabled: true,
        }
    }
}

/// Draco compression parameters.
#[derive(Debug, Clone)]
pub struct DracoConfig {
    pub enabled: bool,
    pub level: u8,
}

impl Default for DracoConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: 7,
        }
    }
}

/// Fully resolved pipeline configuration (constructed from CLI args).
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub input: PathBuf,
    pub output: PathBuf,
    pub units: Option<Units>,
    pub georeference: Option<Georeference>,
    pub offset_file: Option<PathBuf>,
    pub metadata_xml: Option<PathBuf>,
    pub tiling: TilingConfig,
    pub texture: TextureConfig,
    pub draco: DracoConfig,
    pub validate: bool,
    pub dry_run: bool,
    pub show_georef: bool,
    pub verbose: bool,
    pub threads: Option<usize>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            output: PathBuf::new(),
            units: None,
            georeference: None,
            offset_file: None,
            metadata_xml: None,
            tiling: TilingConfig::default(),
            texture: TextureConfig::default(),
            draco: DracoConfig::default(),
            validate: false,
            dry_run: false,
            show_georef: false,
            verbose: false,
            threads: None,
        }
    }
}

/// CLI argument definition (clap derive).
#[derive(Parser, Debug)]
#[command(
    name = "photo-tiler",
    about = "Photogrammetry mesh to OGC 3D Tiles 1.1 converter",
    version
)]
pub struct CliArgs {
    /// Input file (OBJ, glTF, GLB, PLY)
    #[arg(short = 'i', long)]
    pub input: PathBuf,

    /// Output directory
    #[arg(short = 'o', long)]
    pub output: PathBuf,

    /// Input coordinate units
    #[arg(long, value_enum)]
    pub units: Option<Units>,

    /// EPSG code (e.g. 32636)
    #[arg(long)]
    pub epsg: Option<u32>,

    /// Origin easting in metres
    #[arg(long)]
    pub easting: Option<f64>,

    /// Origin northing in metres
    #[arg(long)]
    pub northing: Option<f64>,

    /// Origin elevation in metres
    #[arg(long, default_value_t = 0.0)]
    pub elevation: f64,

    /// True north rotation in degrees
    #[arg(long, default_value_t = 0.0)]
    pub true_north: f64,

    /// Path to offset.xyz file
    #[arg(long)]
    pub offset_file: Option<PathBuf>,

    /// Path to metadata.xml file
    #[arg(long)]
    pub metadata_xml: Option<PathBuf>,

    /// Display detected georeferencing and exit
    #[arg(long)]
    pub show_georef: bool,

    /// Scan input and report stats only
    #[arg(long)]
    pub dry_run: bool,

    /// Max triangles per leaf tile
    #[arg(long, default_value_t = 65_000)]
    pub max_triangles: usize,

    /// Max octree depth
    #[arg(long, default_value_t = 6)]
    pub max_depth: u32,

    /// Disable Draco mesh compression
    #[arg(long)]
    pub no_draco: bool,

    /// Draco compression level (1-10)
    #[arg(long, default_value_t = 7)]
    pub draco_level: u8,

    /// Exclude textures from output
    #[arg(long)]
    pub no_textures: bool,

    /// Texture format: webp, ktx2, or original
    #[arg(long, value_enum, default_value = "webp")]
    pub texture_format: TextureFormat,

    /// Texture compression quality (0-100)
    #[arg(long, default_value_t = 85)]
    pub texture_quality: u8,

    /// Max texture dimension in pixels
    #[arg(long, default_value_t = 2048)]
    pub texture_max_size: u32,

    /// Run tileset validation after conversion
    #[arg(long)]
    pub validate: bool,

    /// Enable verbose logging
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Worker thread count (default: all cores)
    #[arg(short = 'j', long)]
    pub threads: Option<usize>,
}

impl From<CliArgs> for PipelineConfig {
    fn from(args: CliArgs) -> Self {
        let georeference = args.epsg.map(|epsg| Georeference {
            epsg,
            easting: args.easting.unwrap_or(0.0),
            northing: args.northing.unwrap_or(0.0),
            elevation: args.elevation,
            true_north: args.true_north,
        });

        PipelineConfig {
            input: args.input,
            output: args.output,
            units: args.units,
            georeference,
            offset_file: args.offset_file,
            metadata_xml: args.metadata_xml,
            tiling: TilingConfig {
                max_triangles_per_tile: args.max_triangles,
                max_depth: args.max_depth,
            },
            texture: TextureConfig {
                format: args.texture_format,
                quality: args.texture_quality,
                max_size: args.texture_max_size,
                enabled: !args.no_textures,
            },
            draco: DracoConfig {
                enabled: !args.no_draco,
                level: args.draco_level,
            },
            validate: args.validate,
            dry_run: args.dry_run,
            show_georef: args.show_georef,
            verbose: args.verbose,
            threads: args.threads,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tiling_config() {
        let tc = TilingConfig::default();
        assert_eq!(tc.max_triangles_per_tile, 65_000);
        assert_eq!(tc.max_depth, 6);
    }

    #[test]
    fn default_texture_config() {
        let tc = TextureConfig::default();
        assert_eq!(tc.format, TextureFormat::WebP);
        assert_eq!(tc.quality, 85);
        assert_eq!(tc.max_size, 2048);
        assert!(tc.enabled);
    }

    #[test]
    fn default_draco_config() {
        let dc = DracoConfig::default();
        assert!(dc.enabled);
        assert_eq!(dc.level, 7);
    }

    #[test]
    fn units_display() {
        assert_eq!(Units::Millimeters.to_string(), "mm");
        assert_eq!(Units::Centimeters.to_string(), "cm");
        assert_eq!(Units::Meters.to_string(), "m");
        assert_eq!(Units::Feet.to_string(), "ft");
        assert_eq!(Units::Inches.to_string(), "in");
    }

    #[test]
    fn texture_format_display() {
        assert_eq!(TextureFormat::WebP.to_string(), "webp");
        assert_eq!(TextureFormat::Ktx2.to_string(), "ktx2");
        assert_eq!(TextureFormat::Original.to_string(), "original");
    }

    #[test]
    fn cli_args_to_pipeline_config() {
        let args = CliArgs::parse_from([
            "photo-tiler",
            "-i",
            "model.obj",
            "-o",
            "./out",
            "--units",
            "m",
            "--epsg",
            "32636",
            "--easting",
            "500000",
            "--northing",
            "2800000",
            "--max-triangles",
            "50000",
            "--max-depth",
            "4",
            "--no-draco",
            "--no-textures",
            "--validate",
            "--dry-run",
            "-v",
            "-j",
            "8",
        ]);

        let config: PipelineConfig = args.into();

        assert_eq!(config.input, PathBuf::from("model.obj"));
        assert_eq!(config.output, PathBuf::from("./out"));
        assert_eq!(config.units, Some(Units::Meters));
        assert!(config.georeference.is_some());
        let geo = config.georeference.unwrap();
        assert_eq!(geo.epsg, 32636);
        assert!((geo.easting - 500_000.0).abs() < f64::EPSILON);
        assert!((geo.northing - 2_800_000.0).abs() < f64::EPSILON);
        assert_eq!(config.tiling.max_triangles_per_tile, 50_000);
        assert_eq!(config.tiling.max_depth, 4);
        assert!(!config.draco.enabled);
        assert!(!config.texture.enabled);
        assert!(config.validate);
        assert!(config.dry_run);
        assert!(config.verbose);
        assert_eq!(config.threads, Some(8));
    }

    #[test]
    fn cli_args_minimal() {
        let args = CliArgs::parse_from(["photo-tiler", "-i", "test.glb", "-o", "output"]);
        let config: PipelineConfig = args.into();

        assert_eq!(config.input, PathBuf::from("test.glb"));
        assert_eq!(config.output, PathBuf::from("output"));
        assert_eq!(config.units, None);
        assert!(config.georeference.is_none());
        assert!(config.draco.enabled);
        assert!(config.texture.enabled);
        assert!(!config.validate);
        assert!(!config.dry_run);
        assert!(!config.verbose);
        assert_eq!(config.threads, None);
    }
}
