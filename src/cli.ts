#!/usr/bin/env node
/**
 * Photo-Tiler CLI
 *
 * Command-line interface for converting photogrammetry meshes to 3D Tiles 1.1.
 *
 * Supports input from:
 * 1. OBJ files (Pix4D, Agisoft, RealityCapture, DJI Terra exports)
 * 2. glTF/GLB files
 * 3. PLY files (future)
 */

import * as fs from 'fs/promises';
import * as path from 'path';
import { runPipeline, createDefaultConfig, createObjConfig } from './pipeline.js';
import { createLogger } from './utils/logger.js';
import type { Georeference, PipelineConfig, LinearUnit } from './types.js';

const logger = createLogger('cli');

interface CLIOptions {
  input: string;
  output: string;
  epsg?: number;
  easting?: number;
  northing?: number;
  elevation?: number;
  trueNorth?: number;
  maxTriangles?: number;
  maxFeatures?: number;
  maxDepth?: number;
  noDraco?: boolean;
  noTextures?: boolean;
  textureFormat?: 'ktx2' | 'webp' | 'original';
  textureQuality?: number;
  textureMaxSize?: number;
  verbose?: boolean;
  help?: boolean;
  // Unit options
  units?: LinearUnit;
}

const HELP_TEXT = `
Photo-Tiler - Convert photogrammetry meshes to 3D Tiles 1.1

Usage:
  photo-tiler -i <input> -o <output> [options]

Required:
  -i, --input <path>      Input file (OBJ, glTF, GLB, or PLY)
  -o, --output <dir>      Output directory for tileset

Input Options:
  --units <unit>          Input units: mm, cm, m, ft, in
                          Required for OBJ files, optional override for others

Georeferencing:
  --epsg <code>           EPSG code (e.g., 32636 for UTM Zone 36N)
  --easting <meters>      Origin easting in projected coordinates
  --northing <meters>     Origin northing in projected coordinates
  --elevation <meters>    Origin elevation (default: 0)
  --true-north <degrees>  True north rotation from grid north (default: 0)

Tiling Options:
  --max-triangles <n>     Max triangles per tile (default: 100000)
  --max-features <n>      Max features per tile (default: 500)
  --max-depth <n>         Max hierarchy depth (default: 6)
  --no-draco              Disable Draco compression
  --no-textures           Exclude textures from output
  --texture-format <fmt>  Texture compression format: webp (default), ktx2, original
  --texture-quality <n>   Texture quality 0-100 for WebP (default: 85)
  --texture-max-size <n>  Max texture dimension in pixels (default: 2048)

Other:
  -v, --verbose           Enable verbose logging
  -h, --help              Show this help message

Output:
  3D Tiles output is always in meters and ECEF coordinate system.
  Input units are auto-detected when possible, or use --units to specify.
  Textures are included by default (use --no-textures to exclude).

Examples:
  # Convert a glTF file (meters assumed)
  photo-tiler -i model.gltf -o ./output

  # Convert an OBJ file (units required)
  photo-tiler -i model.obj -o ./output --units mm

  # Convert with georeferencing (UTM Zone 36N)
  photo-tiler -i model.glb -o ./output \\
    --epsg 32636 \\
    --easting 500000 \\
    --northing 4000000 \\
    --elevation 100 \\
    --true-north 2.5

  # Convert with KTX2 texture compression
  photo-tiler -i model.obj -o ./output --units m --texture-format ktx2

  # Convert without textures
  photo-tiler -i model.obj -o ./output --units m --no-textures
`;

/**
 * Parse command line arguments
 */
function parseArgs(args: string[]): CLIOptions {
  const options: CLIOptions = {
    input: '',
    output: '',
  };

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    const next = args[i + 1];

    switch (arg) {
      case '-i':
      case '--input':
        options.input = next;
        i++;
        break;
      case '-o':
      case '--output':
        options.output = next;
        i++;
        break;
      case '--units':
        options.units = next as LinearUnit;
        i++;
        break;
      case '--epsg':
        options.epsg = parseInt(next, 10);
        i++;
        break;
      case '--easting':
        options.easting = parseFloat(next);
        i++;
        break;
      case '--northing':
        options.northing = parseFloat(next);
        i++;
        break;
      case '--elevation':
        options.elevation = parseFloat(next);
        i++;
        break;
      case '--true-north':
        options.trueNorth = parseFloat(next);
        i++;
        break;
      case '--max-triangles':
        options.maxTriangles = parseInt(next, 10);
        i++;
        break;
      case '--max-features':
        options.maxFeatures = parseInt(next, 10);
        i++;
        break;
      case '--max-depth':
        options.maxDepth = parseInt(next, 10);
        i++;
        break;
      case '--no-draco':
        options.noDraco = true;
        break;
      case '--no-textures':
        options.noTextures = true;
        break;
      case '--texture-format':
        options.textureFormat = next as 'ktx2' | 'webp' | 'original';
        i++;
        break;
      case '--texture-quality':
        options.textureQuality = parseInt(next, 10);
        i++;
        break;
      case '--texture-max-size':
        options.textureMaxSize = parseInt(next, 10);
        i++;
        break;
      case '-v':
      case '--verbose':
        options.verbose = true;
        break;
      case '-h':
      case '--help':
        options.help = true;
        break;
    }
  }

  return options;
}

/**
 * Validate unit option
 */
function validateUnits(units: string | undefined): LinearUnit | undefined {
  if (!units) return undefined;

  const validUnits: LinearUnit[] = ['mm', 'cm', 'm', 'ft', 'in'];
  if (!validUnits.includes(units as LinearUnit)) {
    console.error(`Error: Invalid units "${units}". Valid options: ${validUnits.join(', ')}`);
    process.exit(1);
  }
  return units as LinearUnit;
}

/**
 * Build georeference from CLI options
 */
function buildGeoreference(options: CLIOptions): Georeference | undefined {
  if (!options.epsg) {
    return undefined;
  }

  return {
    epsg: options.epsg,
    origin: {
      easting: options.easting || 0,
      northing: options.northing || 0,
      elevation: options.elevation || 0,
    },
    trueNorthRotation: options.trueNorth || 0,
    heightReference: 'ellipsoidal',
  };
}

/**
 * Build texture compression options from CLI options
 */
function buildTextureCompression(options: CLIOptions): PipelineConfig['output']['textureCompression'] {
  if (options.noTextures) {
    return undefined;
  }

  const compression: PipelineConfig['output']['textureCompression'] = {
    enabled: true,
    format: options.textureFormat || 'webp',
  };

  if (options.textureQuality !== undefined) {
    compression.quality = options.textureQuality;
  }

  if (options.textureMaxSize !== undefined) {
    compression.maxSize = options.textureMaxSize;
  }

  return compression;
}

/**
 * Detect input type from file extension
 */
function detectInputType(inputPath: string): 'gltf' | 'obj' | 'ply' {
  const ext = path.extname(inputPath).toLowerCase();

  switch (ext) {
    case '.gltf':
    case '.glb':
      return 'gltf';
    case '.obj':
      return 'obj';
    case '.ply':
      return 'ply';
    default:
      console.error(`Error: Unsupported file type: ${ext}`);
      console.error('Supported types: .gltf, .glb, .obj, .ply');
      process.exit(1);
  }
}

/**
 * Build pipeline config from CLI options
 */
function buildConfig(options: CLIOptions): PipelineConfig {
  const georeference = buildGeoreference(options);
  const textureCompression = buildTextureCompression(options);
  const units = validateUnits(options.units);

  if (!options.input) {
    console.error('Error: Input file is required');
    process.exit(1);
  }

  const inputType = detectInputType(options.input);

  // PLY not yet supported
  if (inputType === 'ply') {
    console.error('Error: PLY input is not yet supported (coming soon)');
    process.exit(1);
  }

  // OBJ requires units
  if (inputType === 'obj' && !units) {
    console.error('Error: Units are required for OBJ files.');
    console.error('Use --units mm|cm|m|ft|in to specify input units.');
    process.exit(1);
  }

  // Create config based on input type
  let config: PipelineConfig;

  if (inputType === 'obj') {
    config = createObjConfig(options.input, units!, options.output, georeference);
  } else {
    config = createDefaultConfig(options.input, options.output, georeference, units);
  }

  applyTilingOptions(config, options, textureCompression);
  return config;
}

/**
 * Apply tiling options to config
 */
function applyTilingOptions(
  config: PipelineConfig,
  options: CLIOptions,
  textureCompression?: PipelineConfig['output']['textureCompression']
): void {
  if (options.maxTriangles) {
    config.tiling.maxTrianglesPerTile = options.maxTriangles;
  }
  if (options.maxFeatures) {
    config.tiling.maxFeaturesPerTile = options.maxFeatures;
  }
  if (options.maxDepth) {
    config.tiling.maxDepth = options.maxDepth;
  }
  if (options.noDraco) {
    config.output.draco.enabled = false;
  }
  if (options.noTextures) {
    config.output.includeTextures = false;
  }
  if (textureCompression) {
    config.output.textureCompression = textureCompression;
  }
}

/**
 * Main CLI entry point
 */
async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const options = parseArgs(args);

  // Show help
  if (options.help || args.length === 0) {
    console.log(HELP_TEXT);
    process.exit(0);
  }

  // Validate required options
  if (!options.input) {
    console.error('Error: Input file is required');
    console.error('Use --help for usage information');
    process.exit(1);
  }

  if (!options.output) {
    console.error('Error: Output directory is required');
    console.error('Use --help for usage information');
    process.exit(1);
  }

  // Validate input file exists
  try {
    await fs.access(options.input);
  } catch {
    console.error(`Error: Input file not found: ${options.input}`);
    process.exit(1);
  }

  // Set log level
  if (options.verbose) {
    process.env.LOG_LEVEL = 'debug';
  }

  // Build config and run pipeline
  try {
    console.log('Photo-Tiler starting...');
    console.log(`Input: ${options.input}`);
    console.log(`Output: ${options.output}`);
    if (options.units) {
      console.log(`Units: ${options.units} (user specified)`);
    }

    const config = buildConfig(options);
    const result = await runPipeline(config);

    console.log('\n=== Conversion Complete ===');
    console.log(`Tileset: ${result.tilesetPath}`);
    console.log(`Tiles: ${result.tileCount}`);
    console.log(`Features: ${result.featureCount}`);
    console.log(`Triangles: ${result.triangleCount.toLocaleString()}`);
    console.log(`Size: ${(result.totalSizeBytes / 1024 / 1024).toFixed(2)} MB`);
    console.log(`Time: ${(result.processingTimeMs / 1000).toFixed(2)} seconds`);

    // Print unit information
    if (result.inputUnits) {
      console.log(`\n=== Coordinate Info ===`);
      console.log(
        `Input units: ${result.inputUnits.linearUnit} (${result.inputUnits.source}, ${result.inputUnits.confidence} confidence)`
      );
      console.log(`Output units: meters`);
      console.log(`Output coordinate system: ECEF`);
      if (result.outputInfo?.epsg) {
        console.log(`Reference EPSG: ${result.outputInfo.epsg}`);
      }
    }

    if (result.validation) {
      if (result.validation.valid) {
        console.log('\nValidation: PASSED');
      } else {
        console.log('\nValidation: FAILED');
        for (const error of result.validation.errors) {
          console.log(`  - ${error}`);
        }
      }
    }

    process.exit(0);
  } catch (error) {
    console.error('Error:', error instanceof Error ? error.message : error);
    if (options.verbose) {
      console.error(error);
    }
    process.exit(1);
  }
}

main();
