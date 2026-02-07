# Photo-Tiler: Photogrammetry Mesh to 3D Tiles 1.1

## Comprehensive Implementation Plan

> Fork of BIM-Tiler, purpose-built for converting photogrammetry meshes (Pix4D, Agisoft Metashape, RealityCapture, DJI Terra, OpenDroneMap) into OGC 3D Tiles 1.1 with proper LOD, geometry splitting, and streaming support.

---

## Table of Contents

1. [Why Fork, Not Extend](#1-why-fork-not-extend)
2. [Input Format Analysis](#2-input-format-analysis)
3. [Architecture Overview](#3-architecture-overview)
4. [BIM-Tiler Code Audit: Keep vs Remove vs Modify](#4-bim-tiler-code-audit-keep-vs-remove-vs-modify)
5. [New Modules to Build](#5-new-modules-to-build)
6. [Phase-by-Phase Implementation](#6-phase-by-phase-implementation)
7. [CLI Design](#7-cli-design)
8. [Output Structure](#8-output-structure)
9. [Georeferencing Strategy](#9-georeferencing-strategy)
10. [Texture Strategy](#10-texture-strategy)
11. [Memory & Performance Strategy](#11-memory--performance-strategy)
12. [Existing Tools Analysis & Differentiation](#12-existing-tools-analysis--differentiation)
13. [Testing Strategy](#13-testing-strategy)
14. [Dependencies](#14-dependencies)
15. [Risk Register](#15-risk-register)

---

## 1. Why Fork, Not Extend

BIM-Tiler and Photo-Tiler solve fundamentally different problems:

| Dimension | BIM-Tiler | Photo-Tiler |
|-----------|-----------|-------------|
| **Input structure** | Many small features (walls, doors, etc.) | 1 giant mesh (or a few large chunks) |
| **Tiling unit** | Whole feature (by centroid) | Individual triangles (split at bounds) |
| **Refinement** | `ADD` (parent + children visible) | `REPLACE` (children replace parent) |
| **LOD** | None needed (features are small) | Critical (100M+ tri must simplify) |
| **Metadata** | Rich BIM properties per element | Minimal (maybe source tile ID) |
| **Textures** | Small, embedded per tile | Large atlases, shared across tiles |
| **Memory model** | Load all features, assign to tiles | Must stream; files can be 16GB+ |
| **Typical input** | 10K features, 2M triangles | 1 feature, 50M-500M triangles |

**Evidence from testing:** A 169M triangle photogrammetry mesh through BIM-Tiler produced 8 tiles x 4GB each = 32GB output (2x worse than input) because the entire mesh was assigned to every tile it touched with no splitting or simplification. See `docs/LIMITATIONS.md`.

Trying to make one codebase serve both use cases would create a mess of conditional logic. A clean fork shares the ~30% of code that's genuinely common (transforms, projection, GLB writing primitives, texture compression) and replaces the rest.

---

## 2. Input Format Analysis

### 2.1 What Each Software Exports

#### Pix4D (Pix4Dmapper / Pix4Dmatic)

| Property | Details |
|----------|---------|
| **Mesh formats** | OBJ, PLY, FBX, DXF, PDF |
| **Pix4Dmatic extras** | Direct Cesium 3D Tiles export, SLPK |
| **Textures** | Single JPG (default) or tiled textures (multiple JPGs, OBJ only) |
| **Tiled texture sizes** | Up to 65536x65536 or 131072x131072, split into multiple JPGs |
| **Coordinate system** | Local coordinates centered around project origin (OBJ/PLY are NOT georeferenced) |
| **Georef metadata** | `project_name_offset.xyz` file with 3 offset values (no CRS info) |
| **Georeferenced formats** | Only OSGB and SLPK (Pix4Dmapper), 3D Tiles and SLPK (Pix4Dmatic) |
| **OBJ structure** | Single OBJ + single MTL + 1-N texture JPGs |

**Key takeaway:** When users export OBJ from Pix4D, they get local coordinates. The `offset.xyz` file contains the shift needed to get back to project coordinates, but does NOT include the CRS. Users must supply the EPSG code separately.

#### Agisoft Metashape

| Property | Details |
|----------|---------|
| **Mesh formats** | OBJ, PLY, FBX, 3DS, COLLADA, glTF/GLB, OSGB, KMZ, and more |
| **Tiled export** | "Block Model" mode splits into spatial blocks |
| **Block settings** | Default 256px tile size, 20K faces per megapixel |
| **Textures** | Single atlas or multiple textures depending on settings |
| **Typical atlas** | 4096x4096 or 8192x8192 JPEG |
| **Coordinate system** | Supports UTM, WGS84, and arbitrary EPSG codes |
| **Georef metadata** | Optional `metadata.xml` on export, manual offset specification |
| **Offset handling** | "Load default" subtracts shift values from vertices to avoid large coords |
| **OBJ structure** | Single OBJ or tiled multi-OBJ with per-block textures |

**Key takeaway:** Metashape is the most flexible. Users can export a single OBJ with large coordinates (needs offset.xyz or manual offset), or tiled OBJ with block-local coordinates. The `metadata.xml` is the key georef source.

#### RealityCapture (Trimble/Capturing Reality)

| Property | Details |
|----------|---------|
| **Mesh formats** | OBJ, FBX, GLB, PLY, DAE, STL, 3MF, USD/USDZ, ABC |
| **Direct 3D Tiles** | Yes, built-in LOD export with hierarchical detail |
| **Textures** | WebP, JPG, or PNG; per-LOD level with `_LODn` suffix |
| **LOD export** | Iterative simplification with configurable target % |
| **Recommended settings** | 50% relative target triangle % for web streaming |
| **Coordinate system** | Project CRS or local |
| **OBJ structure** | Single OBJ + MTL + texture(s) |

**Key takeaway:** RealityCapture can already export 3D Tiles directly. Users would use Photo-Tiler when they want more control, different compression, or when they have legacy OBJ exports.

#### DJI Terra

| Property | Details |
|----------|---------|
| **Texture mesh** | OBJ, PLY, I3S |
| **LOD formats** | B3DM (3D Tiles), OSGB, S3MB |
| **OBJ contents** | `.obj` + `.mtl` + `.jpg` files |
| **Georef metadata** | `metadata.xml` generated with OBJ, OSGB, and PLY exports |
| **Coordinate system** | Project CRS stored in metadata.xml |

**Key takeaway:** DJI Terra already outputs B3DM/3D Tiles and OSGB with LOD. Photo-Tiler would be used for OBJ exports that users want to re-tile with different settings.

#### OpenDroneMap / WebODM

| Property | Details |
|----------|---------|
| **Output file** | `odm_textured_model_geo.obj` in `odm_texturing/` |
| **Textures** | Multiple texture atlas JPGs (typically 4-16 files) |
| **Coordinate system** | Auto-selected UTM zone, stored as UTM minus offset |
| **Offset file** | Offset stored in model metadata |
| **3D Tiles** | Via Obj2Tiles integration in WebODM |

### 2.2 Universal Input Contract

Based on the analysis above, Photo-Tiler must handle these input patterns:

```
Pattern A: Single OBJ + Single Texture
  model.obj + model.mtl + texture.jpg
  (Pix4D default, RealityCapture, DJI Terra simple)

Pattern B: Single OBJ + Multiple Textures
  model.obj + model.mtl + texture_0.jpg + texture_1.jpg + ... + texture_N.jpg
  (Pix4D tiled textures, OpenDroneMap, Agisoft single export)

Pattern C: Multiple OBJs (Tiled Export)
  block_0_0/model.obj + block_0_0/model.mtl + block_0_0/texture.jpg
  block_0_1/model.obj + block_0_1/model.mtl + block_0_1/texture.jpg
  ...
  (Agisoft Block Model export)

Pattern D: Single glTF/GLB
  model.glb (embedded textures)
  model.gltf + bin + textures/
  (Agisoft GLB export, RealityCapture GLB)

Pattern E: PLY (no textures or vertex colors)
  model.ply
  (Any software, often vertex-colored)
```

### 2.3 Georeferencing Metadata Sources

| Source | Format | Contains |
|--------|--------|----------|
| `offset.xyz` (Pix4D) | 3 floats, one per line | X, Y, Z offset (no CRS) |
| `metadata.xml` (Agisoft, DJI Terra) | XML | CRS (EPSG or WKT), offset, transform matrix |
| `.prj` file (various) | WKT | Coordinate system definition only |
| CLI flags | User-supplied | EPSG + easting/northing/elevation |
| glTF extras | JSON in glTF | Sometimes contains geo metadata |

---

## 3. Architecture Overview

### 3.1 Pipeline Comparison

```
BIM-Tiler Pipeline:
  Ingest (APS/IFC/glTF/OBJ) → Extract Metadata → Transform → Octree (by feature) → GLB+Metadata → tileset.json

Photo-Tiler Pipeline:
  Ingest (OBJ/glTF/PLY) → Parse Georef → Scan Bounds → Spatial Index
       → Split Triangles by Octant → Simplify per LOD → Write GLB → tileset.json
```

### 3.2 High-Level Data Flow

```
                    ┌──────────────────────────────────────────────────┐
                    │                  PHOTO-TILER                      │
                    │                                                    │
  Input files ──────┤  1. INGEST                                        │
  (OBJ/glTF/PLY)   │     ├─ Parse geometry (streaming for large files) │
  + offset.xyz      │     ├─ Parse georef metadata                      │
  + metadata.xml    │     └─ Detect/apply units                         │
                    │                                                    │
                    │  2. SCAN & INDEX                                   │
                    │     ├─ First pass: compute global bounds           │
                    │     ├─ Build spatial index (triangle → cell)       │
                    │     └─ Determine octree depth from triangle count  │
                    │                                                    │
                    │  3. SPLIT & ASSIGN                                 │
                    │     ├─ For each leaf octant: collect triangles      │
                    │     ├─ Clip triangles at octant boundaries         │
                    │     ├─ Interpolate UVs/normals at clip points      │
                    │     └─ Build per-tile vertex/index buffers         │
                    │                                                    │
                    │  4. SIMPLIFY (LOD)                                 │
                    │     ├─ Leaf tiles: full resolution                  │
                    │     ├─ Parent tiles: simplified union of children   │
                    │     ├─ Root tile: heavily simplified (~1% of input) │
                    │     └─ Compute geometricError per level             │
                    │                                                    │
                    │  5. TEXTURE                                        │
                    │     ├─ Repack UV atlas per tile (only used regions) │
                    │     ├─ Compress to WebP or KTX2                    │
                    │     ├─ Option: shared external textures             │
                    │     └─ Option: embedded per-tile textures           │
                    │                                                    │
                    │  6. OUTPUT                                          │
                    │     ├─ Write GLB per tile (Draco compressed)        │
                    │     ├─ Compute ECEF root transform                  │
                    │     ├─ Write tileset.json (REPLACE refinement)      │
                    │     └─ Optional: implicit tiling with subtrees      │
                    │                                                    │
                    └──────────────────────────────────────────────────┘

Output:
  tileset/
  ├── tileset.json
  ├── textures/           (if using external textures)
  │   ├── atlas_0.webp
  │   └── atlas_1.webp
  └── tiles/
      ├── root.glb        (heavily simplified, loads first)
      ├── 0/tile.glb      (medium detail, replaces root)
      ├── 0/0/tile.glb    (more detail, replaces parent)
      └── ...             (leaf tiles at full resolution)
```

### 3.3 Key Architecture Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Spatial structure | Octree (with optional quadtree for terrain) | Proven for 3D data; quadtree better for relatively flat photogrammetry |
| Refinement | `REPLACE` | Parent is coarse; children replace it at close range. Prevents memory buildup |
| LOD generation | meshoptimizer (WASM) | Best quality/speed ratio; already in BIM-Tiler's dependencies |
| Triangle splitting | Sutherland-Hodgman clipping against axis-aligned planes | Standard algorithm, handles UV/normal interpolation |
| Texture per tile | Cropped atlas (embed) or shared external | Embedded for simplicity; external for large texture sets |
| Memory model | Two-pass streaming | Pass 1: scan bounds + build index. Pass 2: load per-octant |
| Seam handling | Skirts (optional) | Add thin vertical geometry at tile edges to hide cracks |

---

## 4. BIM-Tiler Code Audit: Keep vs Remove vs Modify

### 4.1 Files to KEEP As-Is (Reusable Verbatim)

These modules have zero BIM-specific logic:

| File | Lines | Purpose | Why it's reusable |
|------|-------|---------|-------------------|
| `src/transform/matrix.ts` | 175 | 4x4 matrix ops (gl-matrix) | Pure math, no domain logic |
| `src/transform/ecef.ts` | ~120 | ENU-to-ECEF transform | Standard geodetic math |
| `src/transform/projection.ts` | ~150 | EPSG projection via proj4 | Standard projection |
| `src/utils/logger.ts` | ~80 | Pino logger setup | Infrastructure |
| `src/utils/errors.ts` | ~100 | Custom error classes | Infrastructure |
| `src/utils/index.ts` | ~20 | Utils barrel export | Infrastructure |
| `src/types/draco3dgltf.d.ts` | ~30 | Draco type declarations | Type-only |

**Total reusable as-is: ~675 lines across 7 files**

### 4.2 Files to KEEP with Modifications

| File | Lines | What to Keep | What to Change |
|------|-------|-------------|----------------|
| `src/transform/coordinates.ts` | 388 | `computeTransforms()`, `centerGeometryAtOrigin()`, Y-up→Z-up, ECEF pipeline | Remove BIM metadata references; add optional quadtree Z-flattening |
| `src/transform/index.ts` | ~50 | Barrel exports | Update exports |
| `src/ingestion/obj-converter.ts` | ~200 | OBJ→glTF conversion via obj2gltf | Remove APS-specific options; add streaming mode for large files |
| `src/ingestion/feature.ts` | ~150 | `boundsCenter()`, `boundsHalfSize()`, `boundsDiagonal()` | Remove BIM metadata builders; simplify Feature creation |
| `src/ingestion/gltf-loader.ts` | ~400 | glTF/GLB loading, `readLargeFile()` chunked reader | Remove BIM node name parsing; add vertex attribute extraction |
| `src/ingestion/units.ts` | 457 | Unit scale factors, conversion logic | Remove APS property detection; keep manual unit specification and scale math |
| `src/tiling/glb-writer.ts` | 731 | GLB creation via glTF-transform, Draco compression, PBR materials | Remove `EXT_structural_metadata` and `EXT_mesh_features` (no BIM metadata); add LOD level parameter |
| `src/tiling/texture-optimizer.ts` | 662 | WebP/KTX2 compression, per-tile texture filtering, sharp integration | Keep as-is for compression; add UV atlas cropping/repacking |
| `src/tiling/tileset-writer.ts` | 412 | `boundsToBox()`, `validateTileset()`, tileset.json structure | Change `refine: 'ADD'` → `'REPLACE'`; remove BIM schema; adjust geometric error calculation |
| `src/tiling/implicit-tiling.ts` | 494 | Morton Z-order, subtree availability, implicit tileset structure | Already built but unused in BIM-Tiler; activate for Photo-Tiler |
| `src/tiling/octree.ts` | 368 | Octree structure, traversal, bounds subdivision | Replace feature-centroid assignment with triangle splitting; add quadtree option |
| `src/types.ts` | 780 | `Feature`, `BoundingBox`, `PBRMaterial`, `TileNode`, `TilingOptions` | Remove all BIM types (`FeatureProperties`, `BIMMetadata`, `APSConfig`, `ExtractedGeoreference`, etc.); add `PhotoTilerConfig`, `SplitResult`, `LODLevel` |
| `src/pipeline.ts` | 573 | Pipeline orchestration pattern | Rewrite for Photo-Tiler pipeline (scan → split → simplify → write) |
| `src/cli.ts` | 664 | Commander.js CLI pattern | Rewrite options for photogrammetry inputs |
| `src/index.ts` | ~80 | Public API exports | Update exports |

**Total to modify: ~5,919 lines across 15 files**

### 4.3 Files to DELETE (BIM-Specific, No Value for Fork)

| File | Lines | Why Delete |
|------|-------|------------|
| `src/ingestion/aps-client.ts` | 264 | Autodesk Platform Services integration |
| `src/ingestion/aps-obj-client.ts` | ~300 | APS OBJ export workflow |
| `src/ingestion/aps/auth.ts` | ~150 | APS OAuth authentication |
| `src/ingestion/aps/translation.ts` | ~200 | APS SVF2/OBJ translation management |
| `src/ingestion/aps/aec-data.ts` | 307 | AEC Model Data extraction (Revit-specific) |
| `src/ingestion/aps/georeference.ts` | 560 | APS georeferencing (Survey/Base Points) |
| `src/ingestion/aps/common.ts` | ~50 | APS shared types |
| `src/ingestion/aps/index.ts` | ~30 | APS barrel export |
| `src/ingestion/ifc-loader.ts` | ~400 | IFC file parsing via web-ifc |
| `src/ingestion/metadata-parser.ts` | ~200 | BIM metadata JSON parsing |
| `src/tiling/exterior-classifier.ts` | ~300 | BIM exterior/interior classification |
| `src/tiling/exterior-priority-tiling.ts` | 484 | BIM exterior-first loading strategy |

**Total to delete: ~3,245 lines across 12 files**

### 4.4 Summary

```
BIM-Tiler total:     ~12,500 lines across 36 files
Keep as-is:             ~675 lines across  7 files (5%)
Keep with mods:       ~5,919 lines across 15 files (47%)
Delete:               ~3,245 lines across 12 files (26%)
New code needed:      ~2,500-4,000 lines across 6-8 files (new)
```

---

## 5. New Modules to Build

### 5.1 `src/tiling/triangle-splitter.ts` (~400-600 lines)

**Purpose:** Split triangles that cross octree/quadtree node boundaries.

**Algorithm: Sutherland-Hodgman Polygon Clipping**

```
For each triangle in the mesh:
  1. Compute triangle AABB
  2. Test against all octant bounds
  3. If triangle fits entirely in one octant → assign directly
  4. If triangle spans multiple octants:
     a. For each axis-aligned splitting plane (X, Y, Z midplanes):
        - Classify each vertex as inside/outside
        - Clip polygon against plane using Sutherland-Hodgman
        - Interpolate UV coordinates at new vertices: UV_new = lerp(UV_a, UV_b, t)
        - Interpolate normals at new vertices: N_new = normalize(lerp(N_a, N_b, t))
     b. Triangulate resulting polygons (fan triangulation from first vertex)
     c. Assign sub-triangles to respective octants
```

**Key functions:**
```typescript
clipTriangleByPlane(tri: Triangle, plane: AxisPlane): { inside: Triangle[], outside: Triangle[] }
assignTrianglesToOctants(mesh: IndexedMesh, bounds: BoundingBox): Map<number, IndexedMesh>
interpolateVertexAttributes(v1: Vertex, v2: Vertex, t: number): Vertex
```

**Critical detail:** When a triangle is split, the new vertices at the boundary must have correctly interpolated UVs. If UVs are wrong, there will be visible texture seams. Use linear interpolation based on the parametric `t` where the edge crosses the plane.

### 5.2 `src/tiling/mesh-simplifier.ts` (~300-400 lines)

**Purpose:** Wrapper around meshoptimizer WASM for mesh decimation.

**Interface:**
```typescript
interface SimplifyOptions {
  targetRatio: number;       // 0.0-1.0, fraction of triangles to keep
  targetError?: number;      // max geometric error (default: 0.01)
  lockBorder?: boolean;      // prevent simplifying boundary edges (for seam matching)
  preserveAttributes?: boolean; // try to preserve UV seams
}

async function simplifyMesh(
  positions: Float32Array,     // xyz interleaved
  indices: Uint32Array,
  normals?: Float32Array,
  uvs?: Float32Array,
  options: SimplifyOptions
): Promise<SimplifiedMesh>
```

**LOD configuration:**
```typescript
const DEFAULT_LOD_CONFIG: LODLevel[] = [
  { level: 0, ratio: 0.005, geometricError: 2000 },  // Root: 0.5% of triangles
  { level: 1, ratio: 0.02,  geometricError: 500  },  // 2%
  { level: 2, ratio: 0.05,  geometricError: 200  },  // 5%
  { level: 3, ratio: 0.15,  geometricError: 80   },  // 15%
  { level: 4, ratio: 0.40,  geometricError: 30   },  // 40%
  { level: 5, ratio: 1.00,  geometricError: 0    },  // Leaf: full resolution
];
```

**LOD generation strategy:** Bottom-up. Start with leaf tiles at full resolution, then generate each parent by merging children's geometry and simplifying.

### 5.3 `src/tiling/texture-repacker.ts` (~400-500 lines)

**Purpose:** For each tile, extract only the UV-referenced region of the source texture atlas and repack into a minimal per-tile texture.

**Why needed:** A photogrammetry model might have a 8192x8192 texture atlas, but a single tile only uses a small rectangular region. Embedding the full atlas in every tile wastes bandwidth.

**Algorithm:**
```
For each tile:
  1. Collect all UV coordinates of triangles in this tile
  2. Compute UV bounding box (min_u, min_v, max_u, max_v)
  3. Crop source texture to that region (with padding for filtering)
  4. Remap tile UVs: u_new = (u - min_u) / (max_u - min_u)
  5. Compress cropped texture to WebP/KTX2
  6. Embed in GLB or write as external file
```

**For multi-texture inputs:** Each material group references a different texture. Keep material group assignments through the splitting process. Pack each tile's texture set independently.

**Existing code to leverage:** `src/tiling/texture-optimizer.ts` already has sharp-based WebP/KTX2 compression, per-tile texture filtering, and deduplication. Extend it with UV-based atlas cropping.

### 5.4 `src/ingestion/streaming-obj-parser.ts` (~500-700 lines)

**Purpose:** Parse OBJ files in a streaming fashion without loading the entire file into memory.

**Why needed:** Photogrammetry OBJ files can be 5-20GB. Loading into memory requires 2-3x the file size (~40-60GB RAM). Most machines can't handle this.

**Two-pass architecture:**

```
Pass 1: SCAN (read-only, O(1) memory)
  - Count vertices, faces, texture coords, normals
  - Compute global bounding box
  - Record byte offsets for each material group
  - Parse MTL file for texture references
  - Result: MeshManifest { bounds, vertexCount, faceCount, materialGroups[], byteOffsets[] }

Pass 2: LOAD BY REGION (per-octant, bounded memory)
  - Given an octant bounding box:
    a. Seek to relevant byte offsets
    b. Load only vertices within/near the bounds
    c. Load faces that reference those vertices
    d. Build IndexedMesh for that octant
  - Result: IndexedMesh for one tile
```

**Implementation detail:** Use Node.js `fs.createReadStream()` with `readline` for Pass 1. For Pass 2, use `fs.read()` with position offsets for random access. The spatial index from Pass 1 maps face byte-offset ranges to spatial regions.

**For multi-OBJ inputs (Agisoft Block Model):** Each block is a separate OBJ. Load block manifest first, assign blocks to octants based on block bounds, then process each block independently.

### 5.5 `src/ingestion/georef-parser.ts` (~200-300 lines)

**Purpose:** Parse georeferencing metadata from photogrammetry software output files.

**Supported formats:**

```typescript
// Pix4D offset file (3 lines, one number each)
parseOffsetXYZ(filePath: string): { x: number; y: number; z: number }

// Agisoft/DJI Terra metadata.xml
parseMetadataXML(filePath: string): {
  crs?: { epsg?: number; wkt?: string };
  transform?: number[];  // 4x4 matrix
  offset?: { x: number; y: number; z: number };
}

// PRJ file (WKT coordinate system)
parsePRJ(filePath: string): { wkt: string; epsg?: number }

// Auto-detect: look for offset.xyz, metadata.xml, .prj in input directory
autoDetectGeoref(inputDir: string): GeorefMetadata | null
```

**Auto-detection order:**
1. `metadata.xml` in same directory as input (highest priority, has CRS)
2. `<model_name>_offset.xyz` (Pix4D style, needs user EPSG)
3. `*.prj` file (CRS only, offset from CLI or file coordinates)
4. Fall back to CLI flags (`--epsg`, `--easting`, `--northing`, `--elevation`)

### 5.6 `src/ingestion/ply-loader.ts` (~200-300 lines)

**Purpose:** Load PLY files (common photogrammetry export format, especially for vertex-colored models).

**Details:**
- Parse ASCII and binary PLY formats
- Extract vertex positions, normals, vertex colors
- Extract face indices
- No UV/texture support (PLY rarely has it; models use vertex colors instead)
- Convert vertex colors to PBR material (base color factor per vertex)

### 5.7 `src/tiling/quadtree.ts` (~300-400 lines) - Optional

**Purpose:** Alternative to octree for relatively flat photogrammetry models (aerial/drone captures).

**Why:** Aerial photogrammetry models are essentially 2.5D surfaces. An octree wastes depth splitting on geometry that has minimal Z variation. A quadtree (XY split only) produces more efficient tiles with fewer empty nodes.

**Decision logic:**
```typescript
function chooseSpatialStructure(bounds: BoundingBox): 'octree' | 'quadtree' {
  const zRange = bounds.max[2] - bounds.min[2];
  const xyRange = Math.max(
    bounds.max[0] - bounds.min[0],
    bounds.max[1] - bounds.min[1]
  );
  // If Z extent is < 20% of XY extent, quadtree is more efficient
  return (zRange / xyRange) < 0.2 ? 'quadtree' : 'octree';
}
```

---

## 6. Phase-by-Phase Implementation

### Phase 1: Fork & Strip (Day 1-2)

**Goal:** Clean fork with BIM code removed, builds and runs.

**Steps:**
1. Fork the repository
2. Rename package: `@sgr/bim-tiler` → `@sgr/photo-tiler`
3. Update `bin` in package.json: `bim-tiler` → `photo-tiler`
4. Delete all files listed in Section 4.3 (12 files, ~3,245 lines)
5. Remove imports/references to deleted files in remaining code
6. Strip BIM types from `src/types.ts`:
   - Remove: `FeatureProperties` (BIM fields), `BIMMetadata`, `APSConfig`, `ExtractedGeoreference`, `ExtractedBasePoint`, `AECModelData`, `ObjDerivative`, `ObjDownloadResult`, `PropertySet`
   - Keep: `Feature` (simplify properties), `BoundingBox`, `PBRMaterial`, `MaterialLibrary`, `TileNode`, `TilingOptions`, `TilesetJson`, `TileJson`
   - Add: `PhotoTilerConfig`, `LODLevel`, `SplitResult`, `MeshManifest`
7. Stub out `src/cli.ts` with new options (see Section 7)
8. Stub out `src/pipeline.ts` with new pipeline skeleton
9. Remove dependencies: `web-ifc`, `form-data` (if only APS)
10. Verify `npm run build` succeeds

### Phase 2: Geometry Splitting (Day 3-7)

**Goal:** Triangle-level assignment to octree nodes, with splitting at boundaries.

**Steps:**
1. Create `src/tiling/triangle-splitter.ts`
   - Implement Sutherland-Hodgman clipping for axis-aligned planes
   - Handle UV and normal interpolation
   - Handle degenerate cases (triangle exactly on plane, zero-area triangles)
2. Modify `src/tiling/octree.ts`
   - Replace `featureCentroidInBounds()` with `assignTrianglesToOctants()`
   - Each leaf node gets a subset of triangles, not whole features
   - Add triangle count threshold for leaf nodes (e.g., 50K-200K triangles per leaf)
3. Update `Feature` type to work with triangle subsets
4. Test with a small OBJ (< 1M triangles) to verify correct splitting
5. Validate: each triangle should appear in exactly one leaf tile; boundary triangles should be clipped

### Phase 3: LOD / Mesh Simplification (Day 8-12)

**Goal:** Parent tiles contain simplified geometry; viewers load coarse → fine.

**Steps:**
1. Create `src/tiling/mesh-simplifier.ts`
   - Wrap meshoptimizer WASM (`MeshoptSimplifier.simplify()`)
   - Implement `simplifyMesh()` with configurable target ratio and error
   - Handle edge cases: mesh too small to simplify, degenerate faces after simplification
2. Implement bottom-up LOD generation:
   - Leaf tiles: full resolution (from Phase 2 splitting)
   - Each parent: merge children's geometry → simplify to target ratio
   - Root tile: most aggressive simplification (~0.5-2% of original)
3. Compute `geometricError` values:
   - Leaf: 0 (full detail)
   - Each parent: based on simplification error metric from meshoptimizer
   - Root: diagonal of bounding box / 2
4. Modify `src/tiling/tileset-writer.ts`:
   - Change `refine: 'ADD'` → `refine: 'REPLACE'`
   - Wire up computed `geometricError` values
5. Modify `src/tiling/glb-writer.ts`:
   - Remove `EXT_structural_metadata` and `EXT_mesh_features` extensions
   - Accept simplified geometry as input
   - Keep Draco compression
6. Test: load output in CesiumJS, verify LOD transitions (root loads first, zoom reveals detail)

### Phase 4: Texture Handling (Day 13-17)

**Goal:** Per-tile textures that reference only the UV regions needed.

**Steps:**
1. Create `src/tiling/texture-repacker.ts`
   - Compute UV bounding box per tile
   - Crop source texture atlas using sharp
   - Remap UVs in tile geometry to match cropped texture
   - Handle multi-texture materials (per-material-group cropping)
2. Extend `src/tiling/texture-optimizer.ts`
   - Add atlas cropping mode (in addition to existing per-tile filtering)
   - Add option for external texture references (glTF `image.uri` instead of `image.bufferView`)
3. Implement two texture modes:
   - **Embedded** (default): Cropped+compressed texture embedded in each GLB
   - **External**: Shared texture files in `textures/` directory, GLBs reference via relative URI
4. Handle multi-texture inputs:
   - Parse MTL file to get texture filenames per material group
   - Track material group through triangle splitting
   - Per-tile: only include textures that tile actually references
5. Test with Pix4D (single texture) and OpenDroneMap (multiple textures) outputs

### Phase 5: Georeferencing (Day 18-20)

**Goal:** Correctly geolocate photogrammetry models in ECEF coordinates.

**Steps:**
1. Create `src/ingestion/georef-parser.ts`
   - Implement `parseOffsetXYZ()` for Pix4D
   - Implement `parseMetadataXML()` for Agisoft/DJI Terra
   - Implement `parsePRJ()` for WKT CRS files
   - Implement `autoDetectGeoref()` to scan input directory
2. Integrate with existing transform pipeline:
   - Apply offset to geometry (shift vertices by offset values)
   - Use EPSG code with proj4 for projection to WGS84
   - Compute ENU-to-ECEF root transform (reuse `src/transform/ecef.ts`)
3. Handle coordinate quirks:
   - Pix4D: offset.xyz has no CRS → require user `--epsg`
   - Agisoft: metadata.xml may have full CRS → auto-detect EPSG
   - Large coordinates: center at local origin first (reuse `centerGeometryAtOrigin()`)
4. Add `--show-georef` mode: display detected metadata without processing

### Phase 6: Streaming & Memory (Day 21-25)

**Goal:** Process 10GB+ files without running out of memory.

**Steps:**
1. Create `src/ingestion/streaming-obj-parser.ts`
   - Implement Pass 1 (scan: bounds, counts, byte offsets)
   - Implement Pass 2 (load by region)
   - Target: peak memory < 2x single tile size, not 2x input file size
2. Modify pipeline for streaming:
   - Don't load entire mesh into memory
   - Process one octant at a time: load region → split → simplify → write GLB → free
   - Use immediate tile writing (write GLB to disk, release memory)
3. Handle multi-OBJ inputs (Agisoft Block Model):
   - Scan all blocks, compute global bounds from block-level bounds
   - Assign blocks to octants (blocks are pre-split spatially)
   - Load one block at a time
4. Add progress reporting:
   - Pass 1: "Scanning... X vertices, Y faces"
   - Pass 2: "Processing tile 3/47..."
   - Memory usage reporting in verbose mode

### Phase 7: PLY Support & Polish (Day 26-28)

**Goal:** Complete input format coverage and polish.

**Steps:**
1. Create `src/ingestion/ply-loader.ts`
   - ASCII and binary little-endian PLY parsing
   - Vertex colors → PBR base color
2. Add optional quadtree mode for terrain-like meshes
3. Activate implicit tiling (already built in BIM-Tiler, just unused)
4. Add `--validate` flag using 3d-tiles-validator
5. Add `--dry-run` flag for checking input without processing
6. Final CLI polish, help text, examples

---

## 7. CLI Design

```
photo-tiler [options]

Input:
  -i, --input <path>          Input file or directory (OBJ, glTF, GLB, PLY)
  --input-dir <dir>           Directory containing tiled OBJ blocks (Agisoft Block Model)

Units:
  --units <unit>              Input units: mm, cm, m, ft, in (default: m)

Georeferencing:
  --epsg <code>               EPSG code for coordinate system
  --easting <m>               Origin easting (meters)
  --northing <m>              Origin northing (meters)
  --elevation <m>             Origin elevation (meters, default: 0)
  --offset-file <path>        Path to offset.xyz file (auto-detected if in input dir)
  --metadata-xml <path>       Path to metadata.xml (auto-detected if in input dir)
  --show-georef               Display detected georeferencing data and exit

Tiling:
  --max-depth <n>             Maximum octree depth (default: 6)
  --max-triangles <n>         Max triangles per leaf tile (default: 100000)
  --spatial-structure <type>  octree or quadtree (default: auto)

LOD:
  --lod-levels <n>            Number of LOD levels (default: auto from depth)
  --simplification-ratio <r>  Root tile simplification target, 0.0-1.0 (default: 0.005)

Textures:
  --include-textures          Include textures in output (default: true)
  --no-textures               Strip textures entirely
  --texture-format <fmt>      webp, ktx2, original (default: webp)
  --texture-quality <n>       Compression quality 0-100 (default: 85)
  --texture-max-size <n>      Max texture dimension per tile (default: 2048)
  --external-textures         Use shared external texture files instead of embedded

Compression:
  --no-draco                  Disable Draco mesh compression
  --draco-level <n>           Draco compression level 1-10 (default: 7)

Output:
  -o, --output <dir>          Output directory (required)
  --implicit-tiling           Use 3D Tiles 1.1 implicit tiling
  --validate                  Run 3d-tiles-validator after conversion

Other:
  --dry-run                   Scan input and report stats without processing
  -v, --verbose               Verbose logging
```

**Usage examples:**

```bash
# Simple: single OBJ from Pix4D
photo-tiler -i model.obj -o ./tileset --epsg 32636 --offset-file offset.xyz

# Agisoft with auto-detected metadata.xml
photo-tiler -i model.obj -o ./tileset

# Agisoft tiled export (Block Model)
photo-tiler --input-dir ./blocks/ -o ./tileset --epsg 32633

# Large model with streaming (automatic for files > 2GB)
photo-tiler -i huge_model.obj -o ./tileset --units m --epsg 32636

# GLB from RealityCapture
photo-tiler -i model.glb -o ./tileset --epsg 32636 --easting 500000 --northing 4000000

# Vertex-colored PLY
photo-tiler -i model.ply -o ./tileset --no-textures --epsg 32636

# External textures for multi-texture models
photo-tiler -i model.obj -o ./tileset --external-textures

# Dry run to check input
photo-tiler -i model.obj --dry-run
```

---

## 8. Output Structure

### 8.1 Standard Output (Embedded Textures)

```
tileset/
├── tileset.json              # 3D Tiles 1.1 with REPLACE refinement
└── tiles/
    ├── root.glb              # LOD 0: ~0.5% of triangles (loads first from distance)
    ├── 0/
    │   └── tile.glb          # LOD 1: ~2% of triangles (replaces root when closer)
    ├── 0/0/
    │   └── tile.glb          # LOD 2: ~5% of this octant's triangles
    ├── 0/0/3/
    │   └── tile.glb          # LOD 3: ~15% of this octant's triangles
    └── ...                   # Leaf tiles: 100% resolution of their spatial region
```

### 8.2 External Textures Output

```
tileset/
├── tileset.json
├── textures/
│   ├── atlas_0.webp          # Shared texture (referenced by multiple tiles)
│   ├── atlas_1.webp
│   └── atlas_2.webp
└── tiles/
    ├── root.glb              # References ../textures/atlas_0.webp
    ├── 0/tile.glb
    └── ...
```

### 8.3 Implicit Tiling Output

```
tileset/
├── tileset.json              # Contains implicitTiling descriptor
├── subtrees/
│   └── 0/0/0/0.subtree      # Availability bitstream
└── tiles/
    └── {level}/{x}/{y}/{z}.glb
```

### 8.4 tileset.json Structure

```json
{
  "asset": {
    "version": "1.1",
    "generator": "Photo-Tiler",
    "tilesetVersion": "1.0.0"
  },
  "geometricError": 2000,
  "root": {
    "transform": [ /* 16-element ECEF transform */ ],
    "boundingVolume": {
      "box": [ /* 12-element oriented bounding box */ ]
    },
    "geometricError": 2000,
    "refine": "REPLACE",
    "content": {
      "uri": "tiles/root.glb"
    },
    "children": [
      {
        "boundingVolume": { "box": [...] },
        "geometricError": 500,
        "refine": "REPLACE",
        "content": { "uri": "tiles/0/tile.glb" },
        "children": [...]
      }
    ]
  }
}
```

---

## 9. Georeferencing Strategy

### 9.1 Coordinate Transform Pipeline

```
Source geometry (local coords, source units, Y-up or Z-up)
    │
    ├── If offset.xyz/metadata.xml found:
    │   └── Add offset to vertex positions
    │
    ├── Unit Conversion (source units → meters)
    │
    ├── Axis Conversion (if Y-up → Z-up / ENU)
    │   (OBJ from obj2gltf is Y-up; PLY is usually Z-up)
    │
    ├── Center at Local Origin
    │   └── Subtract centroid from all vertices
    │   └── Store centroid offset for root transform
    │
    ├── Projection (EPSG → WGS84 lat/lon)
    │   └── proj4.forward(centroid + offset) → (lon, lat, h)
    │
    └── ENU-to-ECEF Root Transform
        └── 4x4 matrix stored in tileset.json root.transform
        └── Positions model on the globe
```

### 9.2 Axis Convention Summary

| Source | Native Axis | Conversion Needed |
|--------|-------------|-------------------|
| OBJ (via obj2gltf) | Y-up (glTF convention) | Rotate +90 around X → Z-up |
| OBJ (raw, photogrammetry) | Usually Z-up | None (already ENU) |
| glTF/GLB | Y-up (spec requires it) | Rotate +90 around X → Z-up |
| PLY | Typically Z-up | None (already ENU) |

**Important:** `obj2gltf` converts OBJ (which is typically Z-up from photogrammetry tools) to glTF Y-up convention. So when we load the resulting glTF, we need to convert Y-up → Z-up again. This is the same transform already in BIM-Tiler's `createYUpToZUp()`.

### 9.3 When Georeferencing Is Not Provided

If no EPSG code or offset is given, Photo-Tiler should still work:
- Geometry is centered at (0,0,0)
- No root transform in tileset.json (local coordinates)
- Tileset can still be viewed in CesiumJS by manually positioning it
- Print a warning: "No georeferencing data found. Output will use local coordinates."

---

## 10. Texture Strategy

### 10.1 Decision Matrix

| Input Pattern | Recommended Strategy | Rationale |
|--------------|---------------------|-----------|
| 1 texture, < 4K | Embed per tile (cropped) | Small enough to duplicate |
| 1 texture, 4K-8K | Embed per tile (cropped + downscaled) | Crop to UV region, cap at `--texture-max-size` |
| Multiple textures, total < 20MB | Embed per tile (filtered) | BIM-Tiler's existing per-tile filtering works |
| Multiple textures, total > 20MB | External shared | Avoids massive duplication |
| No textures (vertex colors) | Vertex colors in GLB | No texture processing needed |

### 10.2 UV Atlas Cropping Detail

When a tile uses only a small region of a large texture atlas:

```
Source texture (8192x8192):
┌─────────────────────────────┐
│                             │
│       ┌─────┐               │
│       │TILE │               │  ← Tile's UV region
│       │ UVs │               │     is only 1024x512 of
│       └─────┘               │     the 8192x8192 atlas
│                             │
└─────────────────────────────┘

Cropped per-tile texture (1024x512):
┌─────┐
│TILE │  ← Only the relevant region
│ UVs │     + padding for bilinear filtering
└─────┘

UVs remapped: u_new = (u - u_min) / (u_max - u_min)
```

**Size reduction example:**
- Full atlas: 8192x8192 JPEG = ~16MB, WebP = ~4MB
- Cropped per tile (avg 1024x512): WebP = ~50KB
- 100 tiles with cropped textures: 5MB total (vs 400MB if embedding full atlas)

### 10.3 Multi-Texture Pipeline

```
Input: model.obj + model.mtl + tex_0.jpg + tex_1.jpg + tex_2.jpg

1. Parse MTL → material "ground" uses tex_0.jpg, "building" uses tex_1.jpg, etc.
2. When splitting triangles, carry material assignment with each triangle
3. Per tile: collect unique materials used in that tile
4. Per tile per material: crop texture to UV bounding box of that material's triangles
5. Result: each tile GLB has only the texture data it needs
```

---

## 11. Memory & Performance Strategy

### 11.1 Memory Budget Targets

| Input Size | Peak Memory Target | Strategy |
|-----------|-------------------|----------|
| < 500MB | 2x input | In-memory (current approach works) |
| 500MB - 2GB | 1.5x input | Chunked loading, immediate write |
| 2GB - 10GB | 4GB fixed cap | Streaming two-pass |
| > 10GB | 4GB fixed cap | Streaming two-pass + disk-based index |

### 11.2 Two-Pass Streaming Architecture

```
Pass 1: SCAN (streaming, ~100MB memory)
┌─────────────────────────────────────────────┐
│  Read OBJ line by line                       │
│  ├── Count vertices, faces                   │
│  ├── Track min/max XYZ → global bounds       │
│  ├── Record byte offset of each face block   │
│  ├── Parse MTL references                    │
│  └── Output: MeshManifest                    │
└─────────────────────────────────────────────┘

Between passes: Build octree structure from global bounds + triangle count

Pass 2: PROCESS (per-octant, ~500MB-2GB memory)
┌─────────────────────────────────────────────┐
│  For each leaf octant:                       │
│  ├── Seek to relevant byte offsets           │
│  ├── Load vertices + faces in this region    │
│  ├── Clip boundary triangles                 │
│  ├── Crop textures for this tile             │
│  ├── Create GLB (Draco compressed)           │
│  ├── Write to disk immediately               │
│  └── Free memory                             │
│                                              │
│  For each parent (bottom-up):                │
│  ├── Load children's geometry                │
│  ├── Merge + simplify                        │
│  ├── Write GLB                               │
│  └── Free memory                             │
└─────────────────────────────────────────────┘
```

### 11.3 Performance Estimates

Based on Obj2Tiles benchmarks and BIM-Tiler experience:

| Input | Triangles | Expected Time | Peak Memory |
|-------|-----------|---------------|-------------|
| 1M triangles | 1M | ~10s | ~500MB |
| 10M triangles | 10M | ~2min | ~2GB |
| 50M triangles | 50M | ~10min | ~4GB (streaming) |
| 169M triangles | 169M | ~30min | ~4GB (streaming) |

**Bottleneck analysis:**
- Geometry splitting: O(n) per triangle, fast
- Mesh simplification: O(n log n) per level, main bottleneck
- GLB writing + Draco: O(n) but with high constant factor
- Texture cropping: fast (sharp is optimized)

### 11.4 Parallelization Opportunities

| Operation | Parallelizable? | How |
|-----------|----------------|-----|
| Pass 1 scan | No (sequential file read) | Single thread |
| Triangle splitting | Yes (per octant) | Worker threads |
| Mesh simplification | Yes (per tile) | Worker threads |
| GLB writing | Yes (per tile) | Worker threads |
| Texture cropping | Yes (per tile) | Worker threads (sharp is async) |

Use `worker_threads` for CPU-bound work (simplification, splitting). Use async I/O for disk writes. Target: 4-8 concurrent workers.

---

## 12. Existing Tools Analysis & Differentiation

### 12.1 Competitive Landscape

| Tool | Language | LOD | Splitting | Streaming | Textures | License |
|------|----------|-----|-----------|-----------|----------|---------|
| **Cesium ion** | Closed | Yes | Yes | Yes | KTX2 | Commercial |
| **Obj2Tiles** | C# (.NET) | Yes | Yes | Partial | WebP/JPEG | AGPL-3.0 |
| **py3dtiles** | Python | Limited | No | No | No | Apache-2.0 |
| **objTo3d-tiles** | JS | No | No | No | Basic | MIT |
| **Photo-Tiler (this)** | TypeScript | Yes | Yes | Yes | WebP/KTX2 | MIT |

### 12.2 Differentiation from Obj2Tiles

Obj2Tiles (OpenDroneMap) is the closest open-source competitor. Key differences:

| Feature | Obj2Tiles | Photo-Tiler |
|---------|-----------|-------------|
| **Language** | C# (.NET 8) | TypeScript (Node.js) |
| **Input formats** | OBJ only | OBJ, glTF/GLB, PLY |
| **Georef auto-detect** | No | Yes (metadata.xml, offset.xyz, .prj) |
| **3D Tiles version** | 1.0 (b3dm) | 1.1 (GLB with extensions) |
| **Implicit tiling** | No | Yes (optional) |
| **Texture format** | JPEG/WebP | WebP/KTX2 |
| **Draco compression** | No | Yes |
| **JS ecosystem** | No | Yes (npm package, easy integration) |
| **Multi-OBJ input** | No | Yes (Agisoft Block Model) |

### 12.3 Differentiation from Cesium ion

Cesium ion is the gold standard but it's a paid cloud service:

| Feature | Cesium ion | Photo-Tiler |
|---------|-----------|-------------|
| **Deployment** | Cloud only | Local/self-hosted |
| **Cost** | $1-500+/month | Free (MIT) |
| **Privacy** | Data uploaded to Cesium servers | Data stays local |
| **Customization** | None | Full source access |
| **Quality** | Excellent | Target: good (80-90% of Cesium quality) |

---

## 13. Testing Strategy

### 13.1 Test Data Sources

| Source | Triangles | Textures | Format | Purpose |
|--------|-----------|----------|--------|---------|
| Pix4D sample export | ~500K | 1 JPG | OBJ + offset.xyz | Basic single-texture flow |
| Agisoft single export | ~2M | 4 JPGs | OBJ + metadata.xml | Multi-texture, auto-georef |
| Agisoft Block Model | ~5M total | 1 per block | Multi-OBJ | Tiled input flow |
| OpenDroneMap output | ~1M | 8 JPGs | OBJ (geo) | Multi-texture, offset |
| RealityCapture GLB | ~3M | embedded | GLB | glTF input path |
| Large stress test | 50M+ | 1 large atlas | OBJ | Streaming, memory limits |
| Vertex-colored PLY | ~1M | none | PLY | PLY path, no textures |

### 13.2 Validation Checks

For every test output:

1. **3d-tiles-validator** passes with no errors
2. **CesiumJS visual check**: model loads, LOD transitions work, textures render correctly
3. **Tile size check**: no single tile > 20MB (configurable threshold)
4. **LOD check**: parent tiles have fewer triangles than sum of children
5. **Georef check**: model appears at correct location on globe (when georef provided)
6. **Memory check**: peak RSS stays within target for input size
7. **Triangle conservation**: sum of leaf tile triangles >= original count (splitting adds triangles at boundaries)
8. **No gaps**: at leaf level, every input triangle is represented in some tile

### 13.3 Unit Test Coverage

| Module | Tests |
|--------|-------|
| `triangle-splitter.ts` | Clip triangle by plane (inside, outside, spanning), UV interpolation accuracy, degenerate cases |
| `mesh-simplifier.ts` | Simplification ratio accuracy, border locking, attribute preservation |
| `texture-repacker.ts` | UV bounding box computation, crop accuracy, UV remapping |
| `streaming-obj-parser.ts` | Bounds accuracy vs full parse, face count accuracy, byte offset accuracy |
| `georef-parser.ts` | Parse offset.xyz, parse metadata.xml, parse .prj, auto-detection |
| `ply-loader.ts` | ASCII PLY, binary PLY, vertex colors |
| `tileset-writer.ts` | REPLACE refinement in output, geometric error hierarchy |
| `coordinates.ts` | Y-up→Z-up, unit scaling, ECEF transform |

---

## 14. Dependencies

### 14.1 Keep from BIM-Tiler

```json
{
  "@gltf-transform/core": "^4.2.1",
  "@gltf-transform/extensions": "^4.2.1",
  "@gltf-transform/functions": "^4.2.1",
  "commander": "^12.1.0",
  "draco3dgltf": "^1.5.7",
  "gl-matrix": "^3.4.3",
  "meshoptimizer": "^0.21.0",
  "obj2gltf": "^3.2.0",
  "pino": "^9.0.0",
  "pino-pretty": "^11.0.0",
  "proj4": "^2.11.0",
  "sharp": "^0.34.5"
}
```

### 14.2 Remove (BIM-Specific)

```json
{
  "web-ifc": "REMOVE",
  "axios": "REMOVE (only used for APS HTTP calls)",
  "adm-zip": "REMOVE (only used for APS ZIP extraction)",
  "fs-extra": "EVALUATE (may still be useful)",
  "gltf-pipeline": "EVALUATE (may be replaceable by @gltf-transform)",
  "ktx2-encoder": "KEEP if KTX2 support desired",
  "zod": "EVALUATE (useful for config validation)"
}
```

### 14.3 Add (New)

```json
{
  "xml2js": "^0.6.2"
}
```

Only `xml2js` (or similar) is strictly needed for parsing `metadata.xml`. Everything else is already in the dependency tree. meshoptimizer is already installed.

### 14.4 Dev Dependencies (Keep)

```json
{
  "@types/node": "^20.14.0",
  "typescript": "^5.4.5",
  "jest": "^29.7.0",
  "3d-tiles-validator": "^0.4.0",
  "eslint": "^8.57.0"
}
```

---

## 15. Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| **meshoptimizer WASM can't handle 50M+ tri in one call** | Medium | High | Simplify in chunks; merge first, simplify per-octant |
| **UV interpolation artifacts at tile boundaries** | High | Medium | Add 1-2px padding when cropping textures; accept minor seams |
| **Streaming OBJ parser is complex to implement** | Medium | High | Start with in-memory for < 4GB files; add streaming later |
| **Large texture atlas (64K x 64K) crashes sharp** | Low | Medium | Downsample to max 8192 before processing; load regions via sharp.extract() |
| **Node.js memory limits for 10GB+ files** | High | High | Two-pass streaming is mandatory for large files; `--max-old-space-size` for medium |
| **Draco compression fails on simplified meshes** | Low | Low | meshoptimizer output is clean geometry; Draco handles it well |
| **Quadtree vs octree auto-detection wrong** | Low | Low | Default to octree; quadtree is optional optimization |
| **Multi-OBJ blocks have inconsistent CRS** | Low | Medium | Validate all blocks have same CRS in manifest; error if mismatched |
| **glTF/GLB input missing UV data** | Medium | Medium | Fall back to vertex colors or flat shading; warn user |

---

## Appendix A: File-by-File Action Checklist

```
DELETE:
  [ ] src/ingestion/aps-client.ts
  [ ] src/ingestion/aps-obj-client.ts
  [ ] src/ingestion/aps/auth.ts
  [ ] src/ingestion/aps/translation.ts
  [ ] src/ingestion/aps/aec-data.ts
  [ ] src/ingestion/aps/georeference.ts
  [ ] src/ingestion/aps/common.ts
  [ ] src/ingestion/aps/index.ts
  [ ] src/ingestion/ifc-loader.ts
  [ ] src/ingestion/metadata-parser.ts
  [ ] src/tiling/exterior-classifier.ts
  [ ] src/tiling/exterior-priority-tiling.ts

CREATE:
  [ ] src/tiling/triangle-splitter.ts
  [ ] src/tiling/mesh-simplifier.ts
  [ ] src/tiling/texture-repacker.ts
  [ ] src/ingestion/streaming-obj-parser.ts
  [ ] src/ingestion/georef-parser.ts
  [ ] src/ingestion/ply-loader.ts
  [ ] src/tiling/quadtree.ts (optional)

MODIFY:
  [ ] package.json (rename, update deps)
  [ ] src/types.ts (strip BIM types, add Photo-Tiler types)
  [ ] src/cli.ts (new CLI options)
  [ ] src/pipeline.ts (new pipeline flow)
  [ ] src/index.ts (update exports)
  [ ] src/tiling/octree.ts (triangle splitting instead of feature assignment)
  [ ] src/tiling/glb-writer.ts (remove BIM metadata extensions, add LOD)
  [ ] src/tiling/tileset-writer.ts (REPLACE refinement)
  [ ] src/tiling/texture-optimizer.ts (add atlas cropping)
  [ ] src/tiling/implicit-tiling.ts (activate, was dormant)
  [ ] src/tiling/index.ts (update exports)
  [ ] src/transform/coordinates.ts (remove BIM refs)
  [ ] src/transform/index.ts (update exports)
  [ ] src/ingestion/obj-converter.ts (remove APS-specific, add streaming)
  [ ] src/ingestion/gltf-loader.ts (remove BIM node name parsing)
  [ ] src/ingestion/feature.ts (simplify Feature creation)
  [ ] src/ingestion/units.ts (remove APS detection, keep scale math)
  [ ] src/ingestion/index.ts (update routing)

KEEP AS-IS:
  [ ] src/transform/matrix.ts
  [ ] src/transform/ecef.ts
  [ ] src/transform/projection.ts
  [ ] src/utils/logger.ts
  [ ] src/utils/errors.ts
  [ ] src/utils/index.ts
  [ ] src/types/draco3dgltf.d.ts
```

---

## Appendix B: Glossary

| Term | Definition |
|------|-----------|
| **LOD** | Level of Detail - progressively detailed versions of geometry |
| **REPLACE** | 3D Tiles refinement where children replace parent content on load |
| **ADD** | 3D Tiles refinement where children add to parent content (both visible) |
| **Octree** | Spatial tree that subdivides 3D space into 8 equal octants per level |
| **Quadtree** | Spatial tree that subdivides 2D space into 4 quadrants (good for flat terrain) |
| **Sutherland-Hodgman** | Polygon clipping algorithm against convex clip regions |
| **meshoptimizer** | Library for mesh optimization and simplification (used via WASM) |
| **ECEF** | Earth-Centered Earth-Fixed coordinate system (what 3D Tiles uses) |
| **ENU** | East-North-Up local tangent plane coordinate system |
| **Draco** | Google's geometry compression codec for glTF |
| **KTX2** | Khronos texture container format with GPU-compressed textures |
| **WebP** | Google's image format, 94% smaller than JPEG for textures |
| **Atlas** | Single texture image containing UV-mapped regions for multiple surfaces |
| **Block Model** | Agisoft Metashape's tiled export mode (spatial blocks) |
| **offset.xyz** | Pix4D's file containing XYZ coordinate offsets |
| **metadata.xml** | Agisoft/DJI Terra's file containing CRS and transform metadata |
