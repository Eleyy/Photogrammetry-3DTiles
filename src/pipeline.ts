/**
 * Photo-Tiler Pipeline
 *
 * Main orchestration for converting photogrammetry meshes to 3D Tiles 1.1.
 *
 * Pipeline stages:
 * 1. Ingest: Load geometry from various sources
 * 2. Transform: Apply coordinate transformations (unit, axis, georeference)
 * 3. Tile: Build spatial hierarchy and generate GLB tiles
 * 4. Output: Write tileset.json and tile content
 *
 * Supported input types:
 * - glTF/GLB (direct file)
 * - OBJ (direct file, requires units)
 * - PLY (future)
 */

import * as fs from 'fs/promises';
import * as path from 'path';
import { createLogger } from './utils/logger.js';
import { ValidationError } from './utils/errors.js';
import type {
  PipelineConfig,
  ProcessingResult,
  Georeference,
  LinearUnit,
} from './types.js';

const logger = createLogger('pipeline');

/**
 * Run the full photogrammetry-to-3DTiles conversion pipeline
 */
export async function runPipeline(config: PipelineConfig): Promise<ProcessingResult> {
  const startTime = Date.now();

  logger.info({ inputType: config.input.type }, 'Starting Photo-Tiler pipeline');

  // Validate configuration
  validateConfig(config);

  // Ensure output directory exists
  await fs.mkdir(config.output.outputDir, { recursive: true });

  // Stage 1: Ingest
  logger.info('Stage 1: Ingesting source data');
  const { ingest, getIngestionStats } = await import('./ingestion/index.js');

  const ingestionResult = await ingest({
    inputType: config.input.type,
    geometryPath: config.input.geometryPath,
    units: config.input.units,
    workDir: path.join(config.output.outputDir, '.temp'),
  });

  const stats = getIngestionStats(ingestionResult);
  logger.info(
    {
      featureCount: stats.featureCount,
      triangleCount: stats.triangleCount,
      inputUnits: ingestionResult.detectedUnits?.linearUnit,
    },
    'Ingestion complete'
  );

  // Stage 2: Transform
  logger.info('Stage 2: Applying coordinate transforms');
  const {
    computeTransforms,
    transformFeatures,
    transformBounds,
    centerGeometryAtOrigin,
    applyPreTranslationToRootTransform,
  } = await import('./transform/index.js');

  // Use detected unit scale to convert to meters
  const unitScale = ingestionResult.detectedUnits?.toMeters ?? 1.0;

  const transforms = computeTransforms({
    georeference: config.georeference,
    unitScale: unitScale,
    convertYUpToZUp: true,
  });

  logger.info(
    {
      summary: transforms.summary,
      inputUnits: ingestionResult.detectedUnits?.linearUnit,
      unitScale,
    },
    'Computed transforms'
  );

  // Apply local transform to features
  transformFeatures(ingestionResult.features, transforms.localTransform);

  // Transform global bounds
  const transformedBounds = transformBounds(
    ingestionResult.globalBounds,
    transforms.localTransform
  );

  // Center geometry at origin for proper 3D Tiles placement
  // Cesium best practice: tile content should be centered at local origin,
  // with the root transform positioning the tileset in ECEF.
  const centerOffset = centerGeometryAtOrigin(ingestionResult.features, transformedBounds);

  // Apply center offset to root transform so geometry ends up in correct ECEF position
  const finalRootTransform = applyPreTranslationToRootTransform(
    transforms.rootTransform,
    centerOffset
  );

  // Stage 3: Tile
  logger.info('Stage 3: Building tile hierarchy and generating content');
  const { generateTilesetWithProgress } = await import('./tiling/index.js');

  const tilingResult = await generateTilesetWithProgress(
    ingestionResult.features,
    transformedBounds,
    config.output.outputDir,
    {
      tiling: config.tiling,
      draco: config.output.draco,
      rootTransform: finalRootTransform,
      tilesetVersion: '1.0.0',
      materials: config.output.includeTextures ? ingestionResult.materials : undefined,
      textureCompression: config.output.includeTextures
        ? config.output.textureCompression
        : undefined,
      onProgress: (progress) => {
        logger.debug(progress, 'Tiling progress');
      },
    }
  );

  // Stage 4: Validate (optional)
  let validation: ProcessingResult['validation'];

  if (config.output.validate) {
    logger.info('Stage 4: Validating output');
    const { validateTileset } = await import('./tiling/tileset-writer.js');

    const tilesetJson = JSON.parse(
      await fs.readFile(tilingResult.tilesetPath, 'utf-8')
    );

    const validationResult = validateTileset(tilesetJson);
    validation = {
      valid: validationResult.valid,
      errors: validationResult.errors,
      warnings: validationResult.warnings,
    };

    if (!validationResult.valid) {
      logger.warn({ errors: validationResult.errors }, 'Validation failed');
    } else {
      logger.info('Validation passed');
    }
  }

  // Calculate final statistics
  const processingTimeMs = Date.now() - startTime;
  const totalSizeBytes = await calculateOutputSize(config.output.outputDir);

  const result: ProcessingResult = {
    tilesetPath: tilingResult.tilesetPath,
    tileCount: tilingResult.tileCount,
    featureCount: stats.featureCount,
    triangleCount: stats.triangleCount,
    totalSizeBytes,
    processingTimeMs,
    validation,
    inputUnits: ingestionResult.detectedUnits,
    outputInfo: {
      units: 'meters',
      coordinateSystem: 'ECEF',
      epsg: config.georeference?.epsg,
    },
  };

  logger.info(
    {
      tilesetPath: result.tilesetPath,
      tileCount: result.tileCount,
      featureCount: result.featureCount,
      triangleCount: result.triangleCount,
      totalSizeMB: (totalSizeBytes / 1024 / 1024).toFixed(2),
      processingTimeSec: (processingTimeMs / 1000).toFixed(2),
      inputUnits: result.inputUnits?.linearUnit,
      outputUnits: 'meters',
    },
    'Pipeline complete'
  );

  return result;
}

/**
 * Validate pipeline configuration
 */
function validateConfig(config: PipelineConfig): void {
  const errors: string[] = [];

  // Validate input
  if (!config.input.type) {
    errors.push('Input type is required');
  }

  if (config.input.type === 'gltf') {
    if (!config.input.geometryPath) {
      errors.push('Geometry path is required for glTF input');
    }
  }

  if (config.input.type === 'obj') {
    if (!config.input.geometryPath) {
      errors.push('Geometry path is required for OBJ input');
    }
    if (!config.input.units) {
      errors.push('Units are required for OBJ input (mm, cm, m, ft, in)');
    }
  }

  // Validate output
  if (!config.output.outputDir) {
    errors.push('Output directory is required');
  }

  // Validate georeference if provided
  if (config.georeference) {
    if (!config.georeference.epsg) {
      errors.push('EPSG code is required for georeferencing');
    }
    if (
      config.georeference.origin.easting === undefined ||
      config.georeference.origin.northing === undefined
    ) {
      errors.push('Origin easting and northing are required for georeferencing');
    }
  }

  if (errors.length > 0) {
    throw new ValidationError('Invalid pipeline configuration', { errors });
  }
}

/**
 * Calculate total output size
 */
async function calculateOutputSize(outputDir: string): Promise<number> {
  let totalSize = 0;

  async function walkDir(dir: string): Promise<void> {
    const entries = await fs.readdir(dir, { withFileTypes: true });

    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);

      if (entry.isDirectory()) {
        // Skip temp directories
        if (entry.name.startsWith('.')) continue;
        await walkDir(fullPath);
      } else {
        const stat = await fs.stat(fullPath);
        totalSize += stat.size;
      }
    }
  }

  await walkDir(outputDir);
  return totalSize;
}

/**
 * Create a default pipeline configuration for glTF
 */
export function createDefaultConfig(
  inputPath: string,
  outputDir: string,
  georeference?: Georeference,
  units?: LinearUnit
): PipelineConfig {
  const ext = path.extname(inputPath).toLowerCase();

  if (ext !== '.gltf' && ext !== '.glb') {
    throw new ValidationError(`Unsupported file type: ${ext}. Use createObjConfig for OBJ files.`);
  }

  return {
    input: {
      type: 'gltf',
      geometryPath: inputPath,
      ...(units && { units }),
    },
    georeference,
    tiling: {
      maxDepth: 6,
      maxFeaturesPerTile: 500,
      maxTrianglesPerTile: 100000,
      minTileSize: 1.0,
      geometricErrorDecay: 0.5,
    },
    output: {
      outputDir,
      draco: {
        enabled: true,
        compressionLevel: 7,
        quantizePositionBits: 14,
        quantizeNormalBits: 10,
        quantizeTexcoordBits: 12,
      },
      validate: true,
      includeTextures: true,
    },
  };
}

/**
 * Create configuration for OBJ input (requires units)
 */
export function createObjConfig(
  objPath: string,
  units: LinearUnit,
  outputDir: string,
  georeference?: Georeference
): PipelineConfig {
  return {
    input: {
      type: 'obj',
      geometryPath: objPath,
      units,
    },
    georeference,
    tiling: {
      maxDepth: 6,
      maxFeaturesPerTile: 500,
      maxTrianglesPerTile: 100000,
      minTileSize: 1.0,
      geometricErrorDecay: 0.5,
    },
    output: {
      outputDir,
      draco: {
        enabled: true,
        compressionLevel: 7,
        quantizePositionBits: 14,
        quantizeNormalBits: 10,
        quantizeTexcoordBits: 12,
      },
      validate: true,
      includeTextures: true,
    },
  };
}

/**
 * Quick convert function for simple use cases
 */
export async function convert(
  inputPath: string,
  outputDir: string,
  options?: {
    georeference?: Georeference;
    maxTrianglesPerTile?: number;
    maxFeaturesPerTile?: number;
    draco?: boolean;
    units?: LinearUnit;
  }
): Promise<ProcessingResult> {
  const ext = path.extname(inputPath).toLowerCase();

  let config: PipelineConfig;

  if (ext === '.obj') {
    if (!options?.units) {
      throw new ValidationError('Units are required for OBJ files. Specify units in options.');
    }
    config = createObjConfig(inputPath, options.units, outputDir, options?.georeference);
  } else {
    config = createDefaultConfig(inputPath, outputDir, options?.georeference, options?.units);
  }

  if (options?.maxTrianglesPerTile) {
    config.tiling.maxTrianglesPerTile = options.maxTrianglesPerTile;
  }

  if (options?.maxFeaturesPerTile) {
    config.tiling.maxFeaturesPerTile = options.maxFeaturesPerTile;
  }

  if (options?.draco === false) {
    config.output.draco.enabled = false;
  }

  return runPipeline(config);
}
