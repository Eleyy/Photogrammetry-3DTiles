/**
 * Implicit Tiling for 3D Tiles 1.1
 *
 * Implements implicit octree/quadtree tiling as specified in the 3D Tiles 1.1 spec.
 * This is the modern approach used by Cesium Ion for regular grid-based subdivision.
 *
 * Benefits:
 * - Smaller tileset.json (no need to list every tile)
 * - Predictable tile addresses/URIs
 * - Template-based content URIs
 * - Subtree files for availability
 */

import * as fs from 'fs/promises';
import * as path from 'path';
import { createLogger } from '../utils/logger.js';
import type { Feature, BoundingBox, TileNode, TilingOptions } from '../types.js';
import {
  boundsCenter,
  boundsHalfSize,
  boundsDiagonal,
  totalTriangleCount,
  computeGlobalBounds,
} from '../ingestion/feature.js';

const logger = createLogger('implicit-tiling');

/**
 * Implicit tiling subdivision scheme
 */
export type SubdivisionScheme = 'OCTREE' | 'QUADTREE';

/**
 * Implicit tiling configuration
 */
export interface ImplicitTilingConfig {
  /** Subdivision scheme */
  subdivisionScheme: SubdivisionScheme;

  /** Number of levels in each subtree */
  subtreeLevels: number;

  /** Maximum total levels */
  availableLevels: number;

  /** Content URI template */
  contentUriTemplate: string;

  /** Subtree URI template */
  subtreeUriTemplate: string;
}

/**
 * Subtree file structure
 */
export interface Subtree {
  buffers?: { byteLength: number }[];
  bufferViews?: { buffer: number; byteOffset: number; byteLength: number }[];
  tileAvailability: { bitstream?: number; constant?: number };
  contentAvailability: { bitstream?: number; constant?: number }[];
  childSubtreeAvailability: { bitstream?: number; constant?: number };
}

/**
 * Morton code (Z-order) index for implicit tile addressing
 */
export interface MortonIndex {
  level: number;
  x: number;
  y: number;
  z: number;
}

/**
 * Convert level, x, y, z to Morton code
 */
export function toMortonCode(level: number, x: number, y: number, z: number): bigint {
  let morton = BigInt(0);

  for (let i = 0; i < level; i++) {
    const xBit = BigInt((x >> i) & 1);
    const yBit = BigInt((y >> i) & 1);
    const zBit = BigInt((z >> i) & 1);

    morton |= xBit << BigInt(i * 3);
    morton |= yBit << BigInt(i * 3 + 1);
    morton |= zBit << BigInt(i * 3 + 2);
  }

  return morton;
}

/**
 * Convert Morton code back to level, x, y, z
 */
export function fromMortonCode(morton: bigint, level: number): MortonIndex {
  let x = 0;
  let y = 0;
  let z = 0;

  for (let i = 0; i < level; i++) {
    x |= Number((morton >> BigInt(i * 3)) & BigInt(1)) << i;
    y |= Number((morton >> BigInt(i * 3 + 1)) & BigInt(1)) << i;
    z |= Number((morton >> BigInt(i * 3 + 2)) & BigInt(1)) << i;
  }

  return { level, x, y, z };
}

/**
 * Get the octant index (0-7) for a child
 */
export function getOctantIndex(x: number, y: number, z: number): number {
  return (x & 1) | ((y & 1) << 1) | ((z & 1) << 2);
}

/**
 * Implicit tile node for building the hierarchy
 */
export interface ImplicitTileNode {
  level: number;
  x: number;
  y: number;
  z: number;
  bounds: BoundingBox;
  features: Feature[];
  hasContent: boolean;
  hasChildren: boolean;
}

/**
 * Build implicit tile hierarchy
 */
export function buildImplicitHierarchy(
  features: Feature[],
  globalBounds: BoundingBox,
  options: {
    maxLevels: number;
    minFeaturesForContent: number;
    maxFeaturesPerTile: number;
    maxTrianglesPerTile: number;
  }
): Map<string, ImplicitTileNode> {
  const nodes = new Map<string, ImplicitTileNode>();

  // Create root node
  const rootKey = '0_0_0_0';
  nodes.set(rootKey, {
    level: 0,
    x: 0,
    y: 0,
    z: 0,
    bounds: globalBounds,
    features: [...features],
    hasContent: features.length >= options.minFeaturesForContent,
    hasChildren: false,
  });

  // Recursively subdivide
  subdivideImplicit(nodes, rootKey, options);

  return nodes;
}

/**
 * Subdivide a node into octants
 */
function subdivideImplicit(
  nodes: Map<string, ImplicitTileNode>,
  nodeKey: string,
  options: {
    maxLevels: number;
    minFeaturesForContent: number;
    maxFeaturesPerTile: number;
    maxTrianglesPerTile: number;
  }
): void {
  const node = nodes.get(nodeKey);
  if (!node) return;

  // Check stopping conditions
  if (node.level >= options.maxLevels) return;

  const featureCount = node.features.length;
  const triangleCount = totalTriangleCount(node.features);

  if (
    featureCount <= options.maxFeaturesPerTile &&
    triangleCount <= options.maxTrianglesPerTile
  ) {
    return;
  }

  // Subdivide into 8 octants
  const center = boundsCenter(node.bounds);
  const childLevel = node.level + 1;

  for (let dz = 0; dz < 2; dz++) {
    for (let dy = 0; dy < 2; dy++) {
      for (let dx = 0; dx < 2; dx++) {
        const childX = node.x * 2 + dx;
        const childY = node.y * 2 + dy;
        const childZ = node.z * 2 + dz;

        const childBounds: BoundingBox = {
          min: [
            dx === 0 ? node.bounds.min[0] : center[0],
            dy === 0 ? node.bounds.min[1] : center[1],
            dz === 0 ? node.bounds.min[2] : center[2],
          ],
          max: [
            dx === 0 ? center[0] : node.bounds.max[0],
            dy === 0 ? center[1] : node.bounds.max[1],
            dz === 0 ? center[2] : node.bounds.max[2],
          ],
        };

        // Find features in this octant
        const childFeatures = node.features.filter((f) => {
          const fc = boundsCenter(f.bounds);
          return (
            fc[0] >= childBounds.min[0] &&
            fc[0] <= childBounds.max[0] &&
            fc[1] >= childBounds.min[1] &&
            fc[1] <= childBounds.max[1] &&
            fc[2] >= childBounds.min[2] &&
            fc[2] <= childBounds.max[2]
          );
        });

        if (childFeatures.length === 0) continue;

        const childKey = `${childLevel}_${childX}_${childY}_${childZ}`;
        const childNode: ImplicitTileNode = {
          level: childLevel,
          x: childX,
          y: childY,
          z: childZ,
          bounds: childBounds,
          features: childFeatures,
          hasContent: childFeatures.length >= options.minFeaturesForContent,
          hasChildren: false,
        };

        nodes.set(childKey, childNode);
        node.hasChildren = true;

        // Recursively subdivide
        subdivideImplicit(nodes, childKey, options);
      }
    }
  }
}

/**
 * Generate implicit tiling tileset.json
 */
export function buildImplicitTilesetJson(
  globalBounds: BoundingBox,
  rootGeometricError: number,
  config: ImplicitTilingConfig,
  rootTransform?: number[]
): object {
  const center = boundsCenter(globalBounds);
  const halfSize = boundsHalfSize(globalBounds);

  const tileset = {
    asset: {
      version: '1.1',
      tilesetVersion: '1.0.0',
      generator: 'Photo-Tiler',
    },
    geometricError: rootGeometricError,
    root: {
      boundingVolume: {
        box: [
          center[0], center[1], center[2],
          halfSize[0], 0, 0,
          0, halfSize[1], 0,
          0, 0, halfSize[2],
        ],
      },
      geometricError: rootGeometricError,
      refine: 'REPLACE',
      content: {
        uri: config.contentUriTemplate.replace('{level}', '0').replace('{x}', '0').replace('{y}', '0').replace('{z}', '0'),
      },
      implicitTiling: {
        subdivisionScheme: config.subdivisionScheme,
        subtreeLevels: config.subtreeLevels,
        availableLevels: config.availableLevels,
        subtrees: {
          uri: config.subtreeUriTemplate,
        },
      },
      ...(rootTransform ? { transform: rootTransform } : {}),
    },
  };

  return tileset;
}

/**
 * Build subtree availability data
 */
export function buildSubtree(
  nodes: Map<string, ImplicitTileNode>,
  subtreeRoot: { level: number; x: number; y: number; z: number },
  subtreeLevels: number
): Subtree {
  const tileAvailability: boolean[] = [];
  const contentAvailability: boolean[] = [];
  const childSubtreeAvailability: boolean[] = [];

  // Calculate number of tiles in subtree
  let numTiles = 0;
  for (let l = 0; l < subtreeLevels; l++) {
    numTiles += Math.pow(8, l);
  }

  // Process each level in the subtree
  for (let relLevel = 0; relLevel < subtreeLevels; relLevel++) {
    const absLevel = subtreeRoot.level + relLevel;
    const tilesAtLevel = Math.pow(8, relLevel);

    for (let i = 0; i < tilesAtLevel; i++) {
      // Calculate absolute x, y, z for this tile
      const relCoords = fromMortonCode(BigInt(i), relLevel);
      const absX = subtreeRoot.x * Math.pow(2, relLevel) + relCoords.x;
      const absY = subtreeRoot.y * Math.pow(2, relLevel) + relCoords.y;
      const absZ = subtreeRoot.z * Math.pow(2, relLevel) + relCoords.z;

      const key = `${absLevel}_${absX}_${absY}_${absZ}`;
      const node = nodes.get(key);

      tileAvailability.push(!!node);
      contentAvailability.push(!!node?.hasContent);
    }
  }

  // Calculate child subtree availability (at subtreeLevels depth)
  const childSubtrees = Math.pow(8, subtreeLevels);
  for (let i = 0; i < childSubtrees; i++) {
    const relCoords = fromMortonCode(BigInt(i), subtreeLevels);
    const absLevel = subtreeRoot.level + subtreeLevels;
    const absX = subtreeRoot.x * Math.pow(2, subtreeLevels) + relCoords.x;
    const absY = subtreeRoot.y * Math.pow(2, subtreeLevels) + relCoords.y;
    const absZ = subtreeRoot.z * Math.pow(2, subtreeLevels) + relCoords.z;

    const key = `${absLevel}_${absX}_${absY}_${absZ}`;
    childSubtreeAvailability.push(nodes.has(key));
  }

  // Convert to buffers
  const tileBuffer = boolsToBuffer(tileAvailability);
  const contentBuffer = boolsToBuffer(contentAvailability);
  const childBuffer = boolsToBuffer(childSubtreeAvailability);

  // Check if all values are the same (can use constant)
  const allTilesAvailable = tileAvailability.every((b) => b);
  const noTilesAvailable = tileAvailability.every((b) => !b);
  const allContentAvailable = contentAvailability.every((b) => b);
  const noContentAvailable = contentAvailability.every((b) => !b);
  const allChildrenAvailable = childSubtreeAvailability.every((b) => b);
  const noChildrenAvailable = childSubtreeAvailability.every((b) => !b);

  const subtree: Subtree = {
    tileAvailability: allTilesAvailable
      ? { constant: 1 }
      : noTilesAvailable
      ? { constant: 0 }
      : { bitstream: 0 },
    contentAvailability: [
      allContentAvailable
        ? { constant: 1 }
        : noContentAvailable
        ? { constant: 0 }
        : { bitstream: 1 },
    ],
    childSubtreeAvailability: allChildrenAvailable
      ? { constant: 1 }
      : noChildrenAvailable
      ? { constant: 0 }
      : { bitstream: 2 },
  };

  // Add buffers if needed
  const buffers: ArrayBuffer[] = [];
  if (subtree.tileAvailability.bitstream !== undefined) {
    buffers[subtree.tileAvailability.bitstream] = tileBuffer;
  }
  if (subtree.contentAvailability[0].bitstream !== undefined) {
    buffers[subtree.contentAvailability[0].bitstream] = contentBuffer;
  }
  if (subtree.childSubtreeAvailability.bitstream !== undefined) {
    buffers[subtree.childSubtreeAvailability.bitstream] = childBuffer;
  }

  if (buffers.length > 0) {
    // Combine buffers
    const totalLength = buffers.reduce((sum, b) => sum + (b?.byteLength || 0), 0);
    const combinedBuffer = new ArrayBuffer(totalLength);
    const combinedView = new Uint8Array(combinedBuffer);
    let offset = 0;

    subtree.buffers = [];
    subtree.bufferViews = [];

    for (let i = 0; i < buffers.length; i++) {
      if (buffers[i]) {
        const view = new Uint8Array(buffers[i]);
        combinedView.set(view, offset);
        subtree.bufferViews.push({
          buffer: 0,
          byteOffset: offset,
          byteLength: view.byteLength,
        });
        offset += view.byteLength;
      }
    }

    subtree.buffers.push({ byteLength: totalLength });
  }

  return subtree;
}

/**
 * Convert boolean array to bit-packed buffer
 */
function boolsToBuffer(bools: boolean[]): ArrayBuffer {
  const byteLength = Math.ceil(bools.length / 8);
  const buffer = new ArrayBuffer(byteLength);
  const view = new Uint8Array(buffer);

  for (let i = 0; i < bools.length; i++) {
    if (bools[i]) {
      const byteIndex = Math.floor(i / 8);
      const bitIndex = i % 8;
      view[byteIndex] |= 1 << bitIndex;
    }
  }

  return buffer;
}

/**
 * Get content URI from template
 */
export function getContentUri(
  template: string,
  level: number,
  x: number,
  y: number,
  z: number
): string {
  return template
    .replace('{level}', String(level))
    .replace('{x}', String(x))
    .replace('{y}', String(y))
    .replace('{z}', String(z));
}

/**
 * Get subtree URI from template
 */
export function getSubtreeUri(
  template: string,
  level: number,
  x: number,
  y: number,
  z: number
): string {
  return template
    .replace('{level}', String(level))
    .replace('{x}', String(x))
    .replace('{y}', String(y))
    .replace('{z}', String(z));
}

/**
 * Write subtree file (.subtree)
 */
export async function writeSubtree(
  subtree: Subtree,
  outputPath: string
): Promise<void> {
  const json = JSON.stringify(subtree);
  const jsonBuffer = new TextEncoder().encode(json);

  // Subtree format: JSON chunk + optional binary chunk
  // For simplicity, we're writing JSON-only subtrees
  await fs.writeFile(outputPath, jsonBuffer);
}
