/**
 * ECEF (Earth-Centered Earth-Fixed) Coordinate Calculations
 *
 * Implements WGS84 ellipsoid calculations and East-North-Up to ECEF transforms.
 * This is equivalent to Cesium's Transforms.eastNorthUpToFixedFrame().
 */

import { createLogger } from '../utils/logger.js';
import type { WGS84Position } from '../types.js';
import type { Matrix4, Vector3 } from './matrix.js';

const logger = createLogger('ecef');

// WGS84 Ellipsoid Parameters
const WGS84 = {
  // Semi-major axis (equatorial radius) in meters
  a: 6378137.0,

  // Semi-minor axis (polar radius) in meters
  b: 6356752.314245,

  // Flattening
  f: 1 / 298.257223563,

  // First eccentricity squared
  e2: 0.00669437999014,

  // Second eccentricity squared
  ep2: 0.00673949674228,
};

/**
 * Convert degrees to radians
 */
function toRadians(degrees: number): number {
  return degrees * (Math.PI / 180);
}

/**
 * Convert WGS84 geodetic coordinates (lon, lat, height) to ECEF Cartesian coordinates
 */
export function wgs84ToEcef(position: WGS84Position): Vector3 {
  const lon = toRadians(position.longitude);
  const lat = toRadians(position.latitude);
  const h = position.height;

  const sinLon = Math.sin(lon);
  const cosLon = Math.cos(lon);
  const sinLat = Math.sin(lat);
  const cosLat = Math.cos(lat);

  // Radius of curvature in the prime vertical
  const N = WGS84.a / Math.sqrt(1 - WGS84.e2 * sinLat * sinLat);

  // ECEF coordinates
  const x = (N + h) * cosLat * cosLon;
  const y = (N + h) * cosLat * sinLon;
  const z = (N * (1 - WGS84.e2) + h) * sinLat;

  return [x, y, z];
}

/**
 * Convert ECEF Cartesian coordinates to WGS84 geodetic coordinates
 */
export function ecefToWgs84(ecef: Vector3): WGS84Position {
  const [x, y, z] = ecef;

  // Calculate longitude
  const longitude = Math.atan2(y, x) * (180 / Math.PI);

  // Iterative calculation for latitude (Bowring's method)
  const p = Math.sqrt(x * x + y * y);
  let lat = Math.atan2(z, p * (1 - WGS84.e2));

  for (let i = 0; i < 10; i++) {
    const sinLat = Math.sin(lat);
    const N = WGS84.a / Math.sqrt(1 - WGS84.e2 * sinLat * sinLat);
    const newLat = Math.atan2(z + WGS84.e2 * N * sinLat, p);
    if (Math.abs(newLat - lat) < 1e-12) break;
    lat = newLat;
  }

  const latitude = lat * (180 / Math.PI);

  // Calculate height
  const sinLat = Math.sin(lat);
  const cosLat = Math.cos(lat);
  const N = WGS84.a / Math.sqrt(1 - WGS84.e2 * sinLat * sinLat);
  const height = p / cosLat - N;

  return { longitude, latitude, height };
}

/**
 * Calculate the surface normal (up direction) at a WGS84 position
 * This points away from the ellipsoid surface (geodetic normal)
 */
export function surfaceNormal(position: WGS84Position): Vector3 {
  const lon = toRadians(position.longitude);
  const lat = toRadians(position.latitude);

  const cosLon = Math.cos(lon);
  const sinLon = Math.sin(lon);
  const cosLat = Math.cos(lat);
  const sinLat = Math.sin(lat);

  // Geodetic surface normal
  return [cosLat * cosLon, cosLat * sinLon, sinLat];
}

/**
 * Compute the East-North-Up to ECEF transformation matrix
 *
 * This creates a 4x4 matrix that transforms coordinates from a local
 * East-North-Up (ENU) frame centered at the given position to ECEF coordinates.
 *
 * The ENU frame has:
 * - X axis pointing East
 * - Y axis pointing North
 * - Z axis pointing Up (away from ellipsoid surface)
 *
 * This is equivalent to Cesium's Transforms.eastNorthUpToFixedFrame()
 */
export function eastNorthUpToEcef(position: WGS84Position): Matrix4 {
  const lon = toRadians(position.longitude);
  const lat = toRadians(position.latitude);

  const cosLon = Math.cos(lon);
  const sinLon = Math.sin(lon);
  const cosLat = Math.cos(lat);
  const sinLat = Math.sin(lat);

  // East vector (tangent to parallel, pointing east)
  const east: Vector3 = [-sinLon, cosLon, 0];

  // North vector (tangent to meridian, pointing north)
  const north: Vector3 = [-sinLat * cosLon, -sinLat * sinLon, cosLat];

  // Up vector (geodetic surface normal, pointing away from Earth)
  const up: Vector3 = [cosLat * cosLon, cosLat * sinLon, sinLat];

  // Origin in ECEF
  const origin = wgs84ToEcef(position);

  // Build 4x4 transformation matrix (column-major order)
  // | east.x  north.x  up.x  origin.x |
  // | east.y  north.y  up.y  origin.y |
  // | east.z  north.z  up.z  origin.z |
  // |   0       0       0       1     |
  const matrix: Matrix4 = [
    east[0], east[1], east[2], 0,
    north[0], north[1], north[2], 0,
    up[0], up[1], up[2], 0,
    origin[0], origin[1], origin[2], 1,
  ];

  logger.debug(
    {
      position,
      origin,
      east,
      north,
      up,
    },
    'Computed ENU-to-ECEF transform'
  );

  return matrix;
}

/**
 * Compute the ECEF to East-North-Up transformation matrix
 * (inverse of eastNorthUpToEcef)
 */
export function ecefToEastNorthUp(position: WGS84Position): Matrix4 {
  const lon = toRadians(position.longitude);
  const lat = toRadians(position.latitude);

  const cosLon = Math.cos(lon);
  const sinLon = Math.sin(lon);
  const cosLat = Math.cos(lat);
  const sinLat = Math.sin(lat);

  // These are the rows of the inverse rotation matrix (transpose of rotation)
  const east: Vector3 = [-sinLon, cosLon, 0];
  const north: Vector3 = [-sinLat * cosLon, -sinLat * sinLon, cosLat];
  const up: Vector3 = [cosLat * cosLon, cosLat * sinLon, sinLat];

  const origin = wgs84ToEcef(position);

  // Compute the translation in ENU frame
  const tx = -(east[0] * origin[0] + east[1] * origin[1] + east[2] * origin[2]);
  const ty = -(north[0] * origin[0] + north[1] * origin[1] + north[2] * origin[2]);
  const tz = -(up[0] * origin[0] + up[1] * origin[1] + up[2] * origin[2]);

  // Build inverse matrix (column-major)
  return [
    east[0], north[0], up[0], 0,
    east[1], north[1], up[1], 0,
    east[2], north[2], up[2], 0,
    tx, ty, tz, 1,
  ];
}

/**
 * Calculate the distance between two ECEF points
 */
export function ecefDistance(a: Vector3, b: Vector3): number {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const dz = b[2] - a[2];
  return Math.sqrt(dx * dx + dy * dy + dz * dz);
}

/**
 * Calculate the WGS84 ellipsoid height at a point (for reference)
 */
export function ellipsoidHeightAtLatitude(latitude: number): number {
  const lat = toRadians(latitude);
  const sinLat = Math.sin(lat);
  const cosLat = Math.cos(lat);

  // Radius at latitude (distance from center to surface)
  const r = Math.sqrt(
    (Math.pow(WGS84.a * WGS84.a * cosLat, 2) +
      Math.pow(WGS84.b * WGS84.b * sinLat, 2)) /
      (Math.pow(WGS84.a * cosLat, 2) + Math.pow(WGS84.b * sinLat, 2))
  );

  return r;
}
