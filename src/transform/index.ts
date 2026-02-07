/**
 * Transform Module
 *
 * Handles coordinate system transformations from local coordinates
 * to WGS84/ECEF for 3D Tiles output.
 */

export {
  computeTransforms,
  transformFeatures,
  transformFeatureGeometry,
  transformBounds,
  computeBoundsCenter,
  centerGeometryAtOrigin,
  applyPreTranslationToRootTransform,
  type TransformConfig,
  type TransformResult,
} from './coordinates.js';

export {
  projectToWGS84,
  projectFromWGS84,
  georeferenceToWGS84,
  registerEPSG,
  calculateConvergenceAngle,
} from './projection.js';

export {
  wgs84ToEcef,
  ecefToWgs84,
  eastNorthUpToEcef,
  ecefToEastNorthUp,
  surfaceNormal,
  ecefDistance,
} from './ecef.js';

export {
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
  getTranslation,
  compose,
  isIdentity,
  type Matrix4,
  type Vector3,
} from './matrix.js';
