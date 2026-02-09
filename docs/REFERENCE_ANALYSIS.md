# Reference Implementation Analysis: obj2tiles, mago3d-tiler, cesium-native

Deep technical comparison of the three leading open-source 3D Tiles generation tools. This document serves as the design reference for Photo-Tiler's Rust implementation.

---

## Tool Overview

| | obj2tiles | mago3d-tiler | cesium-native |
|---|---|---|---|
| **Language** | C# (.NET 6+) | Java 21 | C++ 17 |
| **License** | AGPLv3 | MPL 2.0 | Apache 2.0 |
| **Maintainer** | OpenDroneMap | Gaia3D | Cesium (the spec authors) |
| **Primary use** | Photogrammetry OBJ to 3D Tiles | Multi-format (OBJ/glTF/FBX/IFC/CityGML/LAS) to 3D Tiles | 3D Tiles runtime + writing library |
| **Repo** | [OpenDroneMap/Obj2Tiles](https://github.com/OpenDroneMap/Obj2Tiles) | [Gaia3D/mago-3d-tiler](https://github.com/Gaia3D/mago-3d-tiler) | [CesiumGS/cesium-native](https://github.com/CesiumGS/cesium-native) |

---

## 1. Pipeline Architecture

### obj2tiles: Decimate -> Split -> Tile (3 sequential stages)

```
Stage 1: Decimation
  - Generate LOD variants (quality 0.667, 0.333 for --lods 3)
  - Uses Fast Quadric Mesh Simplification (Sven Forstmann port)
  - LOD 0 = original (copied as-is)

Stage 2: Splitting
  - For EACH LOD independently:
    - Recursive binary plane splits (X then Y, optionally Z)
    - Triangle clipping at every boundary
    - Texture atlas repacking per split tile
  - Divisions parameter controls recursion depth (default 2 = 16 tiles)

Stage 3: Tiling
  - Convert each split OBJ to glTF -> GLB -> B3DM
  - Build tileset.json (LOD-chain hierarchy, not spatial)
  - Parallel B3DM conversion via Parallel.ForEach
```

**Key insight**: Decimation happens on the FULL original mesh before any spatial splitting. The simplifier has complete topology context.

### mago3d-tiler: Pre-process -> Split -> Decimate -> Post-process (4 phases)

```
Phase 1: Pre-processing (multi-threaded per file)
  - TileInfoGenerator, texture coord correction
  - Scale, axis alignment, CRS transform
  - Bake transforms into vertices

Phase 2: Tiling (single-threaded for photogrammetry)
  - For each LOD level:
    - Cut mesh into octree tiles via half-edge plane intersection
    - Scissor textures (island detection + guillotine packing)
    - LOD 0: no decimation (full detail)
    - LOD 1-2: half-edge collapse decimation per tile
    - LOD 3+: vertex clustering remesh + optional GPU re-rendering

Phase 3: Post-processing (multi-threaded per tile)
  - Relocate geometry for globe
  - Write B3DM or GLB files
```

**Key insight**: Decimation happens AFTER splitting (per-tile), but compensated by half-edge topology awareness during both cutting and decimation.

### cesium-native: Library approach (not a standalone pipeline)

cesium-native is a C++ library, not a standalone converter. It provides:
- 3D Tiles spec-compliant data structures
- tileset.json writer (auto-generated from JSON Schema)
- GLB reader/writer
- Runtime tile loading and selection

**Key insight**: cesium-native is the reference for CORRECT tileset.json structure and spec compliance. Use it as the guide for our output format.

---

## 2. Spatial Partitioning (Detailed)

### obj2tiles: Recursive Binary Plane Splits

**Not an octree.** Uses a fixed-sequence binary split pattern:

```
RecurseSplitXY(mesh, depth):
  if depth == 0: return [mesh]
  left, right = split(mesh, X, center.x)      // binary split on X
  TL, BL = split(left, Y, center.y)           // binary split on Y
  TR, BR = split(right, Y, center.y)           // binary split on Y
  return [
    RecurseSplitXY(TL, depth-1),
    RecurseSplitXY(BL, depth-1),
    RecurseSplitXY(TR, depth-1),
    RecurseSplitXY(BR, depth-1),
  ]
```

With `--divisions N`: produces `4^N` leaf tiles in XY mode, `8^N` in XYZ mode.

Split point: `mesh.GetVertexBaricenter()` (centroid of all vertices), NOT geometric center.

### mago3d-tiler: True 3D Octree with Half-Edge Cutting

Uses half-edge data structure for topologically correct splitting:

```
1. Compute octree depth: ceil(log2(extent / 25.0))  // 25m target leaf size
2. Generate axis-aligned cutting planes for full octree grid
3. For each plane:
   - Iterate all half-edges
   - Find intersections with plane
   - Split intersected edges, creating new vertices
   - Split affected faces into sub-triangles
   - Maintain half-edge twin relationships
4. Distribute resulting faces to octree leaves by centroid
```

The half-edge structure guarantees topological consistency: when an edge is split, BOTH adjacent faces are updated, and the twin pointers are correctly maintained. This prevents T-junctions.

### Photo-Tiler (Rust): Adaptive Octree with Sutherland-Hodgman

Our approach (following obj2tiles' clipping correctness with octree spatial structure):

```
1. Build octree from bounding box
2. For each triangle:
   a. Fast-path: if AABB fits entirely in one octant, assign directly
   b. Slow-path: clip against all 6 octant planes via Sutherland-Hodgman
   c. Deduplicate boundary vertices via position hash
3. Stop conditions: max depth, triangle count, min tile size
```

---

## 3. Triangle Clipping (Detailed)

### obj2tiles: 8-Case Vertex Classification

For each triangle crossing a split plane at position `q`:

```csharp
var aSide = GetDimension(vA) < q;
var bSide = GetDimension(vB) < q;
var cSide = GetDimension(vC) < q;

// 8 cases (2^3):
// All same side: assign whole triangle
// 1 vs 2 split: compute 2 intersection points, create 3 new triangles

CutEdge(a, b, q):
  dx = a.X - b.X
  my = (a.Y - b.Y) / dx
  mz = (a.Z - b.Z) / dx
  return Vertex3(q, my * (q - a.X) + a.Y, mz * (q - a.X) + a.Z)
```

For textured meshes (`MeshT`), UV interpolation at split points:
```csharp
perc = Common.GetIntersectionPerc(a, splitPoint, edgeLength)
Vertex2.CutEdgePerc(b, perc):
  return Vertex2((b.X - X) * perc + X, (b.Y - Y) * perc + Y)
```

### mago3d-tiler: Half-Edge Split

```java
splitHalfEdge(halfEdge, plane):
  // Compute parametric t along edge
  t = (planePosition - startPos) / (endPos - startPos)

  // Create new vertex at intersection
  newVertex.position = lerp(startPos, endPos, t)
  newVertex.normal = lerp(startNormal, endNormal, t)
  newVertex.texCoord = lerp(startUV, endUV, t)

  // Split the face containing this half-edge
  // AND the face containing the twin half-edge
  // Maintains all topological relationships
```

### Photo-Tiler (Rust): Sutherland-Hodgman Polygon Clipping

```rust
clip_polygon_by_plane(vertices, plane) -> Vec<Vertex>:
  for each edge (current, next):
    curr_inside = classify(current, plane)
    next_inside = classify(next, plane)

    match (curr_inside, next_inside):
      (true, true)   => output.push(next)
      (true, false)  => output.push(intersect(current, next, plane))
      (false, true)  => { output.push(intersect(current, next, plane));
                          output.push(next) }
      (false, false) => skip

intersect(a, b, plane) -> Vertex:
  t = (plane.value - a.pos[axis]) / (b.pos[axis] - a.pos[axis])
  Vertex {
    pos: lerp(a.pos, b.pos, t),
    normal: normalize(lerp(a.normal, b.normal, t)),
    uv: lerp(a.uv, b.uv, t),
    color: lerp(a.color, b.color, t),
  }
```

---

## 4. Texture Atlas Handling (Detailed)

### obj2tiles: MaxRectanglesBinPack

**Step 1: Island detection** via edge adjacency graph
```csharp
GetFacesClusters():
  edgeMapper = GetEdgesMapper()  // edge -> [face1, face2]
  // BFS/DFS traversal grouping connected faces
  return List<FaceCluster>
```

**Step 2: Per-island UV bounds** with normalization for wrapping UVs
```csharp
foreach vertex in cluster:
  minU = min(minU, vertex.u)
  maxU = max(maxU, vertex.u)
  // ... same for V
```

**Step 3: Bin packing** via Jukka Jylänki's MaxRectanglesBinPack
- Tries all rotations and placements
- Chooses placement minimizing wasted space
- Atlas dimensions rounded to next power-of-two

**Step 4: Bleed ring** (`BuildPaddedBlock`)
- Extracts pixel region from source texture
- Adds N-pixel border by sampling edge texels
- Prevents bilinear filtering artifacts at atlas boundaries

**Step 5: UV remapping**
```csharp
newU = (oldU - islandMinU) / islandWidth * packedWidth / atlasWidth + packedX / atlasWidth
newV = (oldV - islandMinV) / islandHeight * packedHeight / atlasHeight + packedY / atlasHeight
```

**Texture strategies by LOD**:
- LOD 0: `Repack` (PNG, highest quality)
- LOD 1+: `RepackCompressed` (JPEG)

### mago3d-tiler: Guillotine Packing

**Step 1: Island detection** via half-edge traversal
```java
getWeldedFacesGroups():
  // Traverse half-edge faces
  // Group by shared edges
mergeWeldedFacesGroupsByTexCoords():
  // Merge groups sharing UV space
```

**Step 2: UV normalization** (handle wrapping)
```java
if (texCoordOriginX < 0.0 || texCoordOriginX > 1.0):
  offsetX = Math.floor(texCoordOriginX)
  texCoord.x -= offsetX
```

**Step 3: Pixel expansion** (adaptive bleed)
```java
expandedPixels = 2  // default
if pixelWidth > 200 || pixelHeight > 200: expandedPixels = 5
elif pixelWidth > 100 || pixelHeight > 100: expandedPixels = 4
```

**Step 4: Guillotine packing**
- Maintains list of free rectangles
- For each island rectangle: find best-fit free rectangle
- Split remaining space after placement
- More predictable than MaxRects but slightly less optimal

**Step 5: Atlas compositing**
- Create new BufferedImage at computed dimensions
- Copy pixel regions from source textures
- Handle ARGB vs RGB image types
- Save with configured JPEG quality

**Texture size limits**:
```java
REALISTIC_LOD0_MAX_TEXTURE_SIZE = 1024
REALISTIC_MAX_TEXTURE_SIZE = 512
REALISTIC_MIN_TEXTURE_SIZE = 32
```

### Photo-Tiler (Rust): Guillotine Packing (following mago3d)

Our implementation follows mago3d's approach with obj2tiles' MaxRects quality:

1. Edge adjacency via `HashMap<(u32, u32), Vec<usize>>` (edge -> face list)
2. BFS island detection
3. UV bounds per island with wrapping normalization
4. Guillotine bin packing (simpler to implement correctly than MaxRects)
5. Adaptive bleed ring (2-5px based on island size)
6. UV remapping to atlas coordinates
7. Image compositing via `image` crate

---

## 5. Mesh Simplification (Detailed)

### obj2tiles: Fast Quadric Mesh Simplification

Port of Sven Forstmann's algorithm (via Mattias Edlund's MeshDecimatorCore):

```
Core loop:
  1. Compute quadric error matrix Q per vertex
     (sum of squared distances to adjacent triangle planes)
  2. For each edge, compute collapse cost = v^T * Q * v
  3. Sort by cost (ascending)
  4. Progressively collapse lowest-cost edges:
     a. Move vertex to optimal position minimizing Q
     b. Update neighboring quadrics
     c. Check topology validity
  5. Stop when target triangle count reached

Threshold progression:
  threshold = 1e-9 * (iteration + 3)^agressiveness
```

**UV preservation**:
- `PreserveSeams = true`: Detects UV seam edges (where same spatial vertex has different UVs on different faces). Refuses to collapse these edges.
- `PreserveBorders = true`: Prevents collapsing boundary edges.
- UV merging: `MergeVertexAttributes()` averages UVs when blending vertices.

**Quality targets** for `--lods 3`:
- LOD 1: keep 66.7% of triangles
- LOD 2: keep 33.3% of triangles

### mago3d-tiler: Three-Tier Simplification

**Tier 1: Half-edge collapse (LOD 1-2)**
```java
decimate(parameters):
  1. Sort half-edges by length (shortest first)
  2. For each edge, check collapse validity:
     - Normal deviation < maxDiffAngDeg
     - Aspect ratio < maxAspectRatio
     - |dot(collapseDir, faceNormal)| < 0.9
     - For small edges: angFactor = (length/smallSize)^2
  3. Collapse valid edges by moving start -> end vertex
  4. Re-sort and repeat for iterationsCount passes
```

Parameters by LOD:
```java
LOD 1: maxAngle=14, minLength=0.1, maxDot=0.9, maxAspect=40
LOD 2: maxAngle=22, minLength=0.2, maxDot=0.9, maxAspect=40
LOD 3+: maxAngle=20, minLength=0.8, maxDot=1.0, maxAspect=36
```

**Tier 2: Vertex clustering (LOD 3+)**
```java
ReMesherVertexCluster:
  1. Divide space into regular voxel grid
  2. voxelSize = nodeSize / 30.0
  3. Average all vertices per cell
  4. Remap faces, discard degenerate triangles
```

**Tier 3: GPU oblique camera re-rendering (extension module)**
- Renders mesh from 6 oblique directions via OpenGL
- Captures depth + color buffers
- Reconstructs simplified mesh from rendered views
- Produces both simplified geometry AND new textures

### Photo-Tiler (Rust): Native meshoptimizer

```rust
meshopt::simplify(
    &indices,
    &VertexDataAdapter::new(
        bytemuck::cast_slice(&positions),
        std::mem::size_of::<[f32; 3]>(),
        0,
    ),
    target_index_count,
    target_error,  // 0.01 = 1% deformation tolerance
    SimplifyOptions::LockBorder,
    Some(&mut result_error),
)
```

meshoptimizer uses quadric error metrics (same family as obj2tiles' algorithm) but with additional optimizations:
- Attribute-aware simplification (considers UV discontinuities)
- `LockBorder` prevents boundary edge collapse
- `simplify_with_attributes_and_locks` for weighted attribute preservation
- Returns error metric for geometric error calculation

---

## 6. GLB / B3DM Writing

### obj2tiles: OBJ -> glTF -> GLB -> B3DM

```
OBJ -> glTF (JSON):
  - Blinn-Phong to PBR conversion
  - Roughness = 1 - inverted specular exponent
  - UV V-flip: v = 1 - v (OBJ to glTF convention)
  - Triangle winding validation via cross product

glTF -> GLB:
  - 12-byte header: magic("glTF"), version(2), totalLength
  - JSON chunk: padded to 4-byte alignment
  - Binary chunk: padded to 4-byte alignment
  - All external refs (buffers, textures) embedded

GLB -> B3DM:
  - 28-byte header: magic("b3dm"), version(1), byteLength, ...
  - Feature table: {"BATCH_LENGTH": 0}
  - Empty batch table
  - GLB data padded to 8-byte alignment
```

### mago3d-tiler: JglTF library + B3DM/GLB

```
GaiaScene -> GltfModel -> GltfModelWriter -> .glb or .b3dm

Photogrammetry optimizations:
  - Force JPEG textures
  - Quantize positions to unsigned short (16-bit)
  - Pack normals to byte (4-component)
  - Compress texcoords to unsigned short
  - Index buffer: uint16 if <65535 verts, uint32 otherwise

B3DM (1.0): Same format as obj2tiles
GLB (1.1): Direct .glb files, metadata via EXT_structural_metadata
```

### cesium-native: Code-generated writer

```
Tileset struct auto-generated from 3D Tiles JSON Schema
TilesetWriter::write(tileset) -> serialized JSON

Smart defaults: optional properties with default values are NOT written
Supports all bounding volume types (region, box, sphere)
```

### Photo-Tiler (Rust): gltf-json direct construction

```rust
// Build glTF document manually via gltf-json structs
let root = gltf_json::Root {
    buffers: vec![buffer],
    buffer_views: vec![pos_view, normal_view, uv_view, index_view],
    accessors: vec![pos_acc, normal_acc, uv_acc, index_acc],
    meshes: vec![mesh],
    nodes: vec![node],
    scenes: vec![scene],
    materials: vec![material],
    textures: vec![texture],
    images: vec![image],
    ..Default::default()
};

// Serialize to GLB
let json_bytes = serde_json::to_vec(&root)?;
// Write: 12-byte header + JSON chunk + binary chunk
```

Direct GLB output (3D Tiles 1.1), no B3DM wrapper.

---

## 7. tileset.json Structure

### obj2tiles: LOD-chain hierarchy

```json
{
  "root": {
    "geometricError": 100,
    "refine": "ADD",
    "children": [
      {
        "content": { "uri": "LOD-2/tile_0.b3dm" },
        "geometricError": 8.5,
        "refine": "REPLACE",
        "children": [
          {
            "content": { "uri": "LOD-1/tile_0.b3dm" },
            "geometricError": 2.1,
            "refine": "REPLACE",
            "children": [
              {
                "content": { "uri": "LOD-0/tile_0.b3dm" },
                "geometricError": 0
              }
            ]
          }
        ]
      }
    ]
  }
}
```

Flat spatial structure, deep LOD chains. All spatial tiles at coarsest LOD are siblings under root.

Geometric error: `pow(relBBoxDiff, lod)` heuristic.

### mago3d-tiler: Spatial octree hierarchy

```json
{
  "root": {
    "geometricError": 500,
    "refine": "REPLACE",
    "boundingVolume": { "region": [...] },
    "children": [
      {
        "content": { "uri": "data/R0000.glb" },
        "geometricError": 2.0,
        "boundingVolume": { "region": [...] },
        "children": [...]
      }
    ]
  }
}
```

Always uses `region` bounding volumes (geographic coordinates).

Geometric error by LOD:
- LOD 0: 0.01
- LOD 1: 1.0
- LOD 2: 2.0
- LOD 3+: nodeSize * 0.05

### Photo-Tiler (Rust): Spatial octree with LOD per level

```json
{
  "asset": { "version": "1.1" },
  "geometricError": 150.0,
  "root": {
    "boundingVolume": { "box": [12 floats] },
    "geometricError": 150.0,
    "refine": "REPLACE",
    "transform": [16 f64 column-major],
    "content": { "uri": "tiles/root.glb" },
    "children": [
      {
        "boundingVolume": { "box": [...] },
        "geometricError": 75.0,
        "content": { "uri": "tiles/0/tile.glb" },
        "children": [...]
      }
    ]
  }
}
```

Uses `box` bounding volumes (ECEF-aligned). Geometric error halves at each level.

---

## 8. Coordinate Systems

### obj2tiles: Y/Z swap + ECEF

```csharp
// OBJ to ECEF bounding volume:
Box = [
    center.X, -center.Z, center.Y,    // Y/Z swap, negate Z
    width/2, 0, 0,
    0, -depth/2, 0,
    0, 0, height/2
]

// GPS to ECEF transform: WGS84 ellipsoid
ToEcefTransform(lat, lon, alt):
  // Standard geodetic-to-ECEF with ENU rotation matrix
  // Column-major 4x4 output
```

Default location if no GPS: Milan, Italy (45.464, 9.190).

### mago3d-tiler: proj4j + GeoTools

- Full CRS support via GeoTools + proj4j
- Terrain height correction (geoid model support)
- Region bounding volumes in radians
- `GaiaTranslationForPhotogrammetry` handles the photogrammetry-specific transform chain

### Photo-Tiler (Rust): proj + custom ECEF

- CRS projection via `proj` crate (binds to PROJ C library)
- Custom geodetic-to-ECEF + ENU rotation
- All transforms in f64
- Box bounding volumes in ECEF-aligned coordinates

---

## 9. Performance Characteristics

### obj2tiles

- Parallel B3DM conversion via `Parallel.ForEach`
- Sequential mesh splitting (single-threaded recursive)
- .NET GC handles memory (occasional pauses)
- Best for: medium-sized photogrammetry OBJ (< 5GB)

### mago3d-tiler

- Conservative threading: max 3 threads by default
- Phase 2 (tiling) is single-threaded for photogrammetry
- Temp file serialization for memory management
- No out-of-core processing (files must fit in RAM)
- Half-edge structure has high per-element memory overhead
- Best for: multi-format support, enterprise deployment

### Photo-Tiler (Rust, target)

- rayon parallelism at every stage (all CPU cores)
- Memory-mapped I/O for large files
- No GC pauses
- Native meshoptimizer (no WASM overhead)
- f64 precision for transforms
- Best for: maximum performance on large photogrammetry datasets

---

## 10. Quality Comparison Summary

| Aspect | obj2tiles | mago3d-tiler | Photo-Tiler (target) |
|---|---|---|---|
| **Gap-free boundaries** | Yes (always clips) | Yes (half-edge split) | Yes (always clips, no fallback) |
| **Correct UV at splits** | Yes (percentage lerp) | Yes (parametric lerp) | Yes (parametric lerp) |
| **Per-island atlas** | Yes (MaxRectsBinPack) | Yes (GuillotinePacker) | Yes (GuillotinePacker) |
| **Bleed padding** | Yes (configurable) | Yes (adaptive 2-5px) | Yes (adaptive 2-5px) |
| **UV seam preservation** | Yes (PreserveSeams flag) | Yes (half-edge topology) | Yes (meshopt LockBorder) |
| **LOD topology quality** | Best (decimate first) | Good (topology-aware post-split) | Best (decimate first) |
| **Precision** | Double (C# default) | Double (Java default) | f64 transforms, f32 storage |
| **Output format** | B3DM (Tiles 1.0) | B3DM or GLB (1.0/1.1) | GLB (Tiles 1.1) |
| **V-flip handling** | Explicit in glTF writer | Handled in texture correction | Explicit in OBJ parser |

---

## 11. Lessons for Photo-Tiler

### From obj2tiles (adopt)
1. **Decimate-first pipeline** -- proven to produce the best LOD quality
2. **Always-clip triangles** -- no size-based fallback, ever
3. **Percentage-based UV interpolation** -- consistent with 3D space interpolation
4. **LOD-specific texture strategies** -- highest quality for LOD 0, compressed for lower LODs
5. **Vertex deduplication** via `Dictionary<Vertex, int>` after splitting

### From mago3d-tiler (adopt)
1. **Guillotine packing** -- simpler and more predictable than MaxRects
2. **Adaptive bleed pixels** -- 2px for small islands, 5px for large ones
3. **UV wrapping normalization** -- handle UVs outside [0,1] before atlas computation
4. **Texture size limits per LOD** -- LOD 0 max 1024px, lower LODs max 512px
5. **Half-edge concept for split correctness** -- even without full half-edge structure, maintain boundary vertex sharing

### From cesium-native (adopt)
1. **Spec-compliant tileset.json** -- follow their serialization patterns
2. **Smart default handling** -- don't write optional fields with default values
3. **Box bounding volumes** -- oriented bounding boxes for tightest fit
4. **3D Tiles 1.1** -- direct GLB content, no B3DM wrapper

### From all three (validate)
1. All three use **REPLACE** refinement for mesh tiles
2. All three use **linear interpolation** for vertex attributes at split points
3. All three generate **per-tile textures** (no shared atlas across tiles)
4. None of the three use centroid-based triangle assignment for photogrammetry meshes

---

## 12. Performance Deep Dive & Optimization Plan (February 2026)

Empirical performance analysis from processing a real 169M triangle photogrammetry model (16.8 GB OBJ, 8K texture) on an 11-core Apple Silicon machine with 18 GB RAM and NVMe SSD.

### 12.1 Current Performance Profile

**Test model**: `test/Model.obj` -- 169,317,482 triangles, 91,304,901 vertices, 8192x8192 JPEG texture

**Pipeline timing (single run, --validate)**:

| Stage | Duration | Notes |
|---|---|---|
| Ingestion (OBJ parse) | ~2.5 min | I/O bound (16.8 GB file read) |
| Transform (scale, center, ECEF) | ~2 sec | Fast -- simple vertex arithmetic |
| Tiling (recursive octree + GLB) | ~5 hours | **THE BOTTLENECK** |
| Validation | ~5-10 min | Reads all GLBs back from disk |
| **Total** | **~5.5 hours** | |

**Output**: ~5,500 GLB tiles, ~14 GB on disk, max tree depth 6

**Resource utilization during tiling**:

| Resource | Utilization | Capacity | Bottleneck? |
|---|---|---|---|
| CPU | 99.9% on **1 of 11 cores** | 11 cores available | **YES -- single-threaded** |
| RAM | 3 GB (17%) | 18 GB | No |
| Disk I/O | 3-14 MB/s | ~3 GB/s (NVMe) | No |
| GPU | 0% | Not used | No |

The pipeline is **purely CPU-bound and single-threaded**. 10 of 11 CPU cores sit idle throughout the 5-hour tiling stage.

### 12.2 Bottleneck Breakdown

#### #1: Triangle Clipper -- Sutherland-Hodgman (est. 30-40% of tiling time)

**Location**: `src/tiling/triangle_clipper.rs`

Every boundary-straddling triangle (~95% of all triangles) is clipped against all 8 octants x 6 planes = 48 clipping operations per triangle. For 169M triangles:

- 169M x 0.95 x 48 = **7.7 billion polygon clipping operations**
- Each boundary triangle clones 3 `ClipVertex` structs (96 bytes each) 8 times = 24 clones/triangle
- No pre-filtering: tests all 8 octants even when a triangle only touches 2-3
- No early exit: fully-clipped-away triangles still do work

**Fix**: Pre-compute an octant bitmask from the triangle's AABB. Only clip against the 2-4 octants the triangle actually overlaps. Expected 3-5x speedup on the clipper alone.

#### #2: WebP Texture Encoding (est. 25% of tiling time)

**Location**: `src/tiling/texture_compress.rs`

Each tile's atlas is encoded to WebP synchronously on the single thread:

- ~5,500 tiles x ~2K atlas each
- WebP encoding ~1-2 sec/tile on modern CPU
- **5,500 tiles x 1.5s = ~2.3 hours just for WebP**
- Uses `image` crate's `write_to()` -- no SIMD, no parallelism

**Fix**: Parallelizing `build_tile_recursive()` (see #3) would automatically parallelize WebP encoding since it happens inside the recursive function. Additionally, switching to KTX2/ETC1S (via `basis-universal` crate) would be 2-3x faster to encode and better for GPU memory at runtime.

#### #3: Sequential Recursive Tiling (est. 20% structural overhead)

**Location**: `src/tiling/tileset_writer.rs`

The core `build_tile_recursive()` function processes children in a sequential `for` loop:

```rust
for (i, sub) in sub_meshes.into_iter().enumerate() {
    let child = build_tile_recursive(sub, ...);  // blocks until complete
}
```

All shared arguments (`materials`, `texture_config`, `out_dir`) are immutable references -- **this is trivially parallelizable** with rayon's `into_par_iter()`. The `rayon` crate is already in `Cargo.toml` but unused in the main tiling path.

**Fix**: Replace the `for` loop with `into_par_iter()`. At depth 0, this immediately utilizes 4-8 cores (one per non-empty octant). At depth 2+, the work-stealing scheduler spreads 64+ subtrees across all 11 cores. Expected 8-11x speedup.

#### #4: Atlas Repacking -- UV Islands + Compositing (est. 10%)

**Location**: `src/tiling/atlas_repacker.rs`

Two sub-problems:
- **UV island detection** uses O(n^2) edge-pair checking in pathological cases (edges shared by many faces)
- **Image compositing** is pixel-by-pixel via `put_pixel()` -- no SIMD batching
- 5,500 tiles x 4M pixels = **22 billion pixel operations**

**Fix**: Use `copy_from_slice()` for scanline-based blitting instead of per-pixel. Consider `image::imageops` for bulk operations. Would parallelize for free with fix #3.

#### #5: Mesh Cloning Cascade (est. 5%)

**Locations**: `src/tiling/simplifier.rs`, `src/tiling/tileset_writer.rs`

Each recursive node: `simplify_mesh()` clones the mesh, `compact_mesh()` allocates new attribute arrays, `split_mesh()` creates 8 new meshes. For 169M triangles:

- ~5,500 tiles x ~4 GB temporary allocations per simplify = **~22 TB of memory traffic** (temporary, but thrashes allocator)
- `compact_mesh()` does 3 passes: build remap, remap indices, copy attributes

**Fix**: Use `Cow<[f32]>` for attribute arrays. Skip compaction at deep levels where meshes are small. Use in-place index remapping.

#### #6: Simplification Double-Scan (est. 5%)

**Location**: `src/tiling/simplifier.rs`

Three sequential passes: meshopt simplify -> vertex cache optimize -> compact. The compact pass re-scans all indices and copies all attribute arrays. For the root level (169M triangles), this is 500M+ index scans.

**Fix**: Use `meshopt::simplify_sloppy()` for deep tree levels (5-6x faster, 20M tri/sec vs 3M tri/sec). Skip vertex cache optimization for intermediate nodes (only leaves are rendered at full detail).

### 12.3 Competitive Benchmarks

#### Cesium Reality Tiler V2 (C++, closed-source, December 2024)

Source: [cesium.com/blog/2024/12/11/reality-tiler-v2](https://cesium.com/blog/2024/12/11/reality-tiler-v2-improves-tiling-time-and-memory-usage/)

| Dataset | Triangles | V1 Time | V2 Time | V2 Peak RAM |
|---|---|---|---|---|
| Carrick Hill | 3.3M | 2m 37s | **1m 11s** | 14.9 GB |
| Liverpool | 7.8M | 8m 48s | **2m 54s** | 22.7 GB |
| Budapest | 43.1M | 59m 05s | **14m 08s** | 21.3 GB |
| Melbourne | 69.9M | 3h 22m | **27m 58s** | 26.9 GB |
| **169M (extrapolated)** | **169M** | **~8 hours** | **~40-60 min** | **~35-50 GB** |

V2 key optimizations over V1:
- Multi-core spatial parallelism (concurrent octant processing)
- Out-of-core memory-mapped files on NVMe SSD
- Non-uniform adaptive octree (better balanced than uniform)
- KTX2/ETC1S texture compression (faster than WebP, 80% GPU memory savings)
- Tile size uniformity: average tile 395 KB (SD 192 KB) vs V1's 1.28 MB (SD 706 KB)

#### Other Open-Source Tools

| Tool | Language | 100M+ tri est. | Notes |
|---|---|---|---|
| **Obj2Tiles** | C# (.NET) | 5-24+ hours | Multi-threaded B3DM stage only. OOMs on large models. Community reports 2 GB model = 5 hours. |
| **mago3d-tiler** | Java 21 | Unknown (hours) | Phase 2 (tiling) is single-threaded. No published benchmarks. |
| **py3dtiles** | Python | N/A | Mesh support not production-ready (funded by NLnet, in development). Point cloud only. |
| **Tyler** | Rust | N/A | CityJSON only, not photogrammetry meshes. Uses rayon for parallel feature indexing. |
| **FME** | Commercial | Unknown | Does not split meshes -- requires pre-splitting. No published mesh tiling benchmarks. |

**There are no other known Rust-based photogrammetry-to-3DTiles tools.**

#### Photo-Tiler Current vs Competitors

| Tool | 169M tri (est.) | Parallelism | Architecture |
|---|---|---|---|
| **Cesium V2** | ~40-60 min | Full multi-core + out-of-core | Best-in-class (closed source) |
| **Cesium V1** | ~8 hours | Limited | Similar arch to Photo-Tiler current |
| **Photo-Tiler (current)** | **~5.5 hours** | Single-threaded | On par with Cesium V1 |
| **Photo-Tiler (optimized)** | **~15-30 min** | Full multi-core | Target: match/beat Cesium V2 |

### 12.4 Optimization Roadmap

Ranked by impact-to-effort ratio:

| Priority | Fix | Speedup | Effort | Details |
|---|---|---|---|---|
| **P0** | Parallelize `build_tile_recursive()` via `into_par_iter()` | **8-11x** | Low (2-line change) | All child octants processed concurrently. All downstream work (simplify, clip, atlas, WebP) parallelized for free. |
| **P1** | Smarter triangle clipper (octant bitmask pre-filter) | **3-5x** | Medium | Pre-compute which octants a triangle overlaps from its AABB. Skip clipping against octants it cannot touch. Reduces 48 clip ops to ~8-12. |
| **P2** | Use `meshopt::simplify_sloppy()` for depth >= 3 | **2-3x on simplify** | Low | 20M tri/sec vs 3M tri/sec. Quality loss acceptable for coarse LODs. |
| **P3** | KTX2/ETC1S textures (via `basis-universal`) | **2-3x on encode** | Medium | Faster encoding than WebP. Smaller files. GPU-native transcoding at runtime. |
| **P4** | SIMD atlas compositing (`copy_from_slice` scanlines) | **2x on atlas** | Medium | Replace per-pixel `put_pixel()` with scanline bulk copies. |
| **P5** | Out-of-core processing (memory-mapped meshes) | RAM savings | High | Required for models > available RAM. Cesium V2 reduced 134 GB to 27 GB peak via mmap. |
| **P6** | Skip compaction at leaf level | **1.5x on simplify** | Low | Leaf meshes are small; compaction overhead > savings. |

**Combined estimated speedup**: 20-40x (limited by Amdahl's law on serial fractions)

**P0 alone** would bring the 169M triangle model from **~5.5 hours to ~30-45 minutes**, competitive with Cesium V2.

### 12.5 Benchmark Results (Microbenchmarks via Criterion)

Test hardware: 11-core Apple Silicon, 18 GB RAM

| Benchmark | Input Size | Time | Throughput |
|---|---|---|---|
| `simplify_mesh_50pct_100k` | 100K triangles, 50% target | 10.636 ms | 9.4M tri/sec |
| `simplify_mesh_25pct_100k` | 100K triangles, 25% target | 10.690 ms | 9.4M tri/sec |
| `lod_chain_4_levels_100k` | 100K triangles, 4 LOD levels | 15.588 ms | 6.4M tri/sec |
| `split_mesh_clipping_88k` | 88K triangles, octree split | 14.985 ms | 5.9M tri/sec |
| `split_mesh_with_attrs_17k` | 17K triangles with UVs/normals | 3.792 ms | 4.5M tri/sec |
| `build_octree_depth4_88k` | 88K triangles, depth 4 | 21.301 ms | 4.1M tri/sec |

The per-operation throughput is reasonable (meshopt is well-optimized). The performance problem is entirely structural: **sequential recursion prevents utilizing available parallelism**.

### 12.6 Key Architectural Insight

From Cesium's spatial subdivision blog post (2017) and Reality Tiler V2 architecture:

> Once you spatially subdivide, each octant is completely independent. The children share only immutable data (materials, config, output path). This is the natural parallelism boundary.

Photo-Tiler's `build_tile_recursive()` already has this property -- the children share only `&MaterialLibrary`, `&TextureConfig`, and `&Path`, all immutable references. The `for` loop to `into_par_iter()` change is the single highest-impact optimization available.

Sources:
- [Reality Tiler V2 Benchmarks (Cesium, Dec 2024)](https://cesium.com/blog/2024/12/11/reality-tiler-v2-improves-tiling-time-and-memory-usage/)
- [Reality Tiler Architecture (Cesium, Nov 2023)](https://cesium.com/blog/2023/11/01/new-reality-tiler/)
- [Cesium On-Premises Reality Tiler Docs](https://cesium.com/learn/3d-tiling/on-prem/on-prem-reality-tiler/)
- [Optimizing Spatial Subdivisions in Practice (Cesium, 2017)](https://cesium.com/blog/2017/04/04/spatial-subdivision-in-practice/)
- [meshoptimizer Performance (zeux)](https://meshoptimizer.org/)
- [OpenDroneMap/Obj2Tiles GitHub](https://github.com/OpenDroneMap/Obj2Tiles)
- [Gaia3D/mago-3d-tiler GitHub](https://github.com/Gaia3D/mago-3d-tiler)
- [3DGI/tyler GitHub (Rust)](https://github.com/3DGI/tyler)
- [Fast Out-of-Core Octree Generation (Schutz 2020)](https://onlinelibrary.wiley.com/doi/10.1111/cgf.14134)
