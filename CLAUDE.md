# Photogrammetry-Tiler - Project Memory

## Architecture Overview
Rust CLI tool that converts photogrammetry meshes (OBJ/glTF/PLY) to OGC 3D Tiles 1.1 format.

### Pipeline Stages
1. **Ingestion** (`src/ingestion/`) - Parse OBJ/glTF/PLY, detect georeferencing
2. **Transform** (`src/transform/`) - Unit scaling, axis swap, centering, ECEF root transform
3. **Tiling** (`src/tiling/`) - LOD generation, octree spatial split, GLB writing, tileset.json
4. **Validation** (`src/pipeline.rs`) - Walk tileset.json, validate GLBs exist and parse

### Key Files
- `src/tiling/tileset_writer.rs` - Builds tile hierarchy (rayon-parallelized), writes tileset.json + GLBs
- `src/tiling/octree.rs` - Octree spatial subdivision (split_mesh, build_octree)
- `src/tiling/lod.rs` - LOD chain generation via meshopt simplification
- `src/tiling/simplifier.rs` - meshopt simplify wrapper + vertex compaction
- `src/tiling/triangle_clipper.rs` - Sutherland-Hodgman triangle clipping with DedupKey (pos+UV+normal)
- `src/tiling/glb_writer.rs` - GLB serialization with EXT_meshopt_compression + KHR_texture_basisu
- `src/tiling/atlas_repacker.rs` - UV island detection, guillotine bin packing, vertex dedup across islands
- `src/tiling/texture_compress.rs` - WebP/PNG encoding + KTX2/UASTC via basis-universal (optional)
- `src/types/tile.rs` - BoundingBox, TileContent, TileNode types
- `src/types/mesh.rs` - IndexedMesh type
- `src/config.rs` - CLI args, TilingConfig, TextureConfig
- `src/pipeline.rs` - Pipeline orchestrator (4 stages)

### Config Defaults
- `max_triangles_per_tile`: 100,000
- `max_depth`: 6
- `max_lod_levels`: 4 (hardcoded in pipeline.rs)

### Feature Flags
- `ktx2` - Enables KTX2/UASTC texture compression via `basis-universal` crate

## Status: Working Pipeline (February 2026)

### Current State
The pipeline is fully operational with correct tileset hierarchy, parallel execution,
and high-quality texture output. Tested on a 169M triangle model (16.8 GB OBJ):

- **9,552 tiles** generated with validation passing
- **34 minutes** total runtime on 11-core Apple Silicon (previously 4.3 hours single-threaded)
- **7+ cores** utilized via rayon work-stealing parallelism
- Correct textures with no black artifacts or zigzag seams

### Tileset Hierarchy (Correct)
Each tree level combines spatial subdivision AND LOD simplification:

```
Root (simplified, entire model, geoError=high)
├── Octant 0 (simplified, NW region, geoError=mid)
│   ├── Sub-octant 0 (less simplified, geoError=low)
│   │   ├── Leaf 0 (full detail, geoError=0.0)
│   │   └── Leaf 1 ...
│   └── Sub-octant 1 ...
├── Octant 1 (simplified, NE region, geoError=mid)
└── ... (4-8 children per node)
```

Every internal node has content (simplified mesh). Geometric error halves at each level.
REPLACE refinement. 3D Tiles 1.1 with GLB content (no B3DM).

### Recent Fixes (Milestone 9 commit)

**Texture quality fixes:**
- `DedupKey` in triangle_clipper.rs: hashes position + UV + normal (was position-only, corrupted UV seams)
- `remap_uvs_with_dedup()` in atlas_repacker.rs: duplicates vertices shared across UV islands (was first-island-wins)
- Half-texel inset in UV remapping: prevents bilinear filter bleed into atlas padding
- Corner bleed fill: replicates corner pixels into pad x pad corner rectangles

**Performance optimizations:**
- `into_par_iter()` in tileset_writer.rs: parallel octant child processing via rayon (7.5x speedup)
- AABB pre-filter in triangle_clipper.rs: skips non-overlapping octants before clipping (3-5x clipper speedup)
- Relaxed simplification for depth >= 3: ratio 0.5, no border lock (faster for coarse LODs)
- Scanline bulk copy in atlas compositing: `copy_from_slice()` for contiguous UV ranges

**KTX2 support (optional, behind `ktx2` feature flag):**
- `basis-universal` crate for UASTC encoding
- `KHR_texture_basisu` glTF extension in GLB writer
- Quality level mapping from config (0-100 to UASTC levels)
