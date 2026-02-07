/**
 * Octree Spatial Subdivision
 *
 * Implements adaptive octree subdivision for 3D Tiles hierarchy.
 * Features are assigned to tiles based on their centroids.
 */

import { createLogger } from '../utils/logger.js';
import { TilingError } from '../utils/errors.js';
import type { Feature, BoundingBox, TileNode, TilingOptions } from '../types.js';
import {
  computeGlobalBounds,
  unionBounds,
  totalTriangleCount,
  boundsDiagonal,
  boundsCenter,
} from '../ingestion/feature.js';

const logger = createLogger('octree');

/**
 * Default tiling options
 */
export const DEFAULT_TILING_OPTIONS: TilingOptions = {
  maxDepth: 6,
  maxFeaturesPerTile: 500,
  maxTrianglesPerTile: 100000,
  minTileSize: 1.0, // meters
  geometricErrorDecay: 0.5,
  baseGeometricError: undefined, // Will be computed from bounds
};

/**
 * Build a tile hierarchy using octree subdivision
 */
export function buildTileHierarchy(
  features: Feature[],
  globalBounds: BoundingBox,
  options: Partial<TilingOptions> = {}
): TileNode {
  const opts: TilingOptions = { ...DEFAULT_TILING_OPTIONS, ...options };

  // Compute base geometric error from bounds diagonal if not provided
  const diagonal = boundsDiagonal(globalBounds);
  const baseGeometricError = opts.baseGeometricError ?? diagonal / 2;

  logger.info(
    {
      featureCount: features.length,
      triangleCount: totalTriangleCount(features),
      diagonal,
      baseGeometricError,
      options: opts,
    },
    'Building tile hierarchy'
  );

  // Create root node
  const root: TileNode = {
    address: 'root',
    pathSegments: [],
    level: 0,
    bounds: globalBounds,
    geometricError: baseGeometricError,
    ownFeatures: [],
    aggregateFeatures: [...features],
    children: [],
  };

  // Recursively subdivide
  subdivideNode(root, opts, baseGeometricError);

  // Assign features to appropriate tiles
  distributeFeatures(root);

  // Log hierarchy stats
  const stats = getHierarchyStats(root);
  logger.info(stats, 'Built tile hierarchy');

  return root;
}

/**
 * Recursively subdivide a tile node
 */
function subdivideNode(
  node: TileNode,
  options: TilingOptions,
  baseGeometricError: number
): void {
  const featureCount = node.aggregateFeatures.length;
  const triangleCount = totalTriangleCount(node.aggregateFeatures);

  // Check stopping conditions
  if (node.level >= options.maxDepth) {
    logger.debug(
      { address: node.address, reason: 'maxDepth' },
      'Stopping subdivision'
    );
    return;
  }

  if (
    featureCount <= options.maxFeaturesPerTile &&
    triangleCount <= options.maxTrianglesPerTile
  ) {
    logger.debug(
      { address: node.address, featureCount, triangleCount, reason: 'belowThreshold' },
      'Stopping subdivision'
    );
    return;
  }

  // Check minimum tile size
  const diagonal = boundsDiagonal(node.bounds);
  if (diagonal / 2 < options.minTileSize) {
    logger.debug(
      { address: node.address, diagonal, reason: 'minTileSize' },
      'Stopping subdivision'
    );
    return;
  }

  // Create 8 child octants
  const childBounds = splitBoundsIntoOctants(node.bounds);
  const childGeometricError = node.geometricError * options.geometricErrorDecay;

  for (let i = 0; i < 8; i++) {
    // Find features in this octant
    const childFeatures = node.aggregateFeatures.filter((f) =>
      featureIntersectsBounds(f, childBounds[i])
    );

    if (childFeatures.length === 0) {
      continue; // Skip empty octants
    }

    const child: TileNode = {
      address: node.address === 'root' ? String(i) : `${node.address}_${i}`,
      pathSegments: [...node.pathSegments, i],
      level: node.level + 1,
      bounds: childBounds[i],
      geometricError: childGeometricError,
      ownFeatures: [],
      aggregateFeatures: childFeatures,
      children: [],
    };

    node.children.push(child);

    // Recursively subdivide child
    subdivideNode(child, options, baseGeometricError);
  }
}

/**
 * Split a bounding box into 8 octants
 */
function splitBoundsIntoOctants(bounds: BoundingBox): BoundingBox[] {
  const center = boundsCenter(bounds);
  const octants: BoundingBox[] = [];

  // Order: -X-Y-Z, +X-Y-Z, -X+Y-Z, +X+Y-Z, -X-Y+Z, +X-Y+Z, -X+Y+Z, +X+Y+Z
  for (let z = 0; z < 2; z++) {
    for (let y = 0; y < 2; y++) {
      for (let x = 0; x < 2; x++) {
        const min: [number, number, number] = [
          x === 0 ? bounds.min[0] : center[0],
          y === 0 ? bounds.min[1] : center[1],
          z === 0 ? bounds.min[2] : center[2],
        ];
        const max: [number, number, number] = [
          x === 0 ? center[0] : bounds.max[0],
          y === 0 ? center[1] : bounds.max[1],
          z === 0 ? center[2] : bounds.max[2],
        ];
        octants.push({ min, max });
      }
    }
  }

  return octants;
}

/**
 * Check if a feature's centroid is within bounds
 */
function featureIntersectsBounds(feature: Feature, bounds: BoundingBox): boolean {
  const featureCenter = boundsCenter(feature.bounds);

  return (
    featureCenter[0] >= bounds.min[0] &&
    featureCenter[0] <= bounds.max[0] &&
    featureCenter[1] >= bounds.min[1] &&
    featureCenter[1] <= bounds.max[1] &&
    featureCenter[2] >= bounds.min[2] &&
    featureCenter[2] <= bounds.max[2]
  );
}

/**
 * Distribute features to leaf tiles
 * Features belong to the smallest tile that contains them
 */
function distributeFeatures(node: TileNode): void {
  if (node.children.length === 0) {
    // Leaf node - all aggregate features are owned
    node.ownFeatures = [...node.aggregateFeatures];
    return;
  }

  // Non-leaf: features owned by children are removed from this node's ownership
  const childFeatureIds = new Set<string>();

  for (const child of node.children) {
    distributeFeatures(child);
    for (const f of child.aggregateFeatures) {
      childFeatureIds.add(f.id);
    }
  }

  // Features not in any child are owned by this node (unlikely with octree)
  node.ownFeatures = node.aggregateFeatures.filter(
    (f) => !childFeatureIds.has(f.id)
  );
}

/**
 * Get statistics about the tile hierarchy
 */
export function getHierarchyStats(root: TileNode): {
  tileCount: number;
  leafCount: number;
  maxDepth: number;
  avgFeaturesPerLeaf: number;
  avgTrianglesPerLeaf: number;
} {
  let tileCount = 0;
  let leafCount = 0;
  let maxDepth = 0;
  let totalLeafFeatures = 0;
  let totalLeafTriangles = 0;

  function traverse(node: TileNode): void {
    tileCount++;
    maxDepth = Math.max(maxDepth, node.level);

    if (node.children.length === 0) {
      leafCount++;
      totalLeafFeatures += node.ownFeatures.length;
      totalLeafTriangles += totalTriangleCount(node.ownFeatures);
    }

    for (const child of node.children) {
      traverse(child);
    }
  }

  traverse(root);

  return {
    tileCount,
    leafCount,
    maxDepth,
    avgFeaturesPerLeaf: leafCount > 0 ? totalLeafFeatures / leafCount : 0,
    avgTrianglesPerLeaf: leafCount > 0 ? totalLeafTriangles / leafCount : 0,
  };
}

/**
 * Traverse the tile hierarchy and apply a function to each node
 */
export function traverseHierarchy(
  root: TileNode,
  fn: (node: TileNode) => void
): void {
  fn(root);
  for (const child of root.children) {
    traverseHierarchy(child, fn);
  }
}

/**
 * Collect all leaf nodes (tiles with content)
 */
export function collectLeafNodes(root: TileNode): TileNode[] {
  const leaves: TileNode[] = [];

  function traverse(node: TileNode): void {
    if (node.children.length === 0) {
      leaves.push(node);
    } else {
      for (const child of node.children) {
        traverse(child);
      }
    }
  }

  traverse(root);
  return leaves;
}

/**
 * Collect all nodes with content (non-empty own features)
 */
export function collectContentNodes(root: TileNode): TileNode[] {
  const nodes: TileNode[] = [];

  function traverse(node: TileNode): void {
    if (node.ownFeatures.length > 0) {
      nodes.push(node);
    }
    for (const child of node.children) {
      traverse(child);
    }
  }

  traverse(root);
  return nodes;
}

/**
 * Compute tight bounds for a tile based on its features
 */
export function computeTightBounds(node: TileNode): BoundingBox {
  if (node.ownFeatures.length === 0) {
    // For non-leaf nodes, compute from children
    if (node.children.length > 0) {
      let bounds: BoundingBox = {
        min: [Infinity, Infinity, Infinity],
        max: [-Infinity, -Infinity, -Infinity],
      };
      for (const child of node.children) {
        bounds = unionBounds(bounds, computeTightBounds(child));
      }
      return bounds;
    }
    return node.bounds;
  }

  return computeGlobalBounds(node.ownFeatures);
}

/**
 * Re-compute geometric errors based on actual bounds
 */
export function recomputeGeometricErrors(
  root: TileNode,
  baseError?: number
): void {
  const actualDiagonal = boundsDiagonal(computeTightBounds(root));
  const base = baseError ?? actualDiagonal / 2;

  function traverse(node: TileNode, parentError: number): void {
    // Geometric error should be approximately half the tile diagonal
    const diagonal = boundsDiagonal(node.bounds);
    node.geometricError = Math.min(parentError * 0.5, diagonal / 2);

    for (const child of node.children) {
      traverse(child, node.geometricError);
    }
  }

  root.geometricError = base;
  for (const child of root.children) {
    traverse(child, base);
  }
}
