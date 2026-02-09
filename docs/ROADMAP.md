# Photo-Tiler Roadmap

## Milestone 1: Foundation (Core Types + CLI Skeleton) -- COMPLETE

**Goal**: Project compiles, CLI parses args, core types defined.

- [x] Rust project scaffolding (`src/main.rs`, `src/lib.rs`, module structure)
- [x] `config.rs` -- `PipelineConfig`, `TilingConfig`, `TextureConfig` structs with clap derive
- [x] `error.rs` -- `PhotoTilerError` enum with thiserror
- [x] `types/mesh.rs` -- `IndexedMesh` struct with position/normal/UV/color/index buffers
- [x] `types/material.rs` -- `PBRMaterial`, `MaterialLibrary`, `TextureData`
- [x] `types/tile.rs` -- `TileNode`, `BoundingBox`
- [x] `main.rs` -- clap CLI matching all flags from the README
- [x] `pipeline.rs` -- pipeline skeleton that calls stage stubs
- [x] Unit tests for config parsing and type construction

## Milestone 2: Ingestion (File Parsers) -- COMPLETE

**Goal**: Load OBJ, glTF, and PLY files into `IndexedMesh`.

- [x] `ingestion/obj_parser.rs` -- memory-mapped two-pass OBJ parser
  - Pass 1: scan for counts and material group byte offsets via `memmap2`
  - Pass 2: parse vertices/faces into pre-allocated `Vec<f32>`/`Vec<u32>`
  - Parallel vertex parsing via rayon `par_chunks` on mmap'd regions
  - MTL parsing and texture loading
  - Handle fan-triangulation of quads/n-gons
- [x] `ingestion/gltf_loader.rs` -- load via `gltf` crate, extract meshes + PBR materials
- [x] `ingestion/ply_loader.rs` -- ASCII and binary (LE/BE) PLY parsing
- [x] `ingestion/georef.rs` -- auto-detect `offset.xyz`, `metadata.xml`, `.prj`
- [x] Integration tests with test data (`test/Model.obj`, `test/Model.mtl`, `test/Model.jpg`)
- [ ] Benchmark: OBJ parse throughput (MB/s) vs Node.js baseline

## Milestone 3: Coordinate Transforms -- COMPLETE

**Goal**: Transform from source CRS to origin-centered with ECEF root matrix.

- [x] `transform/coordinates.rs` -- unit scaling (mm/cm/m/ft/in), Y-up to Z-up, centering
- [x] `transform/ecef.rs` -- geodetic to ECEF, ENU rotation matrix
- [x] `transform/projection.rs` -- CRS projection via `proj` crate (EPSG to WGS84)
- [x] All transforms in f64, final vertex cast to f32
- [x] Root transform 4x4 matrix assembly (pre-translation + ENU rotation + ECEF translation)
- [x] Unit tests against known UTM-to-ECEF conversions

## Milestone 4: Mesh Simplification (Decimate-First Pipeline) -- COMPLETE

**Goal**: Generate LOD meshes from full original mesh using native meshoptimizer.

- [x] `tiling/simplifier.rs` -- wrapper around `meshopt::simplify()`
  - `simplify_mesh(mesh, target_ratio, lock_border) -> IndexedMesh`
  - Compact unused vertices after simplification
  - Preserve UV seam edges via `LockBorder` flag
- [x] LOD level generation: LOD 0 = original, LOD N = `0.5^N` ratio
- [x] Geometric error calculation: `diagonal * (1.0 - ratio) * 0.5`
- [x] Relaxed simplification for deep levels (depth >= 3): ratio 0.5, no border lock
- [ ] Benchmark: simplification speed (triangles/sec) vs WASM baseline

## Milestone 5: Spatial Subdivision (Triangle Clipping) -- COMPLETE

**Goal**: Split each LOD mesh into octree tiles with correct geometry at boundaries.

- [x] `tiling/triangle_clipper.rs` -- Sutherland-Hodgman polygon clipping
  - `clip_polygon_by_plane(vertices, plane) -> Vec<Vertex>`
  - Linear interpolation of all attributes at intersection points
  - Fan-triangulation of clipped polygons
  - No centroid fallback -- always clip
- [x] `tiling/octree.rs` -- adaptive octree subdivision
  - Recursive 8-way bounding box split
  - Stop conditions: max depth, triangle count, min tile size
  - Per-octant triangle assignment via clipping
  - Vertex deduplication at boundaries via `DedupKey` hash (position + UV + normal)
- [x] AABB pre-filter: skip non-overlapping octants before clipping (3-5x speedup)
- [x] Integration test: verify no gaps at tile boundaries (check boundary vertex sharing)
- [x] Test: `split_mesh_preserves_uv_seams` -- verifies UV seam vertices survive splitting

## Milestone 6: Texture Atlas Repacking -- COMPLETE

**Goal**: Per-tile UV island detection, bin packing, bleed padding, UV remapping.

- [x] `tiling/atlas_repacker.rs`
  - Edge adjacency map: for each edge, track which faces share it
  - BFS connected-component island detection (UV-aware edge matching)
  - Per-island UV bounding rectangle computation
  - Guillotine bin packing algorithm (following mago3d-tiler)
  - Bleed ring extraction (2-5 pixels per island size) + corner bleed fill
  - UV coordinate remapping with half-texel inset (prevents bilinear bleed)
  - `remap_uvs_with_dedup()`: duplicates vertices shared across UV islands
  - Atlas image compositing via `image` crate with scanline bulk copy optimization
- [x] `tiling/texture_compress.rs` -- WebP/PNG encoding + KTX2/UASTC (optional `ktx2` feature)
- [x] Handle UV coordinates outside [0,1] (wrapping textures)
- [x] OBJ UV V-flip correction (OBJ V=0 bottom, glTF V=0 top)
- [x] Integration test: verify UV mapping correctness after repacking
- [x] Test: `repack_shared_vertex_across_islands` -- verifies shared vertices get correct UVs

## Milestone 7: GLB Writer + tileset.json -- COMPLETE

**Goal**: Write correct 3D Tiles 1.1 output.

- [x] `tiling/glb_writer.rs` -- build glTF document via `gltf-json`
  - Create buffer, buffer views, accessors for mesh data
  - Create PBR materials with base color textures
  - Create mesh primitives with POSITION, NORMAL, TEXCOORD_0, indices
  - Assemble GLB binary (12-byte header + JSON chunk + binary chunk)
  - 4-byte alignment padding per chunk
  - EXT_meshopt_compression for vertex/index buffer compression
  - KHR_texture_basisu extension when atlas texture is KTX2
- [x] `tiling/tileset_writer.rs` -- tileset.json generation
  - 3D Tiles 1.1 spec compliance (guided by cesium-native)
  - Bounding volumes as oriented boxes (12-element array)
  - Geometric error hierarchy (halves at each level)
  - REPLACE refinement
  - Root transform as 16-element column-major f64 array
  - Content URIs for each tile with geometry
  - Parallel child processing via rayon `into_par_iter()`
- [x] Validation pass: read back tileset.json, verify structure
- [x] End-to-end test: convert test OBJ, load in viewer

## Milestone 8: Full Pipeline Integration -- COMPLETE

**Goal**: Complete end-to-end pipeline works for all input formats.

- [x] Wire all stages together in `pipeline.rs`
- [x] Progress reporting via `tracing`
- [x] Dry-run mode (scan + report without processing)
- [x] Show-georef mode
- [x] Parallel tile processing: rayon `into_par_iter()` over octant children (7.5x speedup)
- [ ] Error recovery: continue processing remaining tiles if one fails
- [x] Test with real photogrammetry datasets (169M tri Pix4D model -- 34 min, 9,552 tiles)
- [x] Performance benchmark: 34 min vs ~4.3 hours single-threaded (7.5x speedup)

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

## Performance Results

| Metric | Node.js (old) | Rust (target) | Rust (actual) |
|--------|--------------|---------------|---------------|
| OBJ parse (16GB) | 10-18 min | <2 min | ~2.5 min |
| Simplify 2M triangles | 2-6s (WASM) | <2s (native) | ~10ms/100K tri |
| Peak memory (169M verts) | 12-16 GB | 5-8 GB | ~10 GB |
| Total pipeline (169M tri) | ~70 min | <10 min | **34 min** |
| CPU utilization | ~12% (1 core) | ~90% (all cores) | **~65% (7+ cores)** |
| Docker image | ~150 MB | <15 MB | TBD |
| Cold start | ~100 ms | <10 ms | TBD |

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
