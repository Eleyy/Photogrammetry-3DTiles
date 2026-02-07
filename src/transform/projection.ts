/**
 * Coordinate Projection using proj4
 *
 * Handles conversion between projected coordinate systems (UTM, State Plane, etc.)
 * and WGS84 geographic coordinates.
 */

import proj4 from 'proj4';
import { GeoreferenceError } from '../utils/errors.js';
import { createLogger } from '../utils/logger.js';
import type { Georeference, WGS84Position } from '../types.js';

const logger = createLogger('projection');

// WGS84 definition (EPSG:4326)
const WGS84 = 'EPSG:4326';

// Common EPSG definitions that proj4 might not have built-in
const EPSG_DEFINITIONS: Record<number, string> = {
  // UTM Zones (WGS84)
  32601: '+proj=utm +zone=1 +datum=WGS84 +units=m +no_defs',
  32636: '+proj=utm +zone=36 +datum=WGS84 +units=m +no_defs',
  32637: '+proj=utm +zone=37 +datum=WGS84 +units=m +no_defs',
  32638: '+proj=utm +zone=38 +datum=WGS84 +units=m +no_defs',

  // Add more as needed...
  // State Plane coordinates, national grids, etc.
};

/**
 * Get proj4 definition string for an EPSG code
 */
function getEPSGDefinition(epsg: number): string {
  // Check our custom definitions first
  if (EPSG_DEFINITIONS[epsg]) {
    return EPSG_DEFINITIONS[epsg];
  }

  // Try to use proj4's built-in definitions
  const epsgString = `EPSG:${epsg}`;
  try {
    // proj4 will throw if not found
    proj4(epsgString);
    return epsgString;
  } catch {
    // Try to construct common projections
    if (epsg >= 32601 && epsg <= 32660) {
      // UTM North zones
      const zone = epsg - 32600;
      return `+proj=utm +zone=${zone} +datum=WGS84 +units=m +no_defs`;
    }
    if (epsg >= 32701 && epsg <= 32760) {
      // UTM South zones
      const zone = epsg - 32700;
      return `+proj=utm +zone=${zone} +south +datum=WGS84 +units=m +no_defs`;
    }

    throw new GeoreferenceError(
      `Unknown EPSG code: ${epsg}. Please add the proj4 definition.`,
      { epsg }
    );
  }
}

/**
 * Register a custom EPSG definition
 */
export function registerEPSG(epsg: number, definition: string): void {
  EPSG_DEFINITIONS[epsg] = definition;
  proj4.defs(`EPSG:${epsg}`, definition);
  logger.debug({ epsg, definition }, 'Registered custom EPSG definition');
}

/**
 * Convert projected coordinates (Easting/Northing) to WGS84 (lon/lat)
 */
export function projectToWGS84(
  easting: number,
  northing: number,
  epsg: number
): { longitude: number; latitude: number } {
  const sourceProj = getEPSGDefinition(epsg);

  try {
    const [longitude, latitude] = proj4(sourceProj, WGS84, [easting, northing]);

    logger.debug(
      { easting, northing, epsg, longitude, latitude },
      'Projected to WGS84'
    );

    return { longitude, latitude };
  } catch (error) {
    throw new GeoreferenceError(
      `Failed to project coordinates from EPSG:${epsg} to WGS84`,
      { easting, northing, epsg, error }
    );
  }
}

/**
 * Convert WGS84 (lon/lat) to projected coordinates (Easting/Northing)
 */
export function projectFromWGS84(
  longitude: number,
  latitude: number,
  epsg: number
): { easting: number; northing: number } {
  const targetProj = getEPSGDefinition(epsg);

  try {
    const [easting, northing] = proj4(WGS84, targetProj, [longitude, latitude]);
    return { easting, northing };
  } catch (error) {
    throw new GeoreferenceError(
      `Failed to project coordinates from WGS84 to EPSG:${epsg}`,
      { longitude, latitude, epsg, error }
    );
  }
}

/**
 * Convert georeference origin to WGS84 position
 */
export function georeferenceToWGS84(georef: Georeference): WGS84Position {
  const { longitude, latitude } = projectToWGS84(
    georef.origin.easting,
    georef.origin.northing,
    georef.epsg
  );

  // For now, we assume elevation is ellipsoidal height
  // TODO: Add geoid model support for orthometric heights
  let height = georef.origin.elevation;

  if (georef.heightReference === 'orthometric') {
    logger.warn(
      'Orthometric height reference specified but geoid model not loaded. ' +
      'Using elevation directly as ellipsoidal height.'
    );
    // In the future, we could use a geoid model (EGM96, etc.) to convert
    // height = elevation + geoidHeight(latitude, longitude)
  }

  return { longitude, latitude, height };
}

/**
 * Calculate the convergence angle (grid north vs true north) for a projected point
 * This is the angle between grid north and true north at a specific location.
 *
 * For UTM, this can be significant away from the central meridian.
 *
 * Note: The user-provided trueNorthRotation should already account for this,
 * but this function can be used to verify or calculate it.
 */
export function calculateConvergenceAngle(
  easting: number,
  northing: number,
  epsg: number
): number {
  // Convert the point to WGS84
  const { longitude, latitude } = projectToWGS84(easting, northing, epsg);

  // Calculate a small offset to the north in WGS84
  const deltaLat = 0.0001; // ~11 meters
  const northPoint = projectFromWGS84(longitude, latitude + deltaLat, epsg);

  // The angle of the grid line from the original point to the north point
  // relative to grid north (positive Y direction)
  const dE = northPoint.easting - easting;
  const dN = northPoint.northing - northing;

  // Convergence angle in degrees (positive = grid north is west of true north)
  const convergenceAngle = Math.atan2(dE, dN) * (180 / Math.PI);

  logger.debug(
    { easting, northing, epsg, convergenceAngle },
    'Calculated convergence angle'
  );

  return convergenceAngle;
}
