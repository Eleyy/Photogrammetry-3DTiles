/**
 * Ingestion Module
 *
 * Orchestrates loading geometry from multiple input types:
 * - glTF/GLB files (direct upload)
 * - OBJ files (direct upload with MTL/textures)
 * - PLY files (future)
 */

import * as fs from 'fs/promises';
import * as path from 'path';
import { createLogger } from '../utils/logger.js';
import { InputError } from '../utils/errors.js';
import type {
  Feature,
  BoundingBox,
  IngestionResult,
  InputType,
  MaterialLibrary,
  LinearUnit,
  DetectedUnits,
} from '../types.js';
import { mergeGeometry, unionBounds } from './feature.js';

// Re-export sub-modules
export { loadGltf, type GltfLoadResult } from './gltf-loader.js';
export { convertObjToGltf, type ObjToGltfOptions } from './obj-converter.js';
export {
  createEmptyFeature,
  computeBounds,
  computeGlobalBounds,
  mergeGeometry,
  unionBounds,
  totalTriangleCount,
  boundsDiagonal,
  boundsCenter,
  boundsHalfSize,
} from './feature.js';

// Re-export units module
export {
  UNIT_TO_METERS,
  UNIT_NAMES,
  parseUnitString,
  createUserSpecifiedUnits,
  getDefaultUnits,
  formatUnitsInfo,
} from './units.js';

const logger = createLogger('ingestion');

// Default unit scale (meters)
const DEFAULT_UNIT_SCALE = 1.0;

export interface IngestionOptions {
  /** Input type detection override */
  inputType?: InputType;

  /** Path to geometry file (glTF/GLB/OBJ) */
  geometryPath?: string;

  /** Unit scale to apply (overrides auto-detection) */
  unitScale?: number;

  /** Input units (overrides auto-detection) */
  units?: LinearUnit;

  /** Working directory for temporary files */
  workDir?: string;
}

/**
 * Detect input type from file extension or configuration
 */
function detectInputType(options: IngestionOptions): InputType {
  if (options.inputType) {
    return options.inputType;
  }

  if (options.geometryPath) {
    const ext = path.extname(options.geometryPath).toLowerCase();
    if (ext === '.gltf' || ext === '.glb') {
      return 'gltf';
    }
    if (ext === '.obj') {
      return 'obj';
    }
    if (ext === '.ply') {
      return 'ply';
    }
  }

  throw new InputError('Cannot detect input type from options', { options });
}

/**
 * Load and ingest a model from any supported input type
 */
export async function ingest(options: IngestionOptions): Promise<IngestionResult> {
  const inputType = detectInputType(options);

  logger.info({ inputType }, 'Starting ingestion');

  let features: Feature[];
  let globalBounds: BoundingBox;
  let sourcePath: string;
  let materials: MaterialLibrary | undefined;
  let detectedUnits: DetectedUnits | undefined;
  let unitScale = options.unitScale ?? DEFAULT_UNIT_SCALE;

  switch (inputType) {
    case 'gltf': {
      if (!options.geometryPath) {
        throw new InputError('geometryPath required for glTF input');
      }

      await validateFileExists(options.geometryPath);

      // glTF spec defines meters, so default scale is 1.0
      // But user can override if their glTF is in different units
      if (options.units) {
        const { UNIT_TO_METERS, createUserSpecifiedUnits } = await import('./units.js');
        unitScale = UNIT_TO_METERS[options.units];
        detectedUnits = createUserSpecifiedUnits(options.units);
      } else {
        const { getDefaultUnits } = await import('./units.js');
        detectedUnits = getDefaultUnits();
        detectedUnits.source = 'assumed';
        detectedUnits.confidence = 'high'; // glTF spec is meters
      }

      const { loadGltf } = await import('./gltf-loader.js');
      const result = await loadGltf(options.geometryPath, unitScale, true);
      features = result.features;
      globalBounds = result.globalBounds;
      materials = result.materials;
      sourcePath = options.geometryPath;
      break;
    }

    case 'obj': {
      if (!options.geometryPath) {
        throw new InputError('geometryPath required for OBJ input');
      }

      await validateFileExists(options.geometryPath);

      // OBJ files require explicit units - no auto-detection possible
      if (!options.units) {
        throw new InputError(
          'Units are required for OBJ input. Use --units mm|cm|m|ft|in'
        );
      }

      const { UNIT_TO_METERS, createUserSpecifiedUnits } = await import('./units.js');
      unitScale = UNIT_TO_METERS[options.units];
      detectedUnits = createUserSpecifiedUnits(options.units);

      // Convert OBJ to glTF first
      const { convertObjToGltf } = await import('./obj-converter.js');
      const workDir = options.workDir || path.join(process.cwd(), '.photo-tiler-temp');
      await fs.mkdir(workDir, { recursive: true });

      const gltfPath = path.join(workDir, 'converted.glb');
      await convertObjToGltf(options.geometryPath, gltfPath, {
        binary: true,
        secure: true,
      });

      // Load the converted glTF
      const { loadGltf } = await import('./gltf-loader.js');
      const result = await loadGltf(gltfPath, unitScale, true);
      features = result.features;
      globalBounds = result.globalBounds;
      materials = result.materials;
      sourcePath = options.geometryPath;
      break;
    }

    case 'ply': {
      throw new InputError('PLY input is not yet supported (coming soon)');
    }

    default:
      throw new InputError(`Unsupported input type: ${inputType}`);
  }

  const triangleCount = features.reduce((sum, f) => sum + f.triangleCount, 0);

  // Log summary with unit information
  logger.info(
    {
      inputType,
      featureCount: features.length,
      triangleCount,
      globalBounds,
      hasMaterials: !!materials,
      inputUnits: detectedUnits?.linearUnit,
      unitSource: detectedUnits?.source,
      outputUnits: 'meters',
    },
    'Ingestion complete'
  );

  return {
    features,
    globalBounds,
    triangleCount,
    sourcePath,
    unitScale,
    materials,
    detectedUnits,
  };
}

/**
 * Validate that a file exists and is readable
 */
async function validateFileExists(filePath: string): Promise<void> {
  try {
    await fs.access(filePath, fs.constants.R_OK);
  } catch {
    throw new InputError(`File not found or not readable: ${filePath}`);
  }
}

/**
 * Get statistics about ingested features
 */
export function getIngestionStats(result: IngestionResult): {
  featureCount: number;
  triangleCount: number;
  vertexCount: number;
  boundsSize: [number, number, number];
  inputUnits?: string;
  outputUnits: string;
} {
  let vertexCount = 0;

  for (const feature of result.features) {
    vertexCount += feature.vertexCount;
  }

  const boundsSize: [number, number, number] = [
    result.globalBounds.max[0] - result.globalBounds.min[0],
    result.globalBounds.max[1] - result.globalBounds.min[1],
    result.globalBounds.max[2] - result.globalBounds.min[2],
  ];

  return {
    featureCount: result.features.length,
    triangleCount: result.triangleCount,
    vertexCount,
    boundsSize,
    inputUnits: result.detectedUnits?.linearUnit,
    outputUnits: 'meters',
  };
}

/**
 * Merge features with the same ID (useful for multi-part elements)
 */
export function mergeFeaturesByID(features: Feature[]): Feature[] {
  const featureMap = new Map<string, Feature>();

  for (const feature of features) {
    const existing = featureMap.get(feature.id);

    if (existing) {
      // Merge geometry
      mergeGeometry(
        existing,
        feature.positions,
        feature.normals,
        feature.uvs,
        feature.indices,
        feature.colors
      );
    } else {
      // Clone the feature to avoid mutation issues
      const cloned: Feature = {
        ...feature,
        positions: new Float32Array(feature.positions),
        normals: feature.normals ? new Float32Array(feature.normals) : null,
        uvs: feature.uvs ? new Float32Array(feature.uvs) : null,
        colors: feature.colors ? new Float32Array(feature.colors) : null,
        indices: new Uint32Array(feature.indices),
        bounds: {
          min: [...feature.bounds.min] as [number, number, number],
          max: [...feature.bounds.max] as [number, number, number],
        },
        properties: { ...feature.properties },
      };
      featureMap.set(feature.id, cloned);
    }
  }

  return Array.from(featureMap.values());
}

/**
 * Estimate memory usage for features
 */
export function estimateMemoryUsage(features: Feature[]): number {
  let bytes = 0;

  for (const feature of features) {
    // Float32Array: 4 bytes per element
    bytes += feature.positions.byteLength;
    if (feature.normals) bytes += feature.normals.byteLength;
    if (feature.uvs) bytes += feature.uvs.byteLength;
    if (feature.colors) bytes += feature.colors.byteLength;

    // Uint32Array: 4 bytes per element
    bytes += feature.indices.byteLength;

    // Rough estimate for object overhead and properties
    bytes += 1000;
  }

  return bytes;
}
