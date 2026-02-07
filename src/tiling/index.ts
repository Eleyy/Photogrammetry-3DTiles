/**
 * Tiling Module
 *
 * Handles spatial subdivision and 3D Tiles generation.
 */

import * as fs from 'fs/promises';
import * as path from 'path';
import { createLogger } from '../utils/logger.js';
import type {
  Feature,
  BoundingBox,
  TileNode,
  TilingOptions,
  DracoOptions,
  MaterialLibrary,
} from '../types.js';
import type { Matrix4 } from '../transform/matrix.js';
import type { TextureCompressionOptions } from './glb-writer.js';

// Re-export sub-modules
export {
  buildTileHierarchy,
  getHierarchyStats,
  traverseHierarchy,
  collectLeafNodes,
  collectContentNodes,
  computeTightBounds,
  recomputeGeometricErrors,
  DEFAULT_TILING_OPTIONS,
} from './octree.js';

export {
  createGLB,
  createTileGLB,
  mergeFeatures,
  estimateGLBSize,
  DEFAULT_DRACO_OPTIONS,
  DEFAULT_COMPRESSION_OPTIONS,
  type TextureCompressionOptions,
} from './glb-writer.js';

export {
  filterMaterialLibrary,
  remapFeatureMaterials,
  optimizeMaterialsForTile,
  compressMaterialLibrary,
  compressTexture,
  compressTextureToKTX2,
  compressTextureToWebP,
  estimateOptimizationSavings,
  type TextureFormat,
} from './texture-optimizer.js';

export {
  writeTileset,
  buildTilesetJson,
  boundsToBox,
  boundsToRegion,
  assignContentUris,
  getTileContentPath,
  ensureTileDirectories,
  getTilesetStats,
  validateTileset,
} from './tileset-writer.js';

export {
  buildImplicitHierarchy,
  buildImplicitTilesetJson,
  buildSubtree,
  toMortonCode,
  fromMortonCode,
  getContentUri,
  getSubtreeUri,
  writeSubtree,
  type SubdivisionScheme,
  type ImplicitTilingConfig,
  type ImplicitTileNode,
  type Subtree,
  type MortonIndex,
} from './implicit-tiling.js';

const logger = createLogger('tiling');

export interface TilingResult {
  /** Path to tileset.json */
  tilesetPath: string;

  /** Total number of tiles */
  tileCount: number;

  /** Number of tiles with content */
  contentTileCount: number;

  /** Total GLB size in bytes */
  totalGlbSize: number;

  /** Processing time in milliseconds */
  processingTimeMs: number;
}

/**
 * Generate a complete 3D Tiles tileset from features
 */
export async function generateTileset(
  features: Feature[],
  globalBounds: BoundingBox,
  outputDir: string,
  options: {
    tiling?: Partial<TilingOptions>;
    draco?: Partial<DracoOptions>;
    rootTransform?: Matrix4;
    tilesetVersion?: string;
    materials?: MaterialLibrary;
    textureCompression?: Partial<TextureCompressionOptions>;
  } = {}
): Promise<TilingResult> {
  const startTime = Date.now();

  logger.info(
    {
      featureCount: features.length,
      outputDir,
    },
    'Generating tileset'
  );

  // Import modules
  const { buildTileHierarchy, getHierarchyStats, collectContentNodes } =
    await import('./octree.js');
  const { createTileGLB, DEFAULT_DRACO_OPTIONS } = await import('./glb-writer.js');
  const {
    writeTileset,
    assignContentUris,
    ensureTileDirectories,
    getTileContentPath,
  } = await import('./tileset-writer.js');

  // Build tile hierarchy (always standard octree)
  const root = buildTileHierarchy(features, globalBounds, options.tiling);
  const stats = getHierarchyStats(root);

  // Assign content URIs
  assignContentUris(root);

  // Ensure directories exist
  await ensureTileDirectories(root, outputDir);

  // Generate GLB content for each tile
  const contentNodes = collectContentNodes(root);
  let totalGlbSize = 0;
  let contentTileCount = 0;

  const dracoOpts = { ...DEFAULT_DRACO_OPTIONS, ...options.draco };

  for (const node of contentNodes) {
    const glb = await createTileGLB(node, dracoOpts, options.materials, options.textureCompression);

    if (glb) {
      const contentPath = getTileContentPath(outputDir, node);
      if (contentPath) {
        await fs.writeFile(contentPath, glb);
        totalGlbSize += glb.byteLength;
        contentTileCount++;

        logger.debug(
          {
            address: node.address,
            size: glb.byteLength,
            features: node.ownFeatures.length,
          },
          'Wrote tile GLB'
        );
      }
    }
  }

  // Write tileset.json (no BIM schema)
  const tilesetPath = await writeTileset(root, outputDir, {
    rootTransform: options.rootTransform,
    tilesetVersion: options.tilesetVersion,
  });

  const processingTimeMs = Date.now() - startTime;

  logger.info(
    {
      tileCount: stats.tileCount,
      contentTileCount,
      totalGlbSize,
      processingTimeMs,
    },
    'Generated tileset'
  );

  return {
    tilesetPath,
    tileCount: stats.tileCount,
    contentTileCount,
    totalGlbSize,
    processingTimeMs,
  };
}

/**
 * Generate tileset with progress reporting
 */
export async function generateTilesetWithProgress(
  features: Feature[],
  globalBounds: BoundingBox,
  outputDir: string,
  options: {
    tiling?: Partial<TilingOptions>;
    draco?: Partial<DracoOptions>;
    rootTransform?: Matrix4;
    tilesetVersion?: string;
    materials?: MaterialLibrary;
    textureCompression?: Partial<TextureCompressionOptions>;
    onProgress?: (progress: {
      stage: string;
      current: number;
      total: number;
      message: string;
    }) => void;
  } = {}
): Promise<TilingResult> {
  const startTime = Date.now();
  const onProgress = options.onProgress || (() => {});

  logger.info(
    {
      featureCount: features.length,
      outputDir,
    },
    'Generating tileset with progress'
  );

  onProgress({
    stage: 'init',
    current: 0,
    total: 100,
    message: 'Building tile hierarchy...',
  });

  // Import modules
  const { buildTileHierarchy, getHierarchyStats, collectContentNodes } =
    await import('./octree.js');
  const { createTileGLB, DEFAULT_DRACO_OPTIONS } = await import('./glb-writer.js');
  const {
    writeTileset,
    assignContentUris,
    ensureTileDirectories,
    getTileContentPath,
  } = await import('./tileset-writer.js');

  // Build tile hierarchy (always standard octree)
  const root = buildTileHierarchy(features, globalBounds, options.tiling);
  const stats = getHierarchyStats(root);

  onProgress({
    stage: 'hierarchy',
    current: 10,
    total: 100,
    message: `Built hierarchy with ${stats.tileCount} tiles`,
  });

  // Assign content URIs
  assignContentUris(root);

  // Ensure directories exist
  await ensureTileDirectories(root, outputDir);

  onProgress({
    stage: 'directories',
    current: 15,
    total: 100,
    message: 'Created output directories',
  });

  // Generate GLB content for each tile
  const contentNodes = collectContentNodes(root);
  let totalGlbSize = 0;
  let contentTileCount = 0;

  const dracoOpts = { ...DEFAULT_DRACO_OPTIONS, ...options.draco };
  const totalNodes = contentNodes.length;

  for (let i = 0; i < contentNodes.length; i++) {
    const node = contentNodes[i];
    const glb = await createTileGLB(node, dracoOpts, options.materials, options.textureCompression);

    if (glb) {
      const contentPath = getTileContentPath(outputDir, node);
      if (contentPath) {
        await fs.writeFile(contentPath, glb);
        totalGlbSize += glb.byteLength;
        contentTileCount++;

        logger.debug(
          {
            address: node.address,
            size: glb.byteLength,
            features: node.ownFeatures.length,
          },
          'Wrote tile GLB'
        );
      }
    }

    // Report progress (15-95%)
    const progress = 15 + Math.floor(((i + 1) / totalNodes) * 80);
    onProgress({
      stage: 'tiles',
      current: progress,
      total: 100,
      message: `Generated tile ${i + 1}/${totalNodes}`,
    });
  }

  // Write tileset.json (no BIM schema)
  onProgress({
    stage: 'tileset',
    current: 95,
    total: 100,
    message: 'Writing tileset.json...',
  });

  const tilesetPath = await writeTileset(root, outputDir, {
    rootTransform: options.rootTransform,
    tilesetVersion: options.tilesetVersion,
  });

  const processingTimeMs = Date.now() - startTime;

  onProgress({
    stage: 'complete',
    current: 100,
    total: 100,
    message: `Complete! Generated ${contentTileCount} tiles`,
  });

  logger.info(
    {
      tileCount: stats.tileCount,
      contentTileCount,
      totalGlbSize,
      processingTimeMs,
    },
    'Generated tileset'
  );

  return {
    tilesetPath,
    tileCount: stats.tileCount,
    contentTileCount,
    totalGlbSize,
    processingTimeMs,
  };
}
