# Photo-Tiler Roadmap

## Milestone 1: Foundation (Core Types + CLI Skeleton)

**Goal**: Project compiles, CLI parses args, core types defined.

- [ ] Rust project scaffolding (`src/main.rs`, `src/lib.rs`, module structure)
- [ ] `config.rs` -- `PipelineConfig`, `TilingConfig`, `TextureConfig` structs with clap derive
- [ ] `error.rs` -- `PhotoTilerError` enum with thiserror
- [ ] `types/mesh.rs` -- `IndexedMesh` struct with position/normal/UV/color/index buffers
- [ ] `types/material.rs` -- `PBRMaterial`, `MaterialLibrary`, `TextureData`
- [ ] `types/tile.rs` -- `TileNode`, `BoundingBox`
- [ ] `main.rs` -- clap CLI matching all flags from the README
- [ ] `pipeline.rs` -- pipeline skeleton that calls stage stubs
- [ ] Unit tests for config parsing and type construction

## Milestone 2: Ingestion (File Parsers)

**Goal**: Load OBJ, glTF, and PLY files into `IndexedMesh`.

- [ ] `ingestion/obj_parser.rs` -- memory-mapped two-pass OBJ parser
  - Pass 1: scan for counts and material group byte offsets via `memmap2`
  - Pass 2: parse vertices/faces into pre-allocated `Vec<f32>`/`Vec<u32>`
  - Parallel vertex parsing via rayon `par_chunks` on mmap'd regions
  - MTL parsing and texture loading
  - Handle fan-triangulation of quads/n-gons
- [ ] `ingestion/gltf_loader.rs` -- load via `gltf` crate, extract meshes + PBR materials
- [ ] `ingestion/ply_loader.rs` -- ASCII and binary (LE/BE) PLY parsing
- [ ] `ingestion/georef.rs` -- auto-detect `offset.xyz`, `metadata.xml`, `.prj`
- [ ] Integration tests with test data (`test/Model.obj`, `test/Model.mtl`, `test/Model.jpg`)
- [ ] Benchmark: OBJ parse throughput (MB/s) vs Node.js baseline

## Milestone 3: Coordinate Transforms

**Goal**: Transform from source CRS to origin-centered with ECEF root matrix.

- [ ] `transform/coordinates.rs` -- unit scaling (mm/cm/m/ft/in), Y-up to Z-up, centering
- [ ] `transform/ecef.rs` -- geodetic to ECEF, ENU rotation matrix
- [ ] `transform/projection.rs` -- CRS projection via `proj` crate (EPSG to WGS84)
- [ ] All transforms in f64, final vertex cast to f32
- [ ] Root transform 4x4 matrix assembly (pre-translation + ENU rotation + ECEF translation)
- [ ] Unit tests against known UTM-to-ECEF conversions

## Milestone 4: Mesh Simplification (Decimate-First Pipeline)

**Goal**: Generate LOD meshes from full original mesh using native meshoptimizer.

- [ ] `tiling/simplifier.rs` -- wrapper around `meshopt::simplify()`
  - `simplify_mesh(mesh, target_ratio, lock_border) -> IndexedMesh`
  - Compact unused vertices after simplification
  - Preserve UV seam edges via `LockBorder` flag
- [ ] LOD level generation: LOD 0 = original, LOD N = `0.5^N` ratio
- [ ] Geometric error calculation: `diagonal * (1.0 - ratio) * 0.5`
- [ ] Benchmark: simplification speed (triangles/sec) vs WASM baseline

## Milestone 5: Spatial Subdivision (Triangle Clipping)

**Goal**: Split each LOD mesh into octree tiles with correct geometry at boundaries.

- [ ] `tiling/triangle_clipper.rs` -- Sutherland-Hodgman polygon clipping
  - `clip_polygon_by_plane(vertices, plane) -> Vec<Vertex>`
  - Linear interpolation of all attributes at intersection points
  - Fan-triangulation of clipped polygons
  - No centroid fallback -- always clip
- [ ] `tiling/octree.rs` -- adaptive octree subdivision
  - Recursive 8-way bounding box split
  - Stop conditions: max depth, triangle count, min tile size
  - Per-octant triangle assignment via clipping
  - Vertex deduplication at boundaries via position hash map
- [ ] Parallel clipping: rayon `par_iter` over triangles per octant
- [ ] Integration test: verify no gaps at tile boundaries (check boundary vertex sharing)

## Milestone 6: Texture Atlas Repacking

**Goal**: Per-tile UV island detection, bin packing, bleed padding, UV remapping.

- [ ] `tiling/atlas_repacker.rs`
  - Edge adjacency map: for each edge, track which faces share it
  - BFS connected-component island detection
  - Per-island UV bounding rectangle computation
  - Guillotine bin packing algorithm (following mago3d-tiler)
  - Bleed ring extraction (2-5 pixels per island size)
  - UV coordinate remapping to atlas space
  - Atlas image compositing via `image` crate
- [ ] `tiling/texture_compress.rs` -- WebP/KTX2 compression
- [ ] Handle UV coordinates outside [0,1] (wrapping textures)
- [ ] OBJ UV V-flip correction (OBJ V=0 bottom, glTF V=0 top)
- [ ] Integration test: verify UV mapping correctness after repacking

## Milestone 7: GLB Writer + tileset.json

**Goal**: Write correct 3D Tiles 1.1 output.

- [ ] `tiling/glb_writer.rs` -- build glTF document via `gltf-json`
  - Create buffer, buffer views, accessors for mesh data
  - Create PBR materials with base color textures
  - Create mesh primitives with POSITION, NORMAL, TEXCOORD_0, indices
  - Assemble GLB binary (12-byte header + JSON chunk + binary chunk)
  - 4-byte alignment padding per chunk
- [ ] `tiling/tileset_writer.rs` -- tileset.json generation
  - 3D Tiles 1.1 spec compliance (guided by cesium-native)
  - Bounding volumes as oriented boxes (12-element array)
  - Geometric error hierarchy
  - REPLACE refinement
  - Root transform as 16-element column-major f64 array
  - Content URIs for each tile with geometry
- [ ] Validation pass: read back tileset.json, verify structure
- [ ] End-to-end test: convert test OBJ, load in viewer

## Milestone 8: Full Pipeline Integration

**Goal**: Complete end-to-end pipeline works for all input formats.

- [ ] Wire all stages together in `pipeline.rs`
- [ ] Progress reporting via `tracing`
- [ ] Dry-run mode (scan + report without processing)
- [ ] Show-georef mode
- [ ] Parallel tile processing: rayon `par_iter` over tiles for repack + GLB
- [ ] Error recovery: continue processing remaining tiles if one fails
- [ ] Test with real photogrammetry datasets (Pix4D, Agisoft, RealityCapture)
- [ ] Performance benchmark vs Node.js version on same dataset

## Milestone 9: HTTP Service

**Goal**: Standalone service that receives jobs via HTTP API.

- [ ] `server/mod.rs` -- axum router with endpoints:
  - `POST /convert` -- submit conversion job
  - `GET /jobs/:id` -- poll job status
  - `GET /jobs/:id/result` -- download result
  - `GET /health` -- liveness check
- [ ] `server/jobs.rs` -- job queue with rayon worker pool
- [ ] Multipart file upload support
- [ ] Job progress streaming via SSE
- [ ] Configurable output: filesystem, S3, or streaming response
- [ ] Dockerfile with multi-stage build (distroless base, ~15MB image)
- [ ] Docker Compose with volume mounts for input/output

## Milestone 10: Production Hardening

**Goal**: Production-ready with monitoring, testing, and documentation.

- [ ] Comprehensive error messages with context for every failure mode
- [ ] Structured JSON logging (tracing-subscriber)
- [ ] Metrics endpoint (Prometheus format) -- processing time, tile count, memory usage
- [ ] Graceful shutdown (drain in-flight jobs)
- [ ] Input validation (reject malformed OBJ, oversized textures, etc.)
- [ ] Memory usage caps (configurable max memory, reject jobs that would exceed)
- [ ] CI/CD: GitHub Actions for build, test, lint, benchmark
- [ ] Cross-platform builds: Linux x86_64, Linux aarch64, macOS arm64
- [ ] Published crate on crates.io
- [ ] Complete API documentation (rustdoc)

## Performance Targets

| Metric | Node.js (old) | Rust (target) |
|--------|--------------|---------------|
| OBJ parse (16GB) | 10-18 min | <2 min |
| Simplify 2M triangles | 2-6s (WASM) | <2s (native) |
| Peak memory (169M verts) | 12-16 GB | 5-8 GB |
| Total pipeline (169M tri) | ~70 min | <10 min |
| CPU utilization | ~12% (1 core) | ~90% (all cores) |
| Docker image | ~150 MB | <15 MB |
| Cold start | ~100 ms | <10 ms |

## Architecture Decisions Log

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | No GC pauses, native meshoptimizer, memory safety, 5-10x I/O, true parallelism |
| Pipeline order | Decimate first, split second | Preserves global mesh topology during simplification (obj2tiles approach) |
| Triangle splitting | Always clip (no centroid fallback) | Prevents gaps at tile boundaries (proven by obj2tiles + mago3d) |
| Texture repacking | Per-island bin packing | Handles scattered UV islands correctly (proven by obj2tiles + mago3d) |
| Output format | Direct GLB (no b3dm wrapper) | 3D Tiles 1.1 standard, b3dm is deprecated |
| Coordinate precision | f64 transforms, f32 storage | Avoids UTM-scale precision loss without doubling vertex memory |
| 3D Tiles writer | Custom (guided by cesium-native) | No mature Rust crate exists; cesium-native is the spec reference |
| Simplification | meshopt crate (native FFI) | Same algorithm as Node.js version but at full native speed |
| HTTP framework | axum (behind feature flag) | Lightweight, async, production-proven |
