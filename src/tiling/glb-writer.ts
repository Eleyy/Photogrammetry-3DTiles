/**
 * GLB Writer for 3D Tiles 1.1
 *
 * Creates GLB files with:
 * - KHR_texture_basisu: For KTX2 compressed textures
 * - PBR materials and textures from photogrammetry
 */

import {
  Document,
  NodeIO,
  Accessor,
  Buffer as GltfBuffer,
  Mesh,
  Node,
  Primitive,
  Material,
  Texture,
  TextureInfo,
} from '@gltf-transform/core';
import { KHRTextureBasisu } from '@gltf-transform/extensions';
import { createLogger } from '../utils/logger.js';
import { OutputError } from '../utils/errors.js';
import {
  optimizeMaterialsForTile,
  TextureCompressionOptions,
  DEFAULT_COMPRESSION_OPTIONS,
} from './texture-optimizer.js';
import type {
  Feature,
  TileNode,
  DracoOptions,
  MaterialLibrary,
  PBRMaterial,
  TextureData,
  TextureReference,
} from '../types.js';

export type { TextureCompressionOptions } from './texture-optimizer.js';
export { DEFAULT_COMPRESSION_OPTIONS } from './texture-optimizer.js';

const logger = createLogger('glb-writer');

// Draco compression support
let dracoEncoder: unknown = null;
let dracoEncoderModule: any = null;

try {
  const draco3d = await import('draco3dgltf');
  dracoEncoderModule = await draco3d.createEncoderModule();
  dracoEncoder = dracoEncoderModule;
} catch (e) {
  logger.warn('Draco encoder not available - files will not be compressed');
}

/**
 * Default Draco options
 */
export const DEFAULT_DRACO_OPTIONS: DracoOptions = {
  enabled: true,
  compressionLevel: 7,
  quantizePositionBits: 14,
  quantizeNormalBits: 10,
  quantizeTexcoordBits: 12,
};

/**
 * Create textures in the document from material library
 * Note: In gltf-transform, sampler settings are set on TextureInfo, not on Texture.
 * We set them when assigning textures to materials.
 */
function createTextures(
  document: Document,
  materialLibrary: MaterialLibrary
): Texture[] {
  const textures: Texture[] = [];

  for (const textureData of materialLibrary.textures) {
    const texture = document.createTexture(textureData.name || 'texture');
    texture.setImage(textureData.data);
    texture.setMimeType(textureData.mimeType);
    textures.push(texture);
  }

  return textures;
}

/**
 * Apply sampler settings to a TextureInfo if sampler data exists
 */
function applySamplerSettings(
  textureInfo: TextureInfo | null,
  samplerData: TextureData['sampler']
): void {
  if (!textureInfo || !samplerData) return;

  if (samplerData.minFilter) {
    const minFilter = TextureInfo.MinFilter[samplerData.minFilter];
    if (minFilter !== undefined) {
      textureInfo.setMinFilter(minFilter);
    }
  }
  if (samplerData.magFilter) {
    const magFilter = TextureInfo.MagFilter[samplerData.magFilter];
    if (magFilter !== undefined) {
      textureInfo.setMagFilter(magFilter);
    }
  }
  if (samplerData.wrapS) {
    const wrapS = TextureInfo.WrapMode[samplerData.wrapS];
    if (wrapS !== undefined) {
      textureInfo.setWrapS(wrapS);
    }
  }
  if (samplerData.wrapT) {
    const wrapT = TextureInfo.WrapMode[samplerData.wrapT];
    if (wrapT !== undefined) {
      textureInfo.setWrapT(wrapT);
    }
  }
}

/**
 * Create materials in the document from material library
 */
function createMaterials(
  document: Document,
  materialLibrary: MaterialLibrary,
  textures: Texture[]
): Material[] {
  const materials: Material[] = [];

  for (const pbrMaterial of materialLibrary.materials) {
    const material = document.createMaterial(pbrMaterial.name);

    // Base color
    material.setBaseColorFactor(pbrMaterial.baseColorFactor);

    // Metallic-roughness
    material.setMetallicFactor(pbrMaterial.metallicFactor);
    material.setRoughnessFactor(pbrMaterial.roughnessFactor);

    // Base color texture
    if (
      pbrMaterial.baseColorTexture &&
      textures[pbrMaterial.baseColorTexture.textureIndex]
    ) {
      material.setBaseColorTexture(
        textures[pbrMaterial.baseColorTexture.textureIndex]
      );
      const samplerData =
        materialLibrary.textures[pbrMaterial.baseColorTexture.textureIndex]
          ?.sampler;
      applySamplerSettings(material.getBaseColorTextureInfo(), samplerData);
    }

    // Metallic-roughness texture
    if (
      pbrMaterial.metallicRoughnessTexture &&
      textures[pbrMaterial.metallicRoughnessTexture.textureIndex]
    ) {
      material.setMetallicRoughnessTexture(
        textures[pbrMaterial.metallicRoughnessTexture.textureIndex]
      );
      const samplerData =
        materialLibrary.textures[
          pbrMaterial.metallicRoughnessTexture.textureIndex
        ]?.sampler;
      applySamplerSettings(
        material.getMetallicRoughnessTextureInfo(),
        samplerData
      );
    }

    // Normal map
    if (
      pbrMaterial.normalTexture &&
      textures[pbrMaterial.normalTexture.textureIndex]
    ) {
      material.setNormalTexture(
        textures[pbrMaterial.normalTexture.textureIndex]
      );
      if (pbrMaterial.normalScale !== undefined) {
        material.setNormalScale(pbrMaterial.normalScale);
      }
      const samplerData =
        materialLibrary.textures[pbrMaterial.normalTexture.textureIndex]
          ?.sampler;
      applySamplerSettings(material.getNormalTextureInfo(), samplerData);
    }

    // Occlusion map
    if (
      pbrMaterial.occlusionTexture &&
      textures[pbrMaterial.occlusionTexture.textureIndex]
    ) {
      material.setOcclusionTexture(
        textures[pbrMaterial.occlusionTexture.textureIndex]
      );
      if (pbrMaterial.occlusionStrength !== undefined) {
        material.setOcclusionStrength(pbrMaterial.occlusionStrength);
      }
      const samplerData =
        materialLibrary.textures[pbrMaterial.occlusionTexture.textureIndex]
          ?.sampler;
      applySamplerSettings(material.getOcclusionTextureInfo(), samplerData);
    }

    // Emissive
    if (pbrMaterial.emissiveFactor) {
      material.setEmissiveFactor(pbrMaterial.emissiveFactor);
    }
    if (
      pbrMaterial.emissiveTexture &&
      textures[pbrMaterial.emissiveTexture.textureIndex]
    ) {
      material.setEmissiveTexture(
        textures[pbrMaterial.emissiveTexture.textureIndex]
      );
      const samplerData =
        materialLibrary.textures[pbrMaterial.emissiveTexture.textureIndex]
          ?.sampler;
      applySamplerSettings(material.getEmissiveTextureInfo(), samplerData);
    }

    // Alpha mode
    if (pbrMaterial.alphaMode) {
      material.setAlphaMode(pbrMaterial.alphaMode);
    }
    if (pbrMaterial.alphaCutoff !== undefined) {
      material.setAlphaCutoff(pbrMaterial.alphaCutoff);
    }

    // Double-sided
    if (pbrMaterial.doubleSided !== undefined) {
      material.setDoubleSided(pbrMaterial.doubleSided);
    }

    materials.push(material);
  }

  return materials;
}

/**
 * Create a GLB file from features
 */
export async function createGLB(
  features: Feature[],
  dracoOptions: Partial<DracoOptions> = {},
  materialLibrary?: MaterialLibrary,
  textureOptions?: Partial<TextureCompressionOptions>
): Promise<Uint8Array> {
  const opts = { ...DEFAULT_DRACO_OPTIONS, ...dracoOptions };
  const texOpts = textureOptions ? { ...DEFAULT_COMPRESSION_OPTIONS, ...textureOptions } : undefined;

  if (features.length === 0) {
    throw new OutputError('Cannot create GLB from empty features array');
  }

  logger.debug(
    {
      featureCount: features.length,
      draco: opts.enabled,
      hasMaterials: !!materialLibrary,
      textureFormat: texOpts?.format,
    },
    'Creating GLB'
  );

  const document = new Document();

  // Register KHR_texture_basisu extension for KTX2 textures
  const basisuExtension = document.createExtension(KHRTextureBasisu);

  // Create buffer for all geometry data
  const buffer = document.createBuffer();

  // Create textures and materials from library
  let gltfTextures: Texture[] = [];
  let gltfMaterials: Material[] = [];
  let processedFeatures = features;

  if (materialLibrary && materialLibrary.materials.length > 0) {
    // Optimize materials: filter to only used ones and compress textures
    let optimizedLibrary = materialLibrary;

    if (texOpts && texOpts.enabled) {
      const optimized = await optimizeMaterialsForTile(
        materialLibrary,
        features,
        texOpts
      );
      optimizedLibrary = optimized.library;
      processedFeatures = optimized.remappedFeatures;
    }

    gltfTextures = createTextures(document, optimizedLibrary);
    gltfMaterials = createMaterials(document, optimizedLibrary, gltfTextures);
    logger.debug(
      {
        originalTextures: materialLibrary.textures.length,
        filteredTextures: optimizedLibrary.textures.length,
        textureCount: gltfTextures.length,
        materialCount: gltfMaterials.length,
      },
      'Created materials and textures'
    );
  }

  // Create a default material for features without material assignment
  const defaultMaterial = document.createMaterial('default');
  defaultMaterial.setBaseColorFactor([0.8, 0.8, 0.8, 1.0]);
  defaultMaterial.setMetallicFactor(0.0);
  defaultMaterial.setRoughnessFactor(0.7);

  // Create a single mesh with multiple primitives
  const mesh = document.createMesh('features');

  for (const feature of processedFeatures) {
    if (feature.vertexCount === 0) continue;

    // Create primitive for this feature
    const primitive = document.createPrimitive();

    // Assign material
    if (
      feature.materialIndex !== undefined &&
      gltfMaterials[feature.materialIndex]
    ) {
      primitive.setMaterial(gltfMaterials[feature.materialIndex]);
    } else {
      primitive.setMaterial(defaultMaterial);
    }

    // Position accessor - copy to ensure ArrayBuffer type
    const positionAccessor = document.createAccessor('positions');
    positionAccessor.setArray(new Float32Array(feature.positions));
    positionAccessor.setType('VEC3');
    positionAccessor.setBuffer(buffer);
    primitive.setAttribute('POSITION', positionAccessor);

    // Normal accessor
    if (
      feature.normals &&
      feature.normals.length === feature.positions.length
    ) {
      const normalAccessor = document.createAccessor('normals');
      normalAccessor.setArray(new Float32Array(feature.normals));
      normalAccessor.setType('VEC3');
      normalAccessor.setBuffer(buffer);
      primitive.setAttribute('NORMAL', normalAccessor);
    }

    // UV accessor
    if (feature.uvs && feature.uvs.length > 0) {
      const uvAccessor = document.createAccessor('uvs');
      uvAccessor.setArray(new Float32Array(feature.uvs));
      uvAccessor.setType('VEC2');
      uvAccessor.setBuffer(buffer);
      primitive.setAttribute('TEXCOORD_0', uvAccessor);
    }

    // Vertex colors accessor
    if (feature.colors && feature.colors.length > 0) {
      const colorAccessor = document.createAccessor('colors');
      colorAccessor.setArray(new Float32Array(feature.colors));
      colorAccessor.setType('VEC4');
      colorAccessor.setBuffer(buffer);
      primitive.setAttribute('COLOR_0', colorAccessor);
    }

    // Index accessor
    const indexAccessor = document.createAccessor('indices');
    indexAccessor.setArray(new Uint32Array(feature.indices));
    indexAccessor.setType('SCALAR');
    indexAccessor.setBuffer(buffer);
    primitive.setIndices(indexAccessor);

    mesh.addPrimitive(primitive);
  }

  // Create scene node
  const node = document.createNode('root');
  node.setMesh(mesh);

  const scene = document.createScene('Scene');
  scene.addChild(node);
  document.getRoot().setDefaultScene(scene);

  // Write GLB
  const io = new NodeIO().registerExtensions([KHRTextureBasisu]);

  // Apply Draco compression if enabled and available
  if (opts.enabled && dracoEncoder) {
    try {
      const { draco } = await import('@gltf-transform/functions');
      await document.transform(
        draco({
          encoderModule: dracoEncoderModule as any,
          method: 'edgebreaker' as any,
          encodeSpeed: 5,
          decodeSpeed: 5,
          quantizePosition: opts.quantizePositionBits,
          quantizeNormal: opts.quantizeNormalBits,
          quantizeTexcoord: opts.quantizeTexcoordBits,
        } as any)
      );
      logger.debug('Applied Draco compression');
    } catch (error) {
      logger.warn({ error }, 'Failed to apply Draco compression');
    }
  }

  const glb = await io.writeBinary(document);

  logger.debug({ size: glb.byteLength }, 'Created GLB');
  return glb;
}

/**
 * Create GLB for a tile node
 */
export async function createTileGLB(
  node: TileNode,
  dracoOptions: Partial<DracoOptions> = {},
  materialLibrary?: MaterialLibrary,
  textureOptions?: Partial<TextureCompressionOptions>
): Promise<Uint8Array | null> {
  const features = node.ownFeatures;

  if (features.length === 0) {
    return null;
  }

  return createGLB(features, dracoOptions, materialLibrary, textureOptions);
}

/**
 * Merge multiple features into a single optimized feature
 * Useful for creating combined LOD content
 */
export function mergeFeatures(features: Feature[]): Feature | null {
  if (features.length === 0) return null;
  if (features.length === 1) return features[0];

  // Calculate total sizes
  let totalVertices = 0;
  let totalIndices = 0;
  let hasNormals = false;
  let hasUvs = false;
  let hasColors = false;

  for (const f of features) {
    totalVertices += f.vertexCount;
    totalIndices += f.indices.length;
    if (f.normals) hasNormals = true;
    if (f.uvs) hasUvs = true;
    if (f.colors) hasColors = true;
  }

  // Allocate merged arrays
  const positions = new Float32Array(totalVertices * 3);
  const normals = hasNormals ? new Float32Array(totalVertices * 3) : null;
  const uvs = hasUvs ? new Float32Array(totalVertices * 2) : null;
  const colors = hasColors ? new Float32Array(totalVertices * 4) : null;
  const indices = new Uint32Array(totalIndices);

  let vertexOffset = 0;
  let indexOffset = 0;

  for (const f of features) {
    // Copy positions
    positions.set(f.positions, vertexOffset * 3);

    // Copy normals
    if (normals) {
      if (f.normals) {
        normals.set(f.normals, vertexOffset * 3);
      }
    }

    // Copy UVs
    if (uvs) {
      if (f.uvs) {
        uvs.set(f.uvs, vertexOffset * 2);
      }
    }

    // Copy colors
    if (colors) {
      if (f.colors) {
        colors.set(f.colors, vertexOffset * 4);
      } else {
        // Default to white for features without colors
        for (let i = 0; i < f.vertexCount; i++) {
          const idx = (vertexOffset + i) * 4;
          colors[idx] = 1.0;
          colors[idx + 1] = 1.0;
          colors[idx + 2] = 1.0;
          colors[idx + 3] = 1.0;
        }
      }
    }

    // Copy indices with offset
    for (let i = 0; i < f.indices.length; i++) {
      indices[indexOffset + i] = f.indices[i] + vertexOffset;
    }

    vertexOffset += f.vertexCount;
    indexOffset += f.indices.length;
  }

  // Compute merged bounds
  let min: [number, number, number] = [Infinity, Infinity, Infinity];
  let max: [number, number, number] = [-Infinity, -Infinity, -Infinity];

  for (const f of features) {
    min = [
      Math.min(min[0], f.bounds.min[0]),
      Math.min(min[1], f.bounds.min[1]),
      Math.min(min[2], f.bounds.min[2]),
    ];
    max = [
      Math.max(max[0], f.bounds.max[0]),
      Math.max(max[1], f.bounds.max[1]),
      Math.max(max[2], f.bounds.max[2]),
    ];
  }

  return {
    id: 'merged',
    name: 'Merged Features',
    positions,
    normals,
    uvs,
    colors,
    indices,
    vertexCount: totalVertices,
    triangleCount: totalIndices / 3,
    bounds: { min, max },
    properties: {},
  };
}

/**
 * Estimate GLB file size before generation
 */
export function estimateGLBSize(features: Feature[]): number {
  let size = 0;

  for (const f of features) {
    // Positions: 4 bytes per float * 3 components * vertex count
    size += f.vertexCount * 3 * 4;

    // Normals
    if (f.normals) {
      size += f.vertexCount * 3 * 4;
    }

    // UVs
    if (f.uvs) {
      size += f.vertexCount * 2 * 4;
    }

    // Indices: 4 bytes per index
    size += f.indices.length * 4;
  }

  // Add overhead for GLB structure, JSON chunk, etc.
  size += 10000; // Rough estimate

  return size;
}
