/**
 * Texture Optimizer
 *
 * Handles texture compression and filtering for optimal tile output.
 * - Filters textures to only include those referenced by materials in each tile
 * - Compresses textures to KTX2 (preferred) or WebP for smaller file sizes
 */

import { Document, Texture } from '@gltf-transform/core';
import { createLogger } from '../utils/logger.js';
import type {
  Feature,
  MaterialLibrary,
  PBRMaterial,
  TextureData,
} from '../types.js';

const logger = createLogger('texture-optimizer');

// Sharp is loaded dynamically to avoid issues if not installed
let sharpModule: typeof import('sharp') | null = null;

async function getSharp(): Promise<typeof import('sharp') | null> {
  if (sharpModule === null) {
    try {
      sharpModule = (await import('sharp')).default;
    } catch (e) {
      logger.warn('Sharp not available - WebP compression disabled');
      sharpModule = null;
    }
  }
  return sharpModule;
}

/**
 * Texture compression format
 */
export type TextureFormat = 'ktx2' | 'webp' | 'jpeg' | 'png' | 'original';

/**
 * Texture compression options
 */
export interface TextureCompressionOptions {
  /** Enable texture compression */
  enabled: boolean;

  /** Target format for compression (ktx2 preferred for GPU efficiency) */
  format: TextureFormat;

  /** KTX2-specific options */
  ktx2?: {
    /** Use UASTC (high quality) or ETC1S (smaller) */
    codec: 'uastc' | 'etc1s';
    /** Quality level (0-255 for UASTC, 1-255 for ETC1S) */
    quality?: number;
    /** Generate mipmaps */
    generateMipmap?: boolean;
  };

  /** Quality (0-100, for WebP/JPEG) */
  quality: number;

  /** Maximum texture dimension (textures larger than this will be resized) */
  maxSize?: number;

  /** Resize only if larger than maxSize (default: true) */
  resizeIfLarger?: boolean;
}

export const DEFAULT_COMPRESSION_OPTIONS: TextureCompressionOptions = {
  enabled: true,
  format: 'webp', // WebP is the default; KTX2 is experimental in Node.js
  ktx2: {
    codec: 'uastc',
    quality: 128,
    generateMipmap: true,
  },
  quality: 85,
  maxSize: 2048,
  resizeIfLarger: true,
};

/**
 * Filter a MaterialLibrary to only include materials and textures used by the given features
 */
export function filterMaterialLibrary(
  fullLibrary: MaterialLibrary,
  features: Feature[]
): MaterialLibrary {
  // Collect all material indices used by features
  const usedMaterialIndices = new Set<number>();
  for (const feature of features) {
    if (feature.materialIndex !== undefined) {
      usedMaterialIndices.add(feature.materialIndex);
    }
    // Also check sub-meshes
    if (feature.subMeshes) {
      for (const subMesh of feature.subMeshes) {
        usedMaterialIndices.add(subMesh.materialIndex);
      }
    }
  }

  // If no materials are used, return empty library
  if (usedMaterialIndices.size === 0) {
    return { materials: [], textures: [] };
  }

  // Collect all texture indices referenced by used materials
  const usedTextureIndices = new Set<number>();
  for (const materialIndex of usedMaterialIndices) {
    const material = fullLibrary.materials[materialIndex];
    if (!material) continue;

    collectTextureReferences(material, usedTextureIndices);
  }

  // Build remapping tables
  const textureRemap = new Map<number, number>();

  // Create filtered textures array with new indices
  const filteredTextures: TextureData[] = [];
  let newTextureIndex = 0;
  for (const oldIndex of Array.from(usedTextureIndices).sort((a, b) => a - b)) {
    if (fullLibrary.textures[oldIndex]) {
      textureRemap.set(oldIndex, newTextureIndex);
      filteredTextures.push(fullLibrary.textures[oldIndex]);
      newTextureIndex++;
    }
  }

  // Create filtered materials array with remapped texture references
  const filteredMaterials: PBRMaterial[] = [];
  let newMaterialIndex = 0;
  for (const oldIndex of Array.from(usedMaterialIndices).sort((a, b) => a - b)) {
    const material = fullLibrary.materials[oldIndex];
    if (!material) continue;

    // Clone material with remapped texture indices
    const remappedMaterial = remapMaterialTextures(material, textureRemap);
    filteredMaterials.push(remappedMaterial);
    newMaterialIndex++;
  }

  logger.debug(
    {
      originalMaterials: fullLibrary.materials.length,
      originalTextures: fullLibrary.textures.length,
      filteredMaterials: filteredMaterials.length,
      filteredTextures: filteredTextures.length,
    },
    'Filtered material library'
  );

  return {
    materials: filteredMaterials,
    textures: filteredTextures,
  };
}

/**
 * Collect all texture indices referenced by a material
 */
function collectTextureReferences(
  material: PBRMaterial,
  textureIndices: Set<number>
): void {
  if (material.baseColorTexture) {
    textureIndices.add(material.baseColorTexture.textureIndex);
  }
  if (material.metallicRoughnessTexture) {
    textureIndices.add(material.metallicRoughnessTexture.textureIndex);
  }
  if (material.normalTexture) {
    textureIndices.add(material.normalTexture.textureIndex);
  }
  if (material.occlusionTexture) {
    textureIndices.add(material.occlusionTexture.textureIndex);
  }
  if (material.emissiveTexture) {
    textureIndices.add(material.emissiveTexture.textureIndex);
  }
}

/**
 * Remap texture references in a material to new indices
 */
function remapMaterialTextures(
  material: PBRMaterial,
  textureRemap: Map<number, number>
): PBRMaterial {
  const remapped: PBRMaterial = { ...material };

  if (material.baseColorTexture) {
    const newIndex = textureRemap.get(material.baseColorTexture.textureIndex);
    if (newIndex !== undefined) {
      remapped.baseColorTexture = {
        ...material.baseColorTexture,
        textureIndex: newIndex,
      };
    } else {
      remapped.baseColorTexture = undefined;
    }
  }

  if (material.metallicRoughnessTexture) {
    const newIndex = textureRemap.get(material.metallicRoughnessTexture.textureIndex);
    if (newIndex !== undefined) {
      remapped.metallicRoughnessTexture = {
        ...material.metallicRoughnessTexture,
        textureIndex: newIndex,
      };
    } else {
      remapped.metallicRoughnessTexture = undefined;
    }
  }

  if (material.normalTexture) {
    const newIndex = textureRemap.get(material.normalTexture.textureIndex);
    if (newIndex !== undefined) {
      remapped.normalTexture = {
        ...material.normalTexture,
        textureIndex: newIndex,
      };
    } else {
      remapped.normalTexture = undefined;
    }
  }

  if (material.occlusionTexture) {
    const newIndex = textureRemap.get(material.occlusionTexture.textureIndex);
    if (newIndex !== undefined) {
      remapped.occlusionTexture = {
        ...material.occlusionTexture,
        textureIndex: newIndex,
      };
    } else {
      remapped.occlusionTexture = undefined;
    }
  }

  if (material.emissiveTexture) {
    const newIndex = textureRemap.get(material.emissiveTexture.textureIndex);
    if (newIndex !== undefined) {
      remapped.emissiveTexture = {
        ...material.emissiveTexture,
        textureIndex: newIndex,
      };
    } else {
      remapped.emissiveTexture = undefined;
    }
  }

  return remapped;
}

/**
 * Remap feature material indices after filtering
 */
export function remapFeatureMaterials(
  features: Feature[],
  fullLibrary: MaterialLibrary
): { features: Feature[]; materialRemap: Map<number, number> } {
  // Collect all material indices used by features
  const usedMaterialIndices = new Set<number>();
  for (const feature of features) {
    if (feature.materialIndex !== undefined) {
      usedMaterialIndices.add(feature.materialIndex);
    }
    if (feature.subMeshes) {
      for (const subMesh of feature.subMeshes) {
        usedMaterialIndices.add(subMesh.materialIndex);
      }
    }
  }

  // Build material remap
  const materialRemap = new Map<number, number>();
  let newMaterialIndex = 0;
  for (const oldIndex of Array.from(usedMaterialIndices).sort((a, b) => a - b)) {
    if (fullLibrary.materials[oldIndex]) {
      materialRemap.set(oldIndex, newMaterialIndex);
      newMaterialIndex++;
    }
  }

  // Remap feature material indices
  const remappedFeatures = features.map((feature) => {
    if (feature.materialIndex === undefined) {
      return feature;
    }

    const newIndex = materialRemap.get(feature.materialIndex);
    const remapped = {
      ...feature,
      materialIndex: newIndex,
    };

    if (feature.subMeshes) {
      remapped.subMeshes = feature.subMeshes.map((subMesh) => ({
        ...subMesh,
        materialIndex: materialRemap.get(subMesh.materialIndex) ?? subMesh.materialIndex,
      }));
    }

    return remapped;
  });

  return { features: remappedFeatures, materialRemap };
}

/**
 * Resize texture using Sharp (for pre-processing before KTX2 or WebP)
 */
async function resizeTexture(
  texture: TextureData,
  maxSize: number
): Promise<TextureData> {
  const sharp = await getSharp();
  if (!sharp) {
    return texture;
  }

  try {
    const metadata = await sharp(Buffer.from(texture.data)).metadata();
    const width = metadata.width || texture.width;
    const height = metadata.height || texture.height;

    if (width <= maxSize && height <= maxSize) {
      return texture;
    }

    const resizedBuffer = await sharp(Buffer.from(texture.data))
      .resize(maxSize, maxSize, {
        fit: 'inside',
        withoutEnlargement: true,
      })
      .png() // Convert to PNG for KTX2 encoder compatibility
      .toBuffer();

    const resizedMetadata = await sharp(resizedBuffer).metadata();

    return {
      ...texture,
      data: new Uint8Array(resizedBuffer),
      mimeType: 'image/png',
      width: resizedMetadata.width || maxSize,
      height: resizedMetadata.height || maxSize,
    };
  } catch (error) {
    logger.warn({ error, texture: texture.name }, 'Failed to resize texture');
    return texture;
  }
}

/**
 * Create an image decoder for ktx2-encoder using Sharp
 */
async function createImageDecoder(): Promise<
  ((buffer: Uint8Array) => Promise<{ width: number; height: number; data: Uint8Array }>) | null
> {
  const sharp = await getSharp();
  if (!sharp) return null;

  return async (buffer: Uint8Array) => {
    const image = sharp(Buffer.from(buffer));
    const metadata = await image.metadata();
    const { data, info } = await image
      .ensureAlpha()
      .raw()
      .toBuffer({ resolveWithObject: true });

    return {
      width: info.width,
      height: info.height,
      data: new Uint8Array(data),
    };
  };
}

/**
 * Compress a single texture to KTX2 format using ktx2-encoder
 */
export async function compressTextureToKTX2(
  texture: TextureData,
  options: TextureCompressionOptions
): Promise<TextureData> {
  try {
    // Dynamically import ktx2-encoder
    const { encodeToKTX2 } = await import('ktx2-encoder');

    // Create image decoder for Node.js
    const imageDecoder = await createImageDecoder();
    if (!imageDecoder) {
      logger.warn('Sharp not available for KTX2 encoding, falling back to WebP');
      return compressTextureToWebP(texture, options);
    }

    // Resize if needed before encoding
    let inputTexture = texture;
    if (options.maxSize && options.resizeIfLarger !== false) {
      inputTexture = await resizeTexture(inputTexture, options.maxSize);
    }

    // Encode to KTX2
    const ktx2Options = {
      isUASTC: options.ktx2?.codec !== 'etc1s',
      qualityLevel: options.ktx2?.quality ?? 128,
      generateMipmap: options.ktx2?.generateMipmap ?? true,
      imageDecoder,
      isKTX2File: true,
    };

    const ktx2Data = await encodeToKTX2(new Uint8Array(inputTexture.data), ktx2Options);

    const compressedTexture: TextureData = {
      ...texture,
      data: new Uint8Array(ktx2Data),
      mimeType: 'image/ktx2',
    };

    const compressionRatio = (1 - ktx2Data.byteLength / texture.data.length) * 100;
    logger.debug(
      {
        name: texture.name,
        originalSize: texture.data.length,
        compressedSize: ktx2Data.byteLength,
        compressionRatio: `${compressionRatio.toFixed(1)}%`,
        codec: ktx2Options.isUASTC ? 'UASTC' : 'ETC1S',
      },
      'Compressed texture to KTX2'
    );

    return compressedTexture;
  } catch (error) {
    logger.warn({ error, texture: texture.name }, 'Failed to compress texture to KTX2, falling back to WebP');
    // Fallback to WebP
    return compressTextureToWebP(texture, options);
  }
}

/**
 * Compress a single texture to WebP format
 */
export async function compressTextureToWebP(
  texture: TextureData,
  options: TextureCompressionOptions
): Promise<TextureData> {
  const sharp = await getSharp();
  if (!sharp) {
    return texture;
  }

  try {
    let image = sharp(Buffer.from(texture.data));

    // Get metadata for resizing decisions
    const metadata = await image.metadata();
    const width = metadata.width || texture.width;
    const height = metadata.height || texture.height;

    // Resize if needed
    if (
      options.maxSize &&
      options.resizeIfLarger !== false &&
      (width > options.maxSize || height > options.maxSize)
    ) {
      image = image.resize(options.maxSize, options.maxSize, {
        fit: 'inside',
        withoutEnlargement: true,
      });
    }

    const outputBuffer = await image.webp({ quality: options.quality }).toBuffer();

    // Get new dimensions after resize
    const outputMetadata = await sharp(outputBuffer).metadata();

    const compressedTexture: TextureData = {
      ...texture,
      data: new Uint8Array(outputBuffer),
      mimeType: 'image/webp',
      width: outputMetadata.width || width,
      height: outputMetadata.height || height,
    };

    const compressionRatio = (1 - outputBuffer.length / texture.data.length) * 100;
    logger.debug(
      {
        name: texture.name,
        originalSize: texture.data.length,
        compressedSize: outputBuffer.length,
        compressionRatio: `${compressionRatio.toFixed(1)}%`,
        format: 'webp',
      },
      'Compressed texture to WebP'
    );

    return compressedTexture;
  } catch (error) {
    logger.warn({ error, texture: texture.name }, 'Failed to compress texture to WebP');
    return texture;
  }
}

/**
 * Compress a single texture using the specified format
 */
export async function compressTexture(
  texture: TextureData,
  options: TextureCompressionOptions
): Promise<TextureData> {
  if (!options.enabled) {
    return texture;
  }

  switch (options.format) {
    case 'ktx2':
      return compressTextureToKTX2(texture, options);
    case 'webp':
      return compressTextureToWebP(texture, options);
    case 'jpeg': {
      const sharp = await getSharp();
      if (!sharp) return texture;
      const buffer = await sharp(Buffer.from(texture.data))
        .jpeg({ quality: options.quality })
        .toBuffer();
      return {
        ...texture,
        data: new Uint8Array(buffer),
        mimeType: 'image/jpeg',
      };
    }
    case 'png': {
      const sharp = await getSharp();
      if (!sharp) return texture;
      const buffer = await sharp(Buffer.from(texture.data))
        .png({ compressionLevel: 9 })
        .toBuffer();
      return {
        ...texture,
        data: new Uint8Array(buffer),
        mimeType: 'image/png',
      };
    }
    default:
      return texture;
  }
}

/**
 * Compress all textures in a MaterialLibrary
 */
export async function compressMaterialLibrary(
  library: MaterialLibrary,
  options: TextureCompressionOptions
): Promise<MaterialLibrary> {
  if (!options.enabled || library.textures.length === 0) {
    return library;
  }

  const startTime = Date.now();
  const compressedTextures: TextureData[] = [];

  // Process textures sequentially to avoid memory issues with large textures
  for (let i = 0; i < library.textures.length; i++) {
    const texture = library.textures[i];
    logger.debug({ index: i, total: library.textures.length, name: texture.name }, 'Compressing texture');
    const compressed = await compressTexture(texture, options);
    compressedTextures.push(compressed);
  }

  // Calculate total savings
  const originalSize = library.textures.reduce((sum, t) => sum + t.data.length, 0);
  const compressedSize = compressedTextures.reduce((sum, t) => sum + t.data.length, 0);
  const savings = originalSize - compressedSize;
  const processingTime = Date.now() - startTime;

  logger.info(
    {
      textureCount: library.textures.length,
      originalSize: `${(originalSize / 1024 / 1024).toFixed(2)} MB`,
      compressedSize: `${(compressedSize / 1024 / 1024).toFixed(2)} MB`,
      savings: `${(savings / 1024 / 1024).toFixed(2)} MB (${((savings / originalSize) * 100).toFixed(1)}%)`,
      format: options.format,
      processingTimeMs: processingTime,
    },
    'Compressed material library textures'
  );

  return {
    materials: library.materials,
    textures: compressedTextures,
  };
}

/**
 * Filter and compress materials for a specific tile
 * This is the main entry point for tile-specific texture optimization
 */
export async function optimizeMaterialsForTile(
  fullLibrary: MaterialLibrary,
  features: Feature[],
  compressionOptions?: Partial<TextureCompressionOptions>
): Promise<{ library: MaterialLibrary; remappedFeatures: Feature[] }> {
  const options = { ...DEFAULT_COMPRESSION_OPTIONS, ...compressionOptions };

  // First, filter to only include used materials/textures
  const filteredLibrary = filterMaterialLibrary(fullLibrary, features);

  // Remap feature material indices
  const { features: remappedFeatures } = remapFeatureMaterials(features, fullLibrary);

  // If no materials/textures, return early
  if (filteredLibrary.textures.length === 0) {
    return { library: filteredLibrary, remappedFeatures };
  }

  // Compress textures
  const compressedLibrary = await compressMaterialLibrary(filteredLibrary, options);

  return { library: compressedLibrary, remappedFeatures };
}

/**
 * Estimate the size reduction from texture optimization
 */
export function estimateOptimizationSavings(
  fullLibrary: MaterialLibrary,
  features: Feature[]
): {
  originalTextureBytes: number;
  filteredTextureBytes: number;
  estimatedCompressedBytes: number;
  estimatedSavingsPercent: number;
} {
  const filtered = filterMaterialLibrary(fullLibrary, features);

  const originalTextureBytes = fullLibrary.textures.reduce(
    (sum, t) => sum + t.data.length,
    0
  );
  const filteredTextureBytes = filtered.textures.reduce(
    (sum, t) => sum + t.data.length,
    0
  );

  // Estimate KTX2/UASTC compression at ~50% reduction for typical textures
  const estimatedCompressedBytes = Math.round(filteredTextureBytes * 0.5);

  const estimatedSavingsPercent =
    originalTextureBytes > 0
      ? ((originalTextureBytes - estimatedCompressedBytes) / originalTextureBytes) * 100
      : 0;

  return {
    originalTextureBytes,
    filteredTextureBytes,
    estimatedCompressedBytes,
    estimatedSavingsPercent,
  };
}
