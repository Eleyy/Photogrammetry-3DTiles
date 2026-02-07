/**
 * OBJ to glTF Converter
 *
 * Wrapper around obj2gltf for converting OBJ files to glTF format.
 */

import * as fs from 'fs/promises';
import * as path from 'path';
import { createLogger } from '../utils/logger.js';
import { InputError } from '../utils/errors.js';

const logger = createLogger('obj-converter');

export interface ObjToGltfOptions {
  /** Output as binary GLB (default: true) */
  binary?: boolean;
  /** Separate textures instead of embedding (default: false) */
  separateTextures?: boolean;
  /** Check for double-sided materials (default: false) */
  checkTransparency?: boolean;
  /** Secure mode - disallow paths outside input directory (default: true) */
  secure?: boolean;
  /** Input up axis - 'X', 'Y', or 'Z' (default: 'Z' for photogrammetry OBJ files) */
  inputUpAxis?: 'X' | 'Y' | 'Z';
}

/**
 * Convert an OBJ file to glTF/GLB format using obj2gltf
 *
 * @param objPath - Path to the input OBJ file
 * @param outputPath - Path for the output glTF/GLB file
 * @param options - Conversion options
 * @returns Path to the generated glTF/GLB file
 */
export async function convertObjToGltf(
  objPath: string,
  outputPath: string,
  options: ObjToGltfOptions = {}
): Promise<string> {
  logger.info({ objPath, outputPath }, 'Converting OBJ to glTF');

  const {
    binary = true,
    separateTextures = false,
    checkTransparency = false,
    secure = true,
    inputUpAxis = 'Z', // Photogrammetry OBJ exports typically use Z-up
  } = options;

  try {
    // Dynamic import of obj2gltf (CommonJS module without type declarations)
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const obj2gltfModule = await (Function('return import("obj2gltf")')() as Promise<any>);
    const convert = obj2gltfModule.default || obj2gltfModule;

    // Determine output extension
    const outputExtension = binary ? '.glb' : '.gltf';
    const finalOutputPath = outputPath.endsWith(outputExtension)
      ? outputPath
      : outputPath.replace(/\.(gltf|glb)$/i, '') + outputExtension;

    logger.debug({ inputUpAxis }, 'Converting OBJ with up axis');

    // Run conversion
    // inputUpAxis tells obj2gltf what the input OBJ's up direction is
    // obj2gltf will then convert to Y-up glTF (glTF standard)
    const gltf = await convert(objPath, {
      binary,
      separateTextures,
      checkTransparency,
      secure,
      inputUpAxis,
    });

    // Write output file
    await fs.mkdir(path.dirname(finalOutputPath), { recursive: true });

    if (binary) {
      // GLB is returned as a Buffer
      await fs.writeFile(finalOutputPath, gltf as Buffer);
    } else {
      // glTF is returned as JSON
      await fs.writeFile(finalOutputPath, JSON.stringify(gltf, null, 2));
    }

    logger.info({ outputPath: finalOutputPath }, 'OBJ to glTF conversion complete');
    return finalOutputPath;
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error);
    throw new InputError(`Failed to convert OBJ to glTF: ${errorMessage}`, {
      objPath,
      outputPath,
      error: errorMessage,
    });
  }
}
