/**
 * glTF/GLB Loader
 *
 * Loads glTF 2.0 files and extracts geometry per feature (node).
 */

import {
  NodeIO,
  Document,
  Node,
  Mesh,
  Primitive,
  Material as GltfMaterial,
  Texture as GltfTexture,
} from '@gltf-transform/core';
import { mat4, mat3, vec3 } from 'gl-matrix';
import * as fs from 'fs';
import * as path from 'path';
import { createLogger } from '../utils/logger.js';
import { InputError } from '../utils/errors.js';
import type {
  Feature,
  BoundingBox,
  FeatureProperties,
  MaterialLibrary,
  PBRMaterial,
  TextureData,
  TextureReference,
  TextureSampler,
} from '../types.js';
import {
  createEmptyFeature,
  computeBounds,
  mergeGeometry,
} from './feature.js';

const logger = createLogger('gltf-loader');

// Try to import draco decoder
let dracoDecoder: unknown = null;
try {
  const draco3d = await import('draco3dgltf');
  dracoDecoder = await draco3d.createDecoderModule();
} catch (e) {
  logger.warn('Draco decoder not available - Draco-compressed files will fail');
}

/**
 * Create a NodeIO instance with Draco support if available
 */
async function createIO(): Promise<NodeIO> {
  const io = new NodeIO();

  if (dracoDecoder) {
    io.registerDependencies({
      'draco3d.decoder': dracoDecoder,
    });
  }

  return io;
}

/**
 * Read a large file (>2GB) into a Uint8Array using chunked reads.
 * Node.js fs.readFile has a 2GB limit, so we use fs.open + read for large files.
 */
async function readLargeFile(filePath: string): Promise<Uint8Array> {
  const stats = fs.statSync(filePath);
  const fileSize = stats.size;

  // For files under 2GB, use standard readFile
  const TWO_GB = 2 * 1024 * 1024 * 1024;
  if (fileSize < TWO_GB) {
    return new Uint8Array(fs.readFileSync(filePath));
  }

  logger.info({ fileSize, filePath }, 'Reading large file (>2GB) using chunked read');

  // For large files, read in chunks
  const CHUNK_SIZE = 256 * 1024 * 1024; // 256 MB chunks
  const buffer = Buffer.allocUnsafe(fileSize);
  const fd = fs.openSync(filePath, 'r');

  try {
    let bytesRead = 0;
    let position = 0;

    while (position < fileSize) {
      const chunkSize = Math.min(CHUNK_SIZE, fileSize - position);
      bytesRead = fs.readSync(fd, buffer, position, chunkSize, position);
      position += bytesRead;

      if (bytesRead === 0) break;

      // Log progress for very large files
      if (fileSize > 1024 * 1024 * 1024) {
        const progress = ((position / fileSize) * 100).toFixed(1);
        logger.debug({ progress: `${progress}%`, bytesRead: position }, 'Reading large file');
      }
    }

    logger.info({ totalBytes: position }, 'Large file read complete');
    return new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.length);
  } finally {
    fs.closeSync(fd);
  }
}

/**
 * Extract feature ID from a node
 * Priority: extras.id > node name > node_${index}
 */
function extractFeatureId(node: Node, index: number): string {
  const extras = node.getExtras();

  if (extras) {
    if (extras.id !== undefined) return String(extras.id);
  }

  const name = node.getName();
  if (name && name.trim().length > 0) {
    return name.trim();
  }

  return `node_${index}`;
}

/**
 * Extract display name from a node
 */
function extractFeatureName(node: Node, index: number): string {
  const name = node.getName();
  if (name && name.trim().length > 0) {
    return name.trim();
  }

  return `Mesh ${index}`;
}

/**
 * Extract initial properties from node extras
 */
function extractNodeProperties(node: Node): FeatureProperties {
  const name = node.getName();
  return {
    source: name || undefined,
  };
}

/**
 * Extract textures from a glTF document
 * Note: Sampler settings are stored per-TextureInfo in gltf-transform,
 * not on the texture itself. We extract them when materials reference textures.
 */
async function extractTextures(document: Document): Promise<TextureData[]> {
  const textures: TextureData[] = [];
  const root = document.getRoot();

  for (const texture of root.listTextures()) {
    const image = texture.getImage();
    if (!image) continue;

    const mimeType = texture.getMimeType() as TextureData['mimeType'];
    const size = texture.getSize();

    textures.push({
      name: texture.getName() || undefined,
      data: new Uint8Array(image),
      mimeType: mimeType || 'image/png',
      width: size?.[0] ?? 256,
      height: size?.[1] ?? 256,
      // Sampler settings will be extracted from TextureInfo when needed
    });
  }

  return textures;
}

/**
 * Get texture reference from a glTF texture info
 */
function getTextureReference(
  texture: GltfTexture | null,
  document: Document,
  texCoord: number = 0
): TextureReference | undefined {
  if (!texture) return undefined;

  const allTextures = document.getRoot().listTextures();
  const textureIndex = allTextures.indexOf(texture);
  if (textureIndex === -1) return undefined;

  return {
    textureIndex,
    texCoord,
  };
}

/**
 * Extract materials from a glTF document
 */
function extractMaterials(
  document: Document,
  textureCount: number
): PBRMaterial[] {
  const materials: PBRMaterial[] = [];
  const root = document.getRoot();

  for (const material of root.listMaterials()) {
    const baseColor = material.getBaseColorFactor() as [
      number,
      number,
      number,
      number,
    ];

    const pbrMaterial: PBRMaterial = {
      name: material.getName() || `material_${materials.length}`,
      baseColorFactor: baseColor,
      metallicFactor: material.getMetallicFactor(),
      roughnessFactor: material.getRoughnessFactor(),
      doubleSided: material.getDoubleSided(),
      alphaMode: material.getAlphaMode() as PBRMaterial['alphaMode'],
      alphaCutoff: material.getAlphaCutoff(),
    };

    // Base color texture
    const baseColorTexture = material.getBaseColorTexture();
    if (baseColorTexture) {
      pbrMaterial.baseColorTexture = getTextureReference(
        baseColorTexture,
        document,
        material.getBaseColorTextureInfo()?.getTexCoord() ?? 0
      );
    }

    // Metallic-roughness texture
    const mrTexture = material.getMetallicRoughnessTexture();
    if (mrTexture) {
      pbrMaterial.metallicRoughnessTexture = getTextureReference(
        mrTexture,
        document,
        material.getMetallicRoughnessTextureInfo()?.getTexCoord() ?? 0
      );
    }

    // Normal map
    const normalTexture = material.getNormalTexture();
    if (normalTexture) {
      pbrMaterial.normalTexture = getTextureReference(
        normalTexture,
        document,
        material.getNormalTextureInfo()?.getTexCoord() ?? 0
      );
      pbrMaterial.normalScale = material.getNormalScale();
    }

    // Occlusion map
    const occlusionTexture = material.getOcclusionTexture();
    if (occlusionTexture) {
      pbrMaterial.occlusionTexture = getTextureReference(
        occlusionTexture,
        document,
        material.getOcclusionTextureInfo()?.getTexCoord() ?? 0
      );
      pbrMaterial.occlusionStrength = material.getOcclusionStrength();
    }

    // Emissive
    const emissiveFactor = material.getEmissiveFactor() as [
      number,
      number,
      number,
    ];
    if (emissiveFactor.some((v) => v > 0)) {
      pbrMaterial.emissiveFactor = emissiveFactor;
    }

    const emissiveTexture = material.getEmissiveTexture();
    if (emissiveTexture) {
      pbrMaterial.emissiveTexture = getTextureReference(
        emissiveTexture,
        document,
        material.getEmissiveTextureInfo()?.getTexCoord() ?? 0
      );
    }

    materials.push(pbrMaterial);
  }

  return materials;
}

/**
 * Ensure indices exist for a primitive (generate if non-indexed)
 */
function ensureIndices(primitive: Primitive): Uint32Array | null {
  const indicesAccessor = primitive.getIndices();

  if (indicesAccessor) {
    const array = indicesAccessor.getArray();
    if (!array) return null;
    return new Uint32Array(array);
  }

  // Generate indices for non-indexed geometry
  const positionAccessor = primitive.getAttribute('POSITION');
  if (!positionAccessor) return null;

  const count = positionAccessor.getCount();
  const indices = new Uint32Array(count);
  for (let i = 0; i < count; i++) {
    indices[i] = i;
  }
  return indices;
}

/**
 * Apply world transform to positions and normals
 */
function applyWorldTransform(
  positions: Float32Array,
  normals: Float32Array | null,
  worldMatrix: mat4,
  unitScale: number
): { positions: Float32Array; normals: Float32Array | null } {
  const vertexCount = positions.length / 3;

  // Normal matrix is inverse transpose of upper-left 3x3
  const normalMatrix = mat3.create();
  mat3.normalFromMat4(normalMatrix, worldMatrix);

  const transformedPositions = new Float32Array(positions.length);
  const transformedNormals = normals ? new Float32Array(normals.length) : null;

  const tempPos = vec3.create();
  const tempNorm = vec3.create();

  for (let i = 0; i < vertexCount; i++) {
    // Transform position
    vec3.set(tempPos, positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2]);
    vec3.transformMat4(tempPos, tempPos, worldMatrix);

    transformedPositions[i * 3] = tempPos[0] * unitScale;
    transformedPositions[i * 3 + 1] = tempPos[1] * unitScale;
    transformedPositions[i * 3 + 2] = tempPos[2] * unitScale;

    // Transform normal
    if (transformedNormals && normals) {
      vec3.set(tempNorm, normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]);
      vec3.transformMat3(tempNorm, tempNorm, normalMatrix);
      vec3.normalize(tempNorm, tempNorm);

      transformedNormals[i * 3] = tempNorm[0];
      transformedNormals[i * 3 + 1] = tempNorm[1];
      transformedNormals[i * 3 + 2] = tempNorm[2];
    }
  }

  return { positions: transformedPositions, normals: transformedNormals };
}

/**
 * Load result including materials
 */
export interface GltfLoadResult {
  features: Feature[];
  globalBounds: BoundingBox;
  materials?: MaterialLibrary;
}

/**
 * Load a glTF/GLB file and extract features
 */
export async function loadGltf(
  filePath: string,
  unitScale: number = 1.0,
  extractMaterialsFlag: boolean = true
): Promise<GltfLoadResult> {
  logger.info({ filePath, unitScale }, 'Loading glTF file');

  const io = await createIO();
  let document: Document;

  try {
    // Check file size - use chunked read for large files (>2GB)
    const stats = fs.statSync(filePath);
    const TWO_GB = 2 * 1024 * 1024 * 1024;

    if (stats.size >= TWO_GB) {
      logger.info({ fileSize: stats.size }, 'Using binary read for large file');
      const buffer = await readLargeFile(filePath);
      document = await io.readBinary(buffer);
    } else {
      document = await io.read(filePath);
    }
  } catch (error) {
    throw new InputError(`Failed to read glTF file: ${filePath}`, { error });
  }

  const root = document.getRoot();
  const scenes = root.listScenes();

  if (scenes.length === 0) {
    throw new InputError('glTF file contains no scenes', { filePath });
  }

  // Extract materials and textures if requested
  let materialLibrary: MaterialLibrary | undefined;
  const allMaterials = root.listMaterials();

  if (extractMaterialsFlag && allMaterials.length > 0) {
    const textures = await extractTextures(document);
    const materials = extractMaterials(document, textures.length);

    materialLibrary = { materials, textures };

    logger.info(
      {
        materialCount: materials.length,
        textureCount: textures.length,
      },
      'Extracted materials and textures'
    );
  }

  const features: Feature[] = [];
  const featureMap = new Map<string, Feature>();
  let nodeIndex = 0;

  // Process all scenes
  for (const scene of scenes) {
    scene.traverse((node: Node) => {
      const mesh = node.getMesh();
      if (!mesh) return;

      const featureId = extractFeatureId(node, nodeIndex);
      const featureName = extractFeatureName(node, nodeIndex);

      // Get or create feature
      let feature = featureMap.get(featureId);
      if (!feature) {
        feature = createEmptyFeature(featureId, featureName);
        feature.properties = extractNodeProperties(node);
        featureMap.set(featureId, feature);
        features.push(feature);
      }

      // Get world transform
      const worldMatrix = node.getWorldMatrix() as mat4;

      // Process each primitive
      for (const primitive of mesh.listPrimitives()) {
        // Only handle triangles
        const mode = primitive.getMode();
        if (mode !== 4) {
          // 4 = TRIANGLES
          logger.debug({ featureId, mode }, 'Skipping non-triangle primitive');
          continue;
        }

        const posAccessor = primitive.getAttribute('POSITION');
        if (!posAccessor) continue;

        const positions = new Float32Array(posAccessor.getArray()!);

        const normAccessor = primitive.getAttribute('NORMAL');
        const normals = normAccessor
          ? new Float32Array(normAccessor.getArray()!)
          : null;

        const uvAccessor = primitive.getAttribute('TEXCOORD_0');
        const uvs = uvAccessor
          ? new Float32Array(uvAccessor.getArray()!)
          : null;

        // Extract vertex colors if present
        const colorAccessor = primitive.getAttribute('COLOR_0');
        let colors: Float32Array | null = null;
        if (colorAccessor) {
          const colorArray = colorAccessor.getArray()!;
          const componentsPerColor =
            colorAccessor.getType() === 'VEC4' ? 4 : 3;
          const vertexCount = posAccessor.getCount();

          // Convert to RGBA Float32Array
          colors = new Float32Array(vertexCount * 4);
          for (let i = 0; i < vertexCount; i++) {
            if (componentsPerColor === 4) {
              colors[i * 4] = colorArray[i * 4];
              colors[i * 4 + 1] = colorArray[i * 4 + 1];
              colors[i * 4 + 2] = colorArray[i * 4 + 2];
              colors[i * 4 + 3] = colorArray[i * 4 + 3];
            } else {
              colors[i * 4] = colorArray[i * 3];
              colors[i * 4 + 1] = colorArray[i * 3 + 1];
              colors[i * 4 + 2] = colorArray[i * 3 + 2];
              colors[i * 4 + 3] = 1.0;
            }
          }
        }

        const indices = ensureIndices(primitive);
        if (!indices || indices.length === 0) continue;

        // Get material index for this primitive
        const primitiveMaterial = primitive.getMaterial();
        let materialIndex: number | undefined;
        if (primitiveMaterial && materialLibrary) {
          materialIndex = allMaterials.indexOf(primitiveMaterial);
          if (materialIndex === -1) materialIndex = undefined;
        }

        // Apply world transform and unit scale
        const transformed = applyWorldTransform(
          positions,
          normals,
          worldMatrix,
          unitScale
        );

        // Merge into feature
        mergeGeometry(
          feature,
          transformed.positions,
          transformed.normals,
          uvs,
          indices,
          colors
        );

        // Set material index on feature (first material wins for simple case)
        if (materialIndex !== undefined && feature.materialIndex === undefined) {
          feature.materialIndex = materialIndex;
        }
      }

      nodeIndex++;
    });
  }

  // Filter out empty features
  const validFeatures = features.filter((f) => f.vertexCount > 0);

  if (validFeatures.length === 0) {
    throw new InputError('No valid mesh features found in glTF file', {
      filePath,
    });
  }

  // Compute global bounds
  let globalBounds: BoundingBox = {
    min: [Infinity, Infinity, Infinity],
    max: [-Infinity, -Infinity, -Infinity],
  };

  for (const feature of validFeatures) {
    globalBounds = {
      min: [
        Math.min(globalBounds.min[0], feature.bounds.min[0]),
        Math.min(globalBounds.min[1], feature.bounds.min[1]),
        Math.min(globalBounds.min[2], feature.bounds.min[2]),
      ],
      max: [
        Math.max(globalBounds.max[0], feature.bounds.max[0]),
        Math.max(globalBounds.max[1], feature.bounds.max[1]),
        Math.max(globalBounds.max[2], feature.bounds.max[2]),
      ],
    };
  }

  const triangleCount = validFeatures.reduce(
    (sum, f) => sum + f.triangleCount,
    0
  );

  logger.info(
    {
      featureCount: validFeatures.length,
      triangleCount,
      globalBounds,
      hasMaterials: !!materialLibrary,
    },
    'Loaded glTF file'
  );

  return {
    features: validFeatures,
    globalBounds,
    materials: materialLibrary,
  };
}
