# Photo-Tiler Architecture

## Overview

Photo-Tiler is a Rust pipeline that converts photogrammetry meshes into OGC 3D Tiles 1.1 tilesets. The architecture is a four-stage pipeline with parallel execution at every stage via rayon.

```
Input (OBJ/glTF/PLY) --> Ingest --> Transform --> Tile --> Output (3D Tiles 1.1)
```

## Design Principles

Informed by deep analysis of obj2tiles (C#), mago3d-tiler (Java), and cesium-native (C++):

1. **Always clip triangles** -- no centroid-based fallback. Every triangle crossing a tile boundary is split precisely via Sutherland-Hodgman clipping with full attribute interpolation. This is how both obj2tiles and mago3d-tiler avoid gaps.

2. **Per-island texture atlas repacking** -- detect connected UV islands via edge adjacency, bin-pack into per-tile atlases with bleed padding, remap UVs. Both obj2tiles (MaxRectanglesBinPack) and mago3d-tiler (GuillotinePacker) do this. Simple UV-AABB cropping produces wrong textures for scattered UV islands.

3. **Decimate before splitting** -- generate LOD meshes from the full original mesh (preserving global topology), then split each LOD independently. This is obj2tiles' approach and produces the best LOD quality. Mago3d-tiler decimates after splitting but compensates with half-edge topology awareness.

4. **Native performance** -- meshoptimizer at full native speed (no WASM overhead), memory-mapped file I/O, rayon parallelism across all cores.

5. **f64 transforms, f32 storage** -- all coordinate transforms and georeferencing use f64 precision. Only final vertex storage uses f32 (glTF spec). This avoids the precision loss that F32-only pipelines suffer at UTM scale.

## Project Structure

```
src/
  main.rs                         # CLI entry point (clap derive)
  lib.rs                          # Public library API
  pipeline.rs                     # Pipeline orchestration (4 stages)
  config.rs                       # Configuration types
  error.rs                        # Error types (thiserror)

  ingestion/                      # Stage 1: Data ingestion
    mod.rs                        # Ingestion orchestrator
    obj_parser.rs                 # Memory-mapped OBJ parser
    gltf_loader.rs                # glTF/GLB loader (gltf crate)
    ply_loader.rs                 # PLY parser (ASCII + binary)
    mtl_parser.rs                 # MTL material parser
    georef.rs                     # Georeferencing auto-detection

  transform/                      # Stage 2: Coordinate transforms
    mod.rs                        # Transform orchestrator
    coordinates.rs                # Unit scaling, axis conversion, centering
    ecef.rs                       # WGS84/ECEF conversions
    projection.rs                 # CRS projection (proj crate)

  tiling/                         # Stage 3: Tiling and output
    mod.rs                        # Tiling orchestrator + LOD generation
    octree.rs                     # Adaptive octree subdivision
    triangle_clipper.rs           # Sutherland-Hodgman triangle clipping
    simplifier.rs                 # meshoptimizer simplification
    atlas_repacker.rs             # Per-island UV atlas repacking + bin packing
    glb_writer.rs                 # GLB generation (gltf-json)
    tileset_writer.rs             # tileset.json generation + validation
    texture_compress.rs           # Texture compression (WebP/KTX2)

  types/                          # Shared data types
    mod.rs                        # Re-exports
    mesh.rs                       # IndexedMesh, Vertex, Triangle
    material.rs                   # PBRMaterial, MaterialLibrary, TextureData
    tile.rs                       # TileNode, BoundingBox
    feature.rs                    # Feature (geometry unit)

  server/                         # HTTP service (behind "server" feature flag)
    mod.rs                        # axum router + handlers
    jobs.rs                       # Job queue and worker pool
```

## Core Data Types

### IndexedMesh

The fundamental geometry container. All buffers are contiguous `Vec<f32>`/`Vec<u32>` for zero-copy interop with meshoptimizer and glTF writers.

```rust
pub struct IndexedMesh {
    pub positions: Vec<f32>,     // [x,y,z, x,y,z, ...] -- interleaved
    pub normals: Vec<f32>,       // [nx,ny,nz, ...] or empty
    pub uvs: Vec<f32>,           // [u,v, u,v, ...] or empty
    pub colors: Vec<f32>,        // [r,g,b,a, ...] or empty
    pub indices: Vec<u32>,       // triangle indices
    pub material_index: Option<usize>,
}
```

### TileNode

Octree hierarchy node.

```rust
pub struct TileNode {
    pub address: String,             // "root", "0", "0_1", "0_1_3"
    pub level: u32,
    pub bounds: BoundingBox,
    pub geometric_error: f64,
    pub content: Option<TileContent>, // GLB bytes + URI
    pub children: Vec<TileNode>,
}
```

### MaterialLibrary

```rust
pub struct MaterialLibrary {
    pub materials: Vec<PBRMaterial>,
    pub textures: Vec<TextureData>,
}

pub struct TextureData {
    pub data: Vec<u8>,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
}
```

## Pipeline Stages

### Stage 1: Ingestion

**OBJ path** (memory-mapped, two-pass):
1. `mmap` the file, scan for vertex/face counts and material group byte offsets
2. Pre-allocate `Vec<f32>` for positions, normals, texcoords from counts
3. Parse geometry directly into pre-allocated buffers using rayon parallel chunks
4. Parse MTL and load texture images

**glTF path**: Use `gltf` crate to load meshes, materials, textures.

**PLY path**: Parse header, then binary/ASCII body into `IndexedMesh`.

**Georeferencing**: Scan input directory for `offset.xyz`, `metadata.xml`, `.prj`.

### Stage 2: Transform

All transforms use f64 precision.

```
Source (mm/cm/ft/in) --[unit scale]--> Meters (f64)
Meters Y-up          --[axis swap]---> Meters Z-up (f64)
Z-up                 --[rotation]----> True-north-aligned (f64)
Aligned              --[centering]---> Origin-centered (f64)
                                       then cast to f32 for storage
```

Root transform (4x4 f64 matrix in tileset.json):
```
Projected CRS --[proj]--> WGS84 (lon, lat)
WGS84         --[ECEF]--> ECEF Cartesian
ECEF          --[ENU]---> East-North-Up rotation at origin
```

### Stage 3: Tiling

This stage follows the obj2tiles architecture: **decimate first, then split**.

**Sub-stages:**

1. **LOD mesh generation** (parallel per LOD level):
   - LOD 0: original mesh (no simplification)
   - LOD 1: simplify to ~50% via `meshopt::simplify()` with `LockBorder`
   - LOD 2: simplify to ~25%
   - LOD N: simplify to `0.5^N`
   - Each LOD is an independent full-mesh simplification from the original

2. **Spatial subdivision** (parallel per LOD):
   - Build octree from bounding box
   - For each triangle: clip against octant boundaries via Sutherland-Hodgman
   - Vertex deduplication at boundaries via position hash map
   - Result: per-octant `IndexedMesh` with properly clipped geometry

3. **Texture atlas repacking** (parallel per tile):
   - Find connected face groups via edge adjacency (half-edge traversal)
   - Compute UV bounding rectangle per island
   - Bin-pack islands into new atlas via guillotine packing
   - Add 2-5 pixel bleed ring per island
   - Remap all UVs to atlas coordinates

4. **GLB generation** (parallel per tile):
   - Build glTF document via `gltf-json`
   - Attach mesh primitives, PBR materials, compressed textures
   - Write binary GLB

5. **tileset.json output**:
   - Build tile hierarchy matching octree + LOD structure
   - Bounding volumes as oriented boxes
   - Geometric error from simplification ratio and bounds diagonal
   - REPLACE refinement, root transform as 4x4 column-major matrix

### Stage 4: Validation (optional)

Read back tileset.json and verify 3D Tiles 1.1 compliance.

## Key Algorithms

### Sutherland-Hodgman Triangle Clipping

For each axis-aligned plane (6 planes per octant: min/max X, Y, Z):
1. Classify each polygon vertex as inside/outside the half-space
2. For each edge crossing the plane, compute intersection point at parameter `t`
3. Interpolate all attributes (position, normal, UV, color) at `t`
4. Fan-triangulate the resulting polygon
5. Deduplicate boundary vertices via position hash

This runs on every triangle regardless of mesh size. No centroid fallback.

### UV Island Detection + Bin Packing

Per-tile atlas repacking (following obj2tiles and mago3d-tiler):

1. **Build edge adjacency**: For each triangle edge, record which faces share it
2. **BFS connected components**: Group faces connected by shared edges into UV islands
3. **Compute island UV bounds**: Min/max UV per island, padded by 2-5 pixels
4. **Guillotine bin packing**: Pack island rectangles into a new atlas. Split free space by longest axis. Minimize total atlas area.
5. **Extract + composite**: Copy pixel regions from source texture to atlas positions, adding bleed ring
6. **UV remap**: Transform each vertex's UV from source space to atlas space

### meshoptimizer Simplification

Native FFI call to meshoptimizer's quadric error metric edge collapse:

```rust
let simplified_indices = meshopt::simplify(
    &indices,
    &vertex_adapter,
    target_count,
    target_error,
    SimplifyOptions::LockBorder,
    Some(&mut result_error),
);
```

Key parameters:
- `LockBorder`: prevents collapsing boundary edges (preserves tile boundary geometry)
- `target_error`: 0.01 = 1% deformation tolerance
- Returns new index buffer referencing original vertices
- Follow with `optimize_vertex_fetch()` to compact the vertex buffer

## Parallelism Model

```
rayon global thread pool (all cores)
  |
  +-- Stage 1: par_chunks for OBJ vertex parsing
  +-- Stage 2: par_iter over features for transforms
  +-- Stage 3:
  |     +-- par_iter over LOD levels for simplification
  |     +-- par_iter over octants for triangle clipping
  |     +-- par_iter over tiles for atlas repacking
  |     +-- par_iter over tiles for GLB generation
  +-- Stage 4: sequential validation
```

All stages except validation use rayon's work-stealing parallelism. No manual thread management.

## Memory Model

- **Memory-mapped OBJ parsing**: File stays on disk, kernel pages data in on demand
- **Pre-allocated buffers**: Vertex counts from scan pass size all allocations upfront
- **No GC**: Deterministic deallocation when buffers go out of scope
- **Zero-copy where possible**: meshoptimizer operates on slices of existing buffers
- **f64 for transforms, f32 for storage**: Avoids precision loss without doubling vertex memory

Peak memory for a 169M-vertex OBJ: ~5-8GB (vs 12-16GB in the Node.js version).

## Error Handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum PhotoTilerError {
    #[error("Input error: {0}")]
    Input(String),
    #[error("Georeferencing error: {0}")]
    Georeference(String),
    #[error("Transform error: {0}")]
    Transform(String),
    #[error("Tiling error: {0}")]
    Tiling(String),
    #[error("Output error: {0}")]
    Output(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
```

All functions return `Result<T, PhotoTilerError>`. The `anyhow` crate is used at the CLI level for context-rich error messages.

## Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` | CLI argument parsing |
| `meshopt` | Native mesh simplification (FFI to meshoptimizer C) |
| `gltf` + `gltf-json` | glTF 2.0 loading and GLB writing |
| `tobj` | OBJ/MTL parsing |
| `ply-rs` | PLY parsing |
| `image` | Texture load/crop/compress (JPEG, PNG, WebP) |
| `glam` | 3D math (Vec3, Mat4, Quat) |
| `rayon` | Data-parallel iteration |
| `proj` | CRS projection (EPSG to WGS84) |
| `memmap2` | Memory-mapped file I/O |
| `serde` + `serde_json` | JSON serialization for tileset.json |
| `tracing` | Structured logging |
| `thiserror` | Error type derivation |
| `axum` | HTTP service (optional, behind `server` feature) |

## Reference Implementations

Design decisions are informed by:

- **[obj2tiles](https://github.com/OpenDroneMap/Obj2Tiles)** (C#) -- decimate-first pipeline, always-clip triangle splitting, MaxRectanglesBinPack atlas repacking, quadric error simplification
- **[mago3d-tiler](https://github.com/Gaia3D/mago-3d-tiler)** (Java) -- half-edge plane cutting, guillotine atlas packing, edge-collapse + vertex clustering LOD, GPU-accelerated remeshing
- **[cesium-native](https://github.com/CesiumGS/cesium-native)** (C++) -- 3D Tiles 1.1 spec reference implementation, tileset.json writer, GLB processing
