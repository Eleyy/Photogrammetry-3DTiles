/**
 * Photo-Tiler
 *
 * Convert photogrammetry meshes to 3D Tiles 1.1 format.
 * Supports glTF/GLB and OBJ inputs from Pix4D, Agisoft, RealityCapture, DJI Terra.
 */

// Core pipeline
export {
  runPipeline,
  createDefaultConfig,
  createObjConfig,
  convert,
} from './pipeline.js';

// Types
export type {
  // Geometry
  BoundingBox,
  Feature,
  FeatureProperties,

  // Georeferencing
  Georeference,
  WGS84Position,

  // Tiling
  TileNode,
  TilingOptions,
  DracoOptions,

  // Output
  TilesetJson,
  TileJson,
  TilesetSchema,
  SchemaClass,
  SchemaProperty,
  OutputOptions,

  // Pipeline
  PipelineConfig,
  ProcessingResult,
  IngestionResult,
  InputType,
} from './types.js';

// Transform utilities
export {
  computeTransforms,
  transformFeatures,
  transformFeatureGeometry,
  transformBounds,
  type TransformConfig,
  type TransformResult,
  projectToWGS84,
  projectFromWGS84,
  georeferenceToWGS84,
  registerEPSG,
  wgs84ToEcef,
  ecefToWgs84,
  eastNorthUpToEcef,
  ecefToEastNorthUp,
  surfaceNormal,
  createIdentity,
  createTranslation,
  createRotationZ,
  createScale,
  createYUpToZUp,
  multiply,
  multiplyChain,
  transformPoint,
  transformDirection,
  invert,
  type Matrix4,
  type Vector3,
} from './transform/index.js';

// Ingestion utilities
export {
  ingest,
  loadGltf,
  createEmptyFeature,
  computeBounds,
  computeGlobalBounds,
  mergeGeometry,
  getIngestionStats,
  UNIT_TO_METERS,
  UNIT_NAMES,
  parseUnitString,
  createUserSpecifiedUnits,
  getDefaultUnits,
  formatUnitsInfo,
  type IngestionOptions,
} from './ingestion/index.js';

// Tiling utilities
export {
  generateTileset,
  generateTilesetWithProgress,
  buildTileHierarchy,
  createGLB,
  createTileGLB,
  writeTileset,
  buildTilesetJson,
  validateTileset,
  DEFAULT_TILING_OPTIONS,
  DEFAULT_DRACO_OPTIONS,
  type TilingResult,
} from './tiling/index.js';

// Error types
export {
  PhotoTilerError,
  InputError,
  GeoreferenceError,
  TransformError,
  TilingError,
  OutputError,
  ValidationError,
} from './utils/errors.js';

// Logger
export { createLogger } from './utils/logger.js';
