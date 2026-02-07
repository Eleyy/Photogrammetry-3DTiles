/**
 * Custom Error Types for Photo-Tiler
 */

export class PhotoTilerError extends Error {
  constructor(
    message: string,
    public code: string,
    public details?: unknown
  ) {
    super(message);
    this.name = 'PhotoTilerError';
    Error.captureStackTrace(this, this.constructor);
  }
}

export class InputError extends PhotoTilerError {
  constructor(message: string, details?: unknown) {
    super(message, 'INPUT_ERROR', details);
    this.name = 'InputError';
  }
}

export class GeoreferenceError extends PhotoTilerError {
  constructor(message: string, details?: unknown) {
    super(message, 'GEOREF_ERROR', details);
    this.name = 'GeoreferenceError';
  }
}

export class TransformError extends PhotoTilerError {
  constructor(message: string, details?: unknown) {
    super(message, 'TRANSFORM_ERROR', details);
    this.name = 'TransformError';
  }
}

export class TilingError extends PhotoTilerError {
  constructor(message: string, details?: unknown) {
    super(message, 'TILING_ERROR', details);
    this.name = 'TilingError';
  }
}

export class OutputError extends PhotoTilerError {
  constructor(message: string, details?: unknown) {
    super(message, 'OUTPUT_ERROR', details);
    this.name = 'OutputError';
  }
}

export class ValidationError extends PhotoTilerError {
  constructor(message: string, details?: unknown) {
    super(message, 'VALIDATION_ERROR', details);
    this.name = 'ValidationError';
  }
}
