/**
 * Matrix Operations for Coordinate Transforms
 *
 * All matrices are 4x4 stored as 16-element arrays in COLUMN-MAJOR order
 * (compatible with glTF and 3D Tiles specifications)
 */

import { mat4, vec3, quat } from 'gl-matrix';

export type Matrix4 = number[];
export type Vector3 = [number, number, number];

/**
 * Create an identity matrix
 */
export function createIdentity(): Matrix4 {
  return Array.from(mat4.create());
}

/**
 * Create a translation matrix
 */
export function createTranslation(x: number, y: number, z: number): Matrix4 {
  const m = mat4.create();
  mat4.fromTranslation(m, [x, y, z]);
  return Array.from(m);
}

/**
 * Create a rotation matrix around the Z axis (for true north rotation)
 * @param degrees Rotation in degrees (positive = counterclockwise when viewed from above)
 */
export function createRotationZ(degrees: number): Matrix4 {
  const m = mat4.create();
  const radians = (degrees * Math.PI) / 180;
  mat4.fromZRotation(m, radians);
  return Array.from(m);
}

/**
 * Create a scale matrix
 */
export function createScale(sx: number, sy: number, sz: number): Matrix4 {
  const m = mat4.create();
  mat4.fromScaling(m, [sx, sy, sz]);
  return Array.from(m);
}

/**
 * Create a Y-up to Z-up conversion matrix
 *
 * Converts from Y-up coordinate system (glTF) to Z-up (ECEF/ENU for 3D Tiles).
 *
 * After obj2gltf converts Z-up OBJ to Y-up glTF:
 * - glTF X = East (from OBJ X)
 * - glTF Y = Up/elevation (from OBJ Z)
 * - glTF Z = South (from -OBJ Y, negated for right-handedness)
 *
 * For ENU (East-North-Up):
 * - ENU X = East = glTF X
 * - ENU Y = North = -glTF Z (negate South to get North)
 * - ENU Z = Up = glTF Y
 *
 * This is a +90° rotation around the X axis: [x, y, z] → [x, -z, y]
 */
export function createYUpToZUp(): Matrix4 {
  // +90° rotation around X axis
  // Equivalent to: [x, y, z] → [x, -z, y]
  const m = mat4.create();
  mat4.rotateX(m, m, Math.PI / 2);
  return Array.from(m);
}

/**
 * Multiply two 4x4 matrices (column-major)
 * Result = A * B
 */
export function multiply(a: Matrix4, b: Matrix4): Matrix4 {
  const ma = mat4.fromValues(...(a as Parameters<typeof mat4.fromValues>));
  const mb = mat4.fromValues(...(b as Parameters<typeof mat4.fromValues>));
  const result = mat4.create();
  mat4.multiply(result, ma, mb);
  return Array.from(result);
}

/**
 * Multiply a chain of matrices (left to right application)
 * For transforms applied in order [A, B, C], point p transforms as: C * B * A * p
 */
export function multiplyChain(...matrices: Matrix4[]): Matrix4 {
  if (matrices.length === 0) return createIdentity();
  if (matrices.length === 1) return [...matrices[0]];

  let result = matrices[0];
  for (let i = 1; i < matrices.length; i++) {
    result = multiply(matrices[i], result);
  }
  return result;
}

/**
 * Transform a point by a matrix
 */
export function transformPoint(matrix: Matrix4, point: Vector3): Vector3 {
  const m = mat4.fromValues(...(matrix as Parameters<typeof mat4.fromValues>));
  const p = vec3.fromValues(...point);
  const result = vec3.create();
  vec3.transformMat4(result, p, m);
  return [result[0], result[1], result[2]];
}

/**
 * Transform a direction vector by a matrix (ignores translation)
 */
export function transformDirection(matrix: Matrix4, direction: Vector3): Vector3 {
  const m = mat4.fromValues(...(matrix as Parameters<typeof mat4.fromValues>));
  // Zero out translation for direction transform
  m[12] = 0;
  m[13] = 0;
  m[14] = 0;
  const d = vec3.fromValues(...direction);
  const result = vec3.create();
  vec3.transformMat4(result, d, m);
  vec3.normalize(result, result);
  return [result[0], result[1], result[2]];
}

/**
 * Invert a matrix
 */
export function invert(matrix: Matrix4): Matrix4 | null {
  const m = mat4.fromValues(...(matrix as Parameters<typeof mat4.fromValues>));
  const result = mat4.create();
  const success = mat4.invert(result, m);
  if (!success) return null;
  return Array.from(result);
}

/**
 * Extract translation from a matrix
 */
export function getTranslation(matrix: Matrix4): Vector3 {
  return [matrix[12], matrix[13], matrix[14]];
}

/**
 * Create a matrix from translation, rotation (quaternion), and scale
 */
export function compose(
  translation: Vector3,
  rotation: [number, number, number, number],
  scale: Vector3
): Matrix4 {
  const m = mat4.create();
  mat4.fromRotationTranslationScale(
    m,
    quat.fromValues(...rotation),
    vec3.fromValues(...translation),
    vec3.fromValues(...scale)
  );
  return Array.from(m);
}

/**
 * Check if a matrix is approximately identity
 */
export function isIdentity(matrix: Matrix4, epsilon = 1e-10): boolean {
  const identity = createIdentity();
  for (let i = 0; i < 16; i++) {
    if (Math.abs(matrix[i] - identity[i]) > epsilon) {
      return false;
    }
  }
  return true;
}
