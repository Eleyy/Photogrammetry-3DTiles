/**
 * Tileset JSON Writer for 3D Tiles 1.1
 *
 * Generates the tileset.json file with proper structure for:
 * - Bounding volumes (box format)
 * - Geometric error for LOD
 * - Content URIs
 * - Root transform for georeferencing
 */

import * as fs from 'fs/promises';
import * as path from 'path';
import { createLogger } from '../utils/logger.js';
import { OutputError } from '../utils/errors.js';
import type {
  TileNode,
  TilesetJson,
  TileJson,
  TilesetSchema,
  BoundingBox,
} from '../types.js';
import type { Matrix4 } from '../transform/matrix.js';
import {
  boundsCenter,
  boundsHalfSize,
  boundsDiagonal,
} from '../ingestion/feature.js';
import { traverseHierarchy, collectContentNodes } from './octree.js';

const logger = createLogger('tileset-writer');

/**
 * Write a complete 3D Tiles 1.1 tileset
 */
export async function writeTileset(
  root: TileNode,
  outputDir: string,
  options: {
    rootTransform?: Matrix4;
    tilesetVersion?: string;
  } = {}
): Promise<string> {
  logger.info({ outputDir }, 'Writing tileset');

  // Ensure output directory exists
  await fs.mkdir(outputDir, { recursive: true });

  // Build tileset JSON
  const tileset = buildTilesetJson(root, options);

  // Write tileset.json
  const tilesetPath = path.join(outputDir, 'tileset.json');
  await fs.writeFile(tilesetPath, JSON.stringify(tileset, null, 2), 'utf-8');

  logger.info({ tilesetPath }, 'Wrote tileset.json');

  return tilesetPath;
}

/**
 * Build the tileset.json structure
 */
export function buildTilesetJson(
  root: TileNode,
  options: {
    rootTransform?: Matrix4;
    tilesetVersion?: string;
  } = {}
): TilesetJson {
  const tileset: TilesetJson = {
    asset: {
      version: '1.1',
      tilesetVersion: options.tilesetVersion || '1.0.0',
      generator: 'Photo-Tiler',
      gltfUpAxis: 'Z',
    },
    geometricError: root.geometricError,
    root: buildTileJson(root, options.rootTransform),
  };

  return tileset;
}

/**
 * Build the tile JSON for a node
 */
function buildTileJson(node: TileNode, rootTransform?: Matrix4): TileJson {
  const tile: TileJson = {
    boundingVolume: {
      box: boundsToBox(node.bounds),
    },
    geometricError: node.geometricError,
    refine: 'REPLACE',
  };

  // Add root transform for georeferencing
  if (node.level === 0 && rootTransform) {
    tile.transform = Array.from(rootTransform);
  }

  // Add content if this tile has features
  if (node.contentUri) {
    tile.content = {
      uri: node.contentUri,
    };
  }

  // Recursively add children
  if (node.children.length > 0) {
    tile.children = node.children.map((child) => buildTileJson(child));
  }

  return tile;
}

/**
 * Convert bounding box to 3D Tiles box format
 * Box format: [center.x, center.y, center.z, halfX.x, halfX.y, halfX.z, halfY.x, halfY.y, halfY.z, halfZ.x, halfZ.y, halfZ.z]
 */
export function boundsToBox(bounds: BoundingBox): number[] {
  const center = boundsCenter(bounds);
  const halfSize = boundsHalfSize(bounds);

  // Axis-aligned box: half-extents along each axis
  return [
    center[0],
    center[1],
    center[2],
    halfSize[0],
    0,
    0, // X axis
    0,
    halfSize[1],
    0, // Y axis
    0,
    0,
    halfSize[2], // Z axis
  ];
}

/**
 * Convert bounding box to region format (for georeferenced data)
 * Region format: [west, south, east, north, minHeight, maxHeight] in radians and meters
 */
export function boundsToRegion(
  bounds: BoundingBox,
  wgs84Origin?: { longitude: number; latitude: number }
): number[] | null {
  if (!wgs84Origin) {
    return null;
  }

  // Convert local bounds to approximate WGS84 region
  // This is a simplified conversion - for accurate results, transform corners
  const metersPerDegree = 111319.9; // at equator
  const cosLat = Math.cos((wgs84Origin.latitude * Math.PI) / 180);

  const west = wgs84Origin.longitude + bounds.min[0] / (metersPerDegree * cosLat);
  const east = wgs84Origin.longitude + bounds.max[0] / (metersPerDegree * cosLat);
  const south = wgs84Origin.latitude + bounds.min[1] / metersPerDegree;
  const north = wgs84Origin.latitude + bounds.max[1] / metersPerDegree;

  return [
    (west * Math.PI) / 180,
    (south * Math.PI) / 180,
    (east * Math.PI) / 180,
    (north * Math.PI) / 180,
    bounds.min[2],
    bounds.max[2],
  ];
}

/**
 * Generate content URIs for all tiles in the hierarchy
 */
export function assignContentUris(
  root: TileNode,
  contentDir: string = 'tiles'
): void {
  traverseHierarchy(root, (node) => {
    if (node.ownFeatures.length > 0) {
      // Generate path based on tile address
      if (node.address === 'root') {
        node.contentUri = `${contentDir}/root.glb`;
      } else {
        // Use path segments to create directory structure
        const pathParts = node.pathSegments.join('/');
        node.contentUri = `${contentDir}/${pathParts}/tile.glb`;
      }
    }
  });
}

/**
 * Get the file system path for a tile's content
 */
export function getTileContentPath(
  outputDir: string,
  node: TileNode
): string | null {
  if (!node.contentUri) {
    return null;
  }
  return path.join(outputDir, node.contentUri);
}

/**
 * Ensure tile content directories exist
 */
export async function ensureTileDirectories(
  root: TileNode,
  outputDir: string
): Promise<void> {
  const contentNodes = collectContentNodes(root);

  for (const node of contentNodes) {
    if (node.contentUri) {
      const contentPath = path.join(outputDir, node.contentUri);
      const contentDir = path.dirname(contentPath);
      await fs.mkdir(contentDir, { recursive: true });
    }
  }
}

/**
 * Calculate tileset statistics
 */
export function getTilesetStats(tileset: TilesetJson): {
  tileCount: number;
  contentCount: number;
  maxDepth: number;
  totalGeometricError: number;
} {
  let tileCount = 0;
  let contentCount = 0;
  let maxDepth = 0;
  let totalGeometricError = 0;

  function traverse(tile: TileJson, depth: number): void {
    tileCount++;
    maxDepth = Math.max(maxDepth, depth);
    totalGeometricError += tile.geometricError;

    if (tile.content) {
      contentCount++;
    }

    if (tile.children) {
      for (const child of tile.children) {
        traverse(child, depth + 1);
      }
    }
  }

  traverse(tileset.root, 0);

  return {
    tileCount,
    contentCount,
    maxDepth,
    totalGeometricError,
  };
}

/**
 * Validate tileset structure
 */
export function validateTileset(tileset: TilesetJson): {
  valid: boolean;
  errors: string[];
  warnings: string[];
} {
  const errors: string[] = [];
  const warnings: string[] = [];

  // Check asset
  if (!tileset.asset?.version) {
    errors.push('Missing asset.version');
  } else if (tileset.asset.version !== '1.1') {
    warnings.push(`Expected version 1.1, got ${tileset.asset.version}`);
  }

  // Check root geometric error
  if (tileset.geometricError === undefined || tileset.geometricError < 0) {
    errors.push('Invalid or missing geometricError');
  }

  // Check root tile
  if (!tileset.root) {
    errors.push('Missing root tile');
  } else {
    validateTile(tileset.root, 'root', errors, warnings);
  }

  return {
    valid: errors.length === 0,
    errors,
    warnings,
  };
}

/**
 * Validate a tile recursively
 */
function validateTile(
  tile: TileJson,
  path: string,
  errors: string[],
  warnings: string[]
): void {
  // Check bounding volume
  if (!tile.boundingVolume) {
    errors.push(`${path}: Missing boundingVolume`);
  } else if (
    !tile.boundingVolume.box &&
    !tile.boundingVolume.region &&
    !tile.boundingVolume.sphere
  ) {
    errors.push(`${path}: boundingVolume must have box, region, or sphere`);
  }

  // Check geometric error
  if (tile.geometricError === undefined) {
    errors.push(`${path}: Missing geometricError`);
  } else if (tile.geometricError < 0) {
    errors.push(`${path}: geometricError must be >= 0`);
  }

  // Check refine (optional but recommended)
  if (tile.children && !tile.refine) {
    warnings.push(`${path}: Has children but no refine strategy specified`);
  }

  // Check content
  if (tile.content && !tile.content.uri) {
    errors.push(`${path}: content must have uri`);
  }

  // Validate children
  if (tile.children) {
    for (let i = 0; i < tile.children.length; i++) {
      validateTile(tile.children[i], `${path}.children[${i}]`, errors, warnings);
    }
  }
}
