/**
 * Coordinate System Transformations
 *
 * Orchestrates the full transform pipeline from local coordinates
 * to ECEF for 3D Tiles output.
 *
 * Pipeline:
 * 1. Unit conversion (source units → meters)
 * 2. Axis conversion (Y-up → Z-up)
 * 3. True North rotation
 * 4. Projection to WGS84
 * 5. ECEF transform computation
 */

import { mat4 } from 'gl-matrix';
import { createLogger } from '../utils/logger.js';
import { TransformError } from '../utils/errors.js';
import type { Georeference, WGS84Position, BoundingBox, Feature } from '../types.js';
import {
  createIdentity,
  createScale,
  createYUpToZUp,
  createTranslation,
  multiplyChain,
  transformPoint,
  type Matrix4,
  type Vector3,
} from './matrix.js';
import { georeferenceToWGS84 } from './projection.js';
import { eastNorthUpToEcef, wgs84ToEcef } from './ecef.js';

const logger = createLogger('coordinates');

const FEET_TO_METERS = 0.3048;

export interface TransformConfig {
  /** Georeferencing data */
  georeference?: Georeference;

  /** Unit scale (e.g., 0.3048 for feet to meters) */
  unitScale?: number;

  /** Apply Y-up to Z-up conversion */
  convertYUpToZUp?: boolean;
}

export interface TransformResult {
  /** Transform to apply to geometry (in local space) */
  localTransform: Matrix4;

  /** Root transform for tileset.json (places in ECEF) */
  rootTransform: Matrix4;

  /** WGS84 position of the origin (for reference) */
  wgs84Origin?: WGS84Position;

  /** Summary of transforms applied */
  summary: string[];
}

/**
 * Compute the full transformation pipeline
 *
 * For 3D Tiles output, we transform geometry to Z-up (ENU convention) and
 * use ENU-to-ECEF as the root transform. GLB files will contain Z-up content.
 */
export function computeTransforms(config: TransformConfig): TransformResult {
  const summary: string[] = [];
  const transforms: Matrix4[] = [];

  // 1. Unit conversion (source units → meters)
  const unitScale = config.unitScale ?? FEET_TO_METERS;
  if (unitScale !== 1.0) {
    transforms.push(createScale(unitScale, unitScale, unitScale));
    summary.push(`Unit scale: ${unitScale} (source units → meters)`);
  }

  // 2. Axis conversion (Y-up → Z-up)
  // This converts glTF Y-up content to Z-up (ENU convention) for 3D Tiles
  if (config.convertYUpToZUp !== false) {
    transforms.push(createYUpToZUp());
    summary.push('Axis conversion: Y-up → Z-up (applied to geometry)');
  }

  // 3. True North rotation (if georeferenced)
  if (config.georeference && config.georeference.trueNorthRotation !== 0) {
    // Rotation around Z axis (up in ENU space after Y-to-Z conversion)
    const rotation = -config.georeference.trueNorthRotation;
    const m = mat4.create();
    const radians = (rotation * Math.PI) / 180;
    mat4.fromZRotation(m, radians);
    transforms.push(Array.from(m));
    summary.push(`True North rotation: ${config.georeference.trueNorthRotation}° (around Z-up axis)`);
  }

  // Combine all local transforms
  const localTransform = transforms.length > 0
    ? multiplyChain(...transforms)
    : createIdentity();

  // 4-5. Compute ECEF root transform (if georeferenced)
  let rootTransform = createIdentity();
  let wgs84Origin: WGS84Position | undefined;

  if (config.georeference) {
    try {
      // Convert origin to WGS84
      wgs84Origin = georeferenceToWGS84(config.georeference);
      summary.push(
        `Origin WGS84: (${wgs84Origin.longitude.toFixed(6)}°, ` +
        `${wgs84Origin.latitude.toFixed(6)}°, ${wgs84Origin.height.toFixed(2)}m)`
      );

      // Compute ENU-to-ECEF transform at the origin
      // Geometry is already in Z-up (ENU) space after localTransform
      rootTransform = eastNorthUpToEcef(wgs84Origin);

      const ecefOrigin = wgs84ToEcef(wgs84Origin);
      summary.push(
        `ECEF origin: (${ecefOrigin[0].toFixed(2)}, ` +
        `${ecefOrigin[1].toFixed(2)}, ${ecefOrigin[2].toFixed(2)})`
      );

      logger.info(
        {
          georeference: config.georeference,
          wgs84Origin,
          ecefOrigin,
        },
        'Computed georeference transforms'
      );
    } catch (error) {
      throw new TransformError(
        'Failed to compute georeference transforms',
        { georeference: config.georeference, error }
      );
    }
  } else {
    // No georeferencing - geometry is in Z-up local coordinates
    summary.push('No georeferencing - using local Z-up coordinates');
  }

  return {
    localTransform,
    rootTransform,
    wgs84Origin,
    summary,
  };
}

/**
 * Apply local transform to a feature's geometry
 */
export function transformFeatureGeometry(
  feature: Feature,
  transform: Matrix4
): void {
  const { positions, normals, bounds } = feature;
  const vertexCount = positions.length / 3;

  // Transform positions
  for (let i = 0; i < vertexCount; i++) {
    const x = positions[i * 3];
    const y = positions[i * 3 + 1];
    const z = positions[i * 3 + 2];

    const transformed = transformPoint(transform, [x, y, z]);

    positions[i * 3] = transformed[0];
    positions[i * 3 + 1] = transformed[1];
    positions[i * 3 + 2] = transformed[2];
  }

  // Transform normals (direction only, no translation)
  if (normals) {
    for (let i = 0; i < vertexCount; i++) {
      const nx = normals[i * 3];
      const ny = normals[i * 3 + 1];
      const nz = normals[i * 3 + 2];

      const tnx = transform[0] * nx + transform[4] * ny + transform[8] * nz;
      const tny = transform[1] * nx + transform[5] * ny + transform[9] * nz;
      const tnz = transform[2] * nx + transform[6] * ny + transform[10] * nz;

      const len = Math.sqrt(tnx * tnx + tny * tny + tnz * tnz);
      if (len > 0) {
        normals[i * 3] = tnx / len;
        normals[i * 3 + 1] = tny / len;
        normals[i * 3 + 2] = tnz / len;
      }
    }
  }

  // Update bounds
  const corners: Vector3[] = [
    [bounds.min[0], bounds.min[1], bounds.min[2]],
    [bounds.min[0], bounds.min[1], bounds.max[2]],
    [bounds.min[0], bounds.max[1], bounds.min[2]],
    [bounds.min[0], bounds.max[1], bounds.max[2]],
    [bounds.max[0], bounds.min[1], bounds.min[2]],
    [bounds.max[0], bounds.min[1], bounds.max[2]],
    [bounds.max[0], bounds.max[1], bounds.min[2]],
    [bounds.max[0], bounds.max[1], bounds.max[2]],
  ];

  let newMin: Vector3 = [Infinity, Infinity, Infinity];
  let newMax: Vector3 = [-Infinity, -Infinity, -Infinity];

  for (const corner of corners) {
    const tc = transformPoint(transform, corner);
    newMin = [
      Math.min(newMin[0], tc[0]),
      Math.min(newMin[1], tc[1]),
      Math.min(newMin[2], tc[2]),
    ];
    newMax = [
      Math.max(newMax[0], tc[0]),
      Math.max(newMax[1], tc[1]),
      Math.max(newMax[2], tc[2]),
    ];
  }

  feature.bounds.min = newMin;
  feature.bounds.max = newMax;
}

/**
 * Transform all features in place
 */
export function transformFeatures(
  features: Feature[],
  transform: Matrix4
): void {
  logger.info({ featureCount: features.length }, 'Transforming features');

  for (const feature of features) {
    transformFeatureGeometry(feature, transform);
  }
}

/**
 * Compute transformed global bounds
 */
export function transformBounds(
  bounds: BoundingBox,
  transform: Matrix4
): BoundingBox {
  const corners: Vector3[] = [
    [bounds.min[0], bounds.min[1], bounds.min[2]],
    [bounds.min[0], bounds.min[1], bounds.max[2]],
    [bounds.min[0], bounds.max[1], bounds.min[2]],
    [bounds.min[0], bounds.max[1], bounds.max[2]],
    [bounds.max[0], bounds.min[1], bounds.min[2]],
    [bounds.max[0], bounds.min[1], bounds.max[2]],
    [bounds.max[0], bounds.max[1], bounds.min[2]],
    [bounds.max[0], bounds.max[1], bounds.max[2]],
  ];

  let newMin: Vector3 = [Infinity, Infinity, Infinity];
  let newMax: Vector3 = [-Infinity, -Infinity, -Infinity];

  for (const corner of corners) {
    const tc = transformPoint(transform, corner);
    newMin = [
      Math.min(newMin[0], tc[0]),
      Math.min(newMin[1], tc[1]),
      Math.min(newMin[2], tc[2]),
    ];
    newMax = [
      Math.max(newMax[0], tc[0]),
      Math.max(newMax[1], tc[1]),
      Math.max(newMax[2], tc[2]),
    ];
  }

  return { min: newMin, max: newMax };
}

/**
 * Compute the center of a bounding box
 */
export function computeBoundsCenter(bounds: BoundingBox): Vector3 {
  return [
    (bounds.min[0] + bounds.max[0]) / 2,
    (bounds.min[1] + bounds.max[1]) / 2,
    (bounds.min[2] + bounds.max[2]) / 2,
  ];
}

/**
 * Center geometry at origin for proper 3D Tiles placement
 *
 * Cesium best practice: Tile content should be centered at the local origin.
 * The root transform then positions the tileset in ECEF.
 *
 * This function:
 * 1. Translates all features so the bounding box center is at (0, 0, 0)
 * 2. Returns the original center (to be incorporated into root transform)
 *
 * @param features - Features to center (modified in place)
 * @param bounds - Global bounding box (will be updated)
 * @returns The original center point that was subtracted
 */
export function centerGeometryAtOrigin(
  features: Feature[],
  bounds: BoundingBox
): Vector3 {
  const center = computeBoundsCenter(bounds);

  // Skip centering if already near origin (threshold: 100 meters)
  const distanceFromOrigin = Math.sqrt(
    center[0] * center[0] + center[1] * center[1] + center[2] * center[2]
  );
  if (distanceFromOrigin < 100) {
    logger.debug({ center, distance: distanceFromOrigin }, 'Geometry already near origin, skipping centering');
    return [0, 0, 0];
  }

  logger.info(
    {
      originalCenter: center,
      centerX: center[0].toFixed(2),
      centerY: center[1].toFixed(2),
      centerZ: center[2].toFixed(2),
      distanceFromOrigin: distanceFromOrigin.toFixed(2),
    },
    'Centering geometry at origin'
  );

  // Create translation matrix to move center to origin
  const centeringTransform = createTranslation(-center[0], -center[1], -center[2]);

  // Transform all features
  for (const feature of features) {
    transformFeatureGeometry(feature, centeringTransform);
  }

  // Update the global bounds
  bounds.min = [
    bounds.min[0] - center[0],
    bounds.min[1] - center[1],
    bounds.min[2] - center[2],
  ];
  bounds.max = [
    bounds.max[0] - center[0],
    bounds.max[1] - center[1],
    bounds.max[2] - center[2],
  ];

  return center;
}

/**
 * Apply a pre-translation to the root transform
 *
 * This incorporates the centering offset into the root transform,
 * so the geometry ends up in the correct ECEF position even though
 * it's stored centered at the origin.
 *
 * @param rootTransform - The ENU-to-ECEF transform
 * @param centerOffset - The center offset from centerGeometryAtOrigin()
 * @returns New root transform with pre-translation applied
 */
export function applyPreTranslationToRootTransform(
  rootTransform: Matrix4,
  centerOffset: Vector3
): Matrix4 {
  if (centerOffset[0] === 0 && centerOffset[1] === 0 && centerOffset[2] === 0) {
    return rootTransform;
  }

  // Create translation for the center offset
  const preTranslation = createTranslation(centerOffset[0], centerOffset[1], centerOffset[2]);

  // We need: rootTransform * preTranslation (first translate locally, then ENU-to-ECEF)
  // multiplyChain applies in reverse order, so pass preTranslation first
  return multiplyChain(preTranslation, rootTransform);
}
