/**
 * Photo-Tiler Type Definitions
 */

// ============================================================================
// Geometry Types
// ============================================================================

export interface BoundingBox {
  min: [number, number, number];
  max: [number, number, number];
}

export interface Feature {
  /** Unique identifier */
  id: string;

  /** Display name */
  name: string;

  /** Vertex positions (xyz, interleaved) in meters */
  positions: Float32Array;

  /** Vertex normals (xyz, interleaved) */
  normals: Float32Array | null;

  /** Texture coordinates (uv, interleaved) */
  uvs: Float32Array | null;

  /** Vertex colors (RGBA, 0-1) */
  colors: Float32Array | null;

  /** Triangle indices */
  indices: Uint32Array;

  /** Vertex count */
  vertexCount: number;

  /** Triangle count */
  triangleCount: number;

  /** Axis-aligned bounding box */
  bounds: BoundingBox;

  /** Properties */
  properties: FeatureProperties;

  /** Material index (references MaterialLibrary.materials) */
  materialIndex?: number;

  /** Sub-meshes with different materials (for multi-material features) */
  subMeshes?: SubMesh[];
}

/**
 * Sub-mesh for features with multiple materials
 */
export interface SubMesh {
  /** Start index in the indices array */
  indexStart: number;

  /** Number of indices */
  indexCount: number;

  /** Material index */
  materialIndex: number;
}

export interface FeatureProperties {
  category?: string;
  name?: string;
  [key: string]: unknown;
}

// ============================================================================
// Material & Texture Types
// ============================================================================

/**
 * PBR Material definition (glTF 2.0 metallic-roughness)
 */
export interface PBRMaterial {
  /** Material name */
  name: string;

  /** Base color factor (RGBA, 0-1) */
  baseColorFactor: [number, number, number, number];

  /** Base color texture */
  baseColorTexture?: TextureReference;

  /** Metallic factor (0-1) */
  metallicFactor: number;

  /** Roughness factor (0-1) */
  roughnessFactor: number;

  /** Metallic-roughness texture (G=roughness, B=metallic) */
  metallicRoughnessTexture?: TextureReference;

  /** Normal map texture */
  normalTexture?: TextureReference;

  /** Normal scale (strength) */
  normalScale?: number;

  /** Occlusion texture (R channel) */
  occlusionTexture?: TextureReference;

  /** Occlusion strength (0-1) */
  occlusionStrength?: number;

  /** Emissive factor (RGB, 0-1) */
  emissiveFactor?: [number, number, number];

  /** Emissive texture */
  emissiveTexture?: TextureReference;

  /** Alpha mode */
  alphaMode?: 'OPAQUE' | 'MASK' | 'BLEND';

  /** Alpha cutoff (for MASK mode) */
  alphaCutoff?: number;

  /** Double-sided rendering */
  doubleSided?: boolean;
}

/**
 * Reference to a texture
 */
export interface TextureReference {
  /** Texture index in the textures array */
  textureIndex: number;

  /** UV coordinate set (TEXCOORD_n) */
  texCoord?: number;
}

/**
 * Texture definition
 */
export interface TextureData {
  /** Texture name */
  name?: string;

  /** Image data (PNG, JPEG, WebP, or KTX2) */
  data: Uint8Array;

  /** MIME type */
  mimeType: 'image/png' | 'image/jpeg' | 'image/webp' | 'image/ktx2';

  /** Image width in pixels */
  width: number;

  /** Image height in pixels */
  height: number;

  /** Sampler settings */
  sampler?: TextureSampler;
}

/**
 * Texture sampler settings
 */
export interface TextureSampler {
  /** Minification filter */
  minFilter?: 'NEAREST' | 'LINEAR' | 'NEAREST_MIPMAP_NEAREST' | 'LINEAR_MIPMAP_NEAREST' | 'NEAREST_MIPMAP_LINEAR' | 'LINEAR_MIPMAP_LINEAR';

  /** Magnification filter */
  magFilter?: 'NEAREST' | 'LINEAR';

  /** Wrap mode for S (U) coordinate */
  wrapS?: 'CLAMP_TO_EDGE' | 'MIRRORED_REPEAT' | 'REPEAT';

  /** Wrap mode for T (V) coordinate */
  wrapT?: 'CLAMP_TO_EDGE' | 'MIRRORED_REPEAT' | 'REPEAT';
}

/**
 * Material library for a tileset
 */
export interface MaterialLibrary {
  /** Materials by index */
  materials: PBRMaterial[];

  /** Textures by index */
  textures: TextureData[];
}

// ============================================================================
// Unit Types
// ============================================================================

/** Supported linear units */
export type LinearUnit = 'mm' | 'cm' | 'm' | 'ft' | 'in';

/** Detected or specified unit information */
export interface DetectedUnits {
  /** The linear unit */
  linearUnit: LinearUnit;

  /** Conversion factor to meters */
  toMeters: number;

  /** How the units were determined */
  source: 'detected' | 'user-specified' | 'assumed' | 'inferred';

  /** Confidence level of detection */
  confidence: 'high' | 'medium' | 'low';
}

/** Project-level information */
export interface ProjectInfo {
  /** Project name */
  name?: string;

  /** Project number */
  number?: string;

  /** Project address */
  address?: string;

  /** Detected units */
  units?: DetectedUnits;

  /** EPSG code if georeferenced */
  epsg?: number;

  /** Origin offset in source units */
  origin?: { x: number; y: number; z: number };
}

// ============================================================================
// Georeferencing Types
// ============================================================================

export interface Georeference {
  /** EPSG code (e.g., 32636 for UTM Zone 36N) */
  epsg: number;

  /** Origin in projected coordinates */
  origin: {
    easting: number;
    northing: number;
    elevation: number;
  };

  /** Rotation from grid north to true north (degrees, clockwise) */
  trueNorthRotation: number;

  /** Height reference system */
  heightReference?: 'ellipsoidal' | 'orthometric';
}

export interface WGS84Position {
  longitude: number;  // degrees
  latitude: number;   // degrees
  height: number;     // meters above WGS84 ellipsoid
}

// ============================================================================
// Tiling Types
// ============================================================================

export interface TileNode {
  /** Unique tile address (e.g., "root", "0", "0_1") */
  address: string;

  /** Path segments for file naming */
  pathSegments: number[];

  /** Hierarchy depth (root = 0) */
  level: number;

  /** Bounding box in local coordinates */
  bounds: BoundingBox;

  /** Geometric error for LOD decisions */
  geometricError: number;

  /** Features owned by this tile (not in children) */
  ownFeatures: Feature[];

  /** All features in this tile and descendants */
  aggregateFeatures: Feature[];

  /** Child tiles */
  children: TileNode[];

  /** Content URI (set after GLB generation) */
  contentUri?: string;
}

export interface TilingOptions {
  /** Maximum hierarchy depth */
  maxDepth: number;

  /** Maximum features per leaf tile */
  maxFeaturesPerTile: number;

  /** Maximum triangles per leaf tile */
  maxTrianglesPerTile: number;

  /** Minimum tile size in meters */
  minTileSize: number;

  /** Geometric error decay factor per level */
  geometricErrorDecay: number;

  /** Base geometric error (default: half root diagonal) */
  baseGeometricError?: number;
}

// ============================================================================
// Output Types
// ============================================================================

export interface DracoOptions {
  enabled: boolean;
  compressionLevel: number;
  quantizePositionBits: number;
  quantizeNormalBits: number;
  quantizeTexcoordBits: number;
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

export interface OutputOptions {
  /** Output directory */
  outputDir: string;

  /** Draco compression settings */
  draco: DracoOptions;

  /** Run validation after generation */
  validate: boolean;

  /** Include textures in output */
  includeTextures: boolean;

  /** Texture compression settings (used when includeTextures is true) */
  textureCompression?: Partial<TextureCompressionOptions>;
}

// ============================================================================
// Tileset JSON Types (3D Tiles 1.1)
// ============================================================================

export interface TilesetJson {
  asset: {
    version: '1.1';
    tilesetVersion?: string;
    generator?: string;
    /** Up-axis of GLB content: 'Y' (glTF default) or 'Z' (ENU convention) */
    gltfUpAxis?: 'X' | 'Y' | 'Z';
  };
  schema?: TilesetSchema;
  geometricError: number;
  root: TileJson;
  metadata?: Record<string, unknown>;
}

export interface TilesetSchema {
  id: string;
  classes: Record<string, SchemaClass>;
}

export interface SchemaClass {
  properties: Record<string, SchemaProperty>;
}

export interface SchemaProperty {
  type: 'STRING' | 'FLOAT32' | 'INT32' | 'UINT32' | 'BOOLEAN';
  description?: string;
  required?: boolean;
}

export interface TileJson {
  transform?: number[];
  boundingVolume: {
    box?: number[];
    region?: number[];
    sphere?: number[];
  };
  geometricError: number;
  refine?: 'ADD' | 'REPLACE';
  content?: {
    uri: string;
    boundingVolume?: TileJson['boundingVolume'];
  };
  children?: TileJson[];
}

// ============================================================================
// Ingestion Types
// ============================================================================

export type InputType = 'gltf' | 'obj' | 'ply';

export interface IngestionResult {
  /** Extracted features */
  features: Feature[];

  /** Global bounding box */
  globalBounds: BoundingBox;

  /** Total triangle count */
  triangleCount: number;

  /** Source file path or identifier */
  sourcePath: string;

  /** Detected or applied unit scale */
  unitScale: number;

  /** Material library (textures and PBR materials) */
  materials?: MaterialLibrary;

  /** Detected units information */
  detectedUnits?: DetectedUnits;

  /** Project information (if available) */
  projectInfo?: ProjectInfo;
}

// ============================================================================
// Pipeline Configuration
// ============================================================================

export interface PipelineConfig {
  /** Input configuration */
  input: {
    type: InputType;
    geometryPath?: string;
    metadataPath?: string;
    units?: LinearUnit;
  };

  /** Georeferencing configuration */
  georeference?: Georeference;

  /** Tiling configuration */
  tiling: TilingOptions;

  /** Output configuration */
  output: OutputOptions;
}

// ============================================================================
// Processing Results
// ============================================================================

export interface ProcessingResult {
  /** Path to generated tileset.json */
  tilesetPath: string;

  /** Number of tiles generated */
  tileCount: number;

  /** Number of features processed */
  featureCount: number;

  /** Total triangle count */
  triangleCount: number;

  /** Total output size in bytes */
  totalSizeBytes: number;

  /** Processing time in milliseconds */
  processingTimeMs: number;

  /** Validation results (if enabled) */
  validation?: {
    valid: boolean;
    errors: string[];
    warnings: string[];
  };

  /** Input units detected or specified */
  inputUnits?: DetectedUnits;

  /** Output is always meters + ECEF */
  outputInfo?: {
    units: 'meters';
    coordinateSystem: 'ECEF';
    epsg?: number;
  };
}
