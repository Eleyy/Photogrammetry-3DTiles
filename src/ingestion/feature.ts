/**
 * Feature Data Structure and Utilities
 */

import type { Feature, BoundingBox, FeatureProperties } from '../types.js';

/**
 * Create an empty feature
 */
export function createEmptyFeature(id: string, name: string): Feature {
  return {
    id,
    name,
    positions: new Float32Array(0),
    normals: null,
    uvs: null,
    colors: null,
    indices: new Uint32Array(0),
    vertexCount: 0,
    triangleCount: 0,
    bounds: {
      min: [Infinity, Infinity, Infinity],
      max: [-Infinity, -Infinity, -Infinity],
    },
    properties: {},
  };
}

/**
 * Compute bounding box from positions
 */
export function computeBounds(positions: Float32Array): BoundingBox {
  const min: [number, number, number] = [Infinity, Infinity, Infinity];
  const max: [number, number, number] = [-Infinity, -Infinity, -Infinity];

  for (let i = 0; i < positions.length; i += 3) {
    const x = positions[i];
    const y = positions[i + 1];
    const z = positions[i + 2];

    if (x < min[0]) min[0] = x;
    if (y < min[1]) min[1] = y;
    if (z < min[2]) min[2] = z;
    if (x > max[0]) max[0] = x;
    if (y > max[1]) max[1] = y;
    if (z > max[2]) max[2] = z;
  }

  return { min, max };
}

/**
 * Union of two bounding boxes
 */
export function unionBounds(a: BoundingBox, b: BoundingBox): BoundingBox {
  return {
    min: [
      Math.min(a.min[0], b.min[0]),
      Math.min(a.min[1], b.min[1]),
      Math.min(a.min[2], b.min[2]),
    ],
    max: [
      Math.max(a.max[0], b.max[0]),
      Math.max(a.max[1], b.max[1]),
      Math.max(a.max[2], b.max[2]),
    ],
  };
}

/**
 * Compute global bounds from features
 */
export function computeGlobalBounds(features: Feature[]): BoundingBox {
  let bounds: BoundingBox = {
    min: [Infinity, Infinity, Infinity],
    max: [-Infinity, -Infinity, -Infinity],
  };

  for (const feature of features) {
    bounds = unionBounds(bounds, feature.bounds);
  }

  return bounds;
}

/**
 * Merge geometry from a primitive into a feature
 */
export function mergeGeometry(
  feature: Feature,
  positions: Float32Array,
  normals: Float32Array | null,
  uvs: Float32Array | null,
  indices: Uint32Array,
  colors: Float32Array | null = null
): void {
  const vertexCount = positions.length / 3;
  const indexOffset = feature.vertexCount;

  // Merge positions
  const newPositions = new Float32Array(
    feature.positions.length + positions.length
  );
  newPositions.set(feature.positions, 0);
  newPositions.set(positions, feature.positions.length);
  feature.positions = newPositions;

  // Merge normals
  if (normals || feature.normals) {
    const prevNormals =
      feature.normals || new Float32Array(feature.vertexCount * 3);
    const newNormals = normals || new Float32Array(vertexCount * 3);
    const mergedNormals = new Float32Array(prevNormals.length + newNormals.length);
    mergedNormals.set(prevNormals, 0);
    mergedNormals.set(newNormals, prevNormals.length);
    feature.normals = mergedNormals;
  }

  // Merge UVs
  if (uvs || feature.uvs) {
    const prevUvs = feature.uvs || new Float32Array(feature.vertexCount * 2);
    const newUvs = uvs || new Float32Array(vertexCount * 2);
    const mergedUvs = new Float32Array(prevUvs.length + newUvs.length);
    mergedUvs.set(prevUvs, 0);
    mergedUvs.set(newUvs, prevUvs.length);
    feature.uvs = mergedUvs;
  }

  // Merge vertex colors (RGBA)
  if (colors || feature.colors) {
    const prevColors =
      feature.colors || new Float32Array(feature.vertexCount * 4);
    const newColors = colors || createDefaultColors(vertexCount);
    const mergedColors = new Float32Array(prevColors.length + newColors.length);
    mergedColors.set(prevColors, 0);
    mergedColors.set(newColors, prevColors.length);
    feature.colors = mergedColors;
  }

  // Merge indices (offset by previous vertex count)
  const adjustedIndices = new Uint32Array(indices.length);
  for (let i = 0; i < indices.length; i++) {
    adjustedIndices[i] = indices[i] + indexOffset;
  }
  const newIndices = new Uint32Array(
    feature.indices.length + adjustedIndices.length
  );
  newIndices.set(feature.indices, 0);
  newIndices.set(adjustedIndices, feature.indices.length);
  feature.indices = newIndices;

  // Update counts
  feature.vertexCount += vertexCount;
  feature.triangleCount += indices.length / 3;

  // Update bounds
  const addedBounds = computeBounds(positions);
  feature.bounds = unionBounds(feature.bounds, addedBounds);
}

/**
 * Create default white vertex colors
 */
function createDefaultColors(vertexCount: number): Float32Array {
  const colors = new Float32Array(vertexCount * 4);
  for (let i = 0; i < vertexCount; i++) {
    colors[i * 4] = 1.0;
    colors[i * 4 + 1] = 1.0;
    colors[i * 4 + 2] = 1.0;
    colors[i * 4 + 3] = 1.0;
  }
  return colors;
}

/**
 * Calculate total triangle count for features
 */
export function totalTriangleCount(features: Feature[]): number {
  return features.reduce((sum, f) => sum + f.triangleCount, 0);
}

/**
 * Calculate bounds diagonal (for geometric error)
 */
export function boundsDiagonal(bounds: BoundingBox): number {
  const dx = bounds.max[0] - bounds.min[0];
  const dy = bounds.max[1] - bounds.min[1];
  const dz = bounds.max[2] - bounds.min[2];
  return Math.sqrt(dx * dx + dy * dy + dz * dz);
}

/**
 * Get bounds center
 */
export function boundsCenter(bounds: BoundingBox): [number, number, number] {
  return [
    (bounds.min[0] + bounds.max[0]) / 2,
    (bounds.min[1] + bounds.max[1]) / 2,
    (bounds.min[2] + bounds.max[2]) / 2,
  ];
}

/**
 * Get bounds half-size
 */
export function boundsHalfSize(bounds: BoundingBox): [number, number, number] {
  return [
    (bounds.max[0] - bounds.min[0]) / 2,
    (bounds.max[1] - bounds.min[1]) / 2,
    (bounds.max[2] - bounds.min[2]) / 2,
  ];
}
