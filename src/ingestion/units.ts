/**
 * Unit Detection and Conversion
 *
 * Detects input units from various sources and provides
 * conversion factors to meters for standardized 3D Tiles output.
 */

import { createLogger } from '../utils/logger.js';
import type { LinearUnit, DetectedUnits } from '../types.js';

const logger = createLogger('units');

/**
 * Unit conversion factors to meters
 */
export const UNIT_TO_METERS: Record<LinearUnit, number> = {
  mm: 0.001,
  cm: 0.01,
  m: 1.0,
  ft: 0.3048,
  in: 0.0254,
};

/**
 * Unit display names
 */
export const UNIT_NAMES: Record<LinearUnit, string> = {
  mm: 'millimeters',
  cm: 'centimeters',
  m: 'meters',
  ft: 'feet',
  in: 'inches',
};

/**
 * Parse a unit string and return detected units
 */
export function parseUnitString(unitStr: string): DetectedUnits | null {
  const lower = unitStr.toLowerCase().trim();

  if (lower.includes('millimeter') || lower === 'mm') {
    return {
      linearUnit: 'mm',
      toMeters: UNIT_TO_METERS.mm,
      source: 'detected',
      confidence: 'high',
    };
  }
  if (lower.includes('centimeter') || lower === 'cm') {
    return {
      linearUnit: 'cm',
      toMeters: UNIT_TO_METERS.cm,
      source: 'detected',
      confidence: 'high',
    };
  }
  if (lower.includes('meter') || lower === 'm') {
    // Make sure it's not millimeter or centimeter
    if (!lower.includes('milli') && !lower.includes('centi')) {
      return {
        linearUnit: 'm',
        toMeters: UNIT_TO_METERS.m,
        source: 'detected',
        confidence: 'high',
      };
    }
  }
  if (lower.includes('feet') || lower.includes('foot') || lower === 'ft') {
    return {
      linearUnit: 'ft',
      toMeters: UNIT_TO_METERS.ft,
      source: 'detected',
      confidence: 'high',
    };
  }
  if (lower.includes('inch') || lower === 'in' || lower === '"') {
    return {
      linearUnit: 'in',
      toMeters: UNIT_TO_METERS.in,
      source: 'detected',
      confidence: 'high',
    };
  }

  return null;
}

/**
 * Create DetectedUnits from user-specified unit
 */
export function createUserSpecifiedUnits(unit: LinearUnit): DetectedUnits {
  return {
    linearUnit: unit,
    toMeters: UNIT_TO_METERS[unit],
    source: 'user-specified',
    confidence: 'high',
  };
}

/**
 * Get default units (meters) when no detection is possible
 */
export function getDefaultUnits(): DetectedUnits {
  return {
    linearUnit: 'm',
    toMeters: 1.0,
    source: 'assumed',
    confidence: 'low',
  };
}

/**
 * Format units info for logging
 */
export function formatUnitsInfo(units: DetectedUnits): string {
  return `${UNIT_NAMES[units.linearUnit]} (${units.linearUnit}) [${units.source}, ${units.confidence} confidence]`;
}
