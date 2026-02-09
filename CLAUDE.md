# Photogrammetry-Tiler - Project Memory

## Architecture Overview
Rust CLI tool that converts photogrammetry meshes (OBJ/glTF/PLY) to OGC 3D Tiles 1.1 format.

### Pipeline Stages
1. **Ingestion** (`src/ingestion/`) - Parse OBJ/glTF/PLY, detect georeferencing
2. **Transform** (`src/transform/`) - Unit scaling, axis swap, centering, ECEF root transform
3. **Tiling** (`src/tiling/`) - LOD generation, octree spatial split, GLB writing, tileset.json
4. **Validation** (`src/pipeline.rs`) - Walk tileset.json, validate GLBs exist and parse

### Key Files
- `src/tiling/tileset_writer.rs` - Builds tile hierarchy, writes tileset.json + GLBs
- `src/tiling/octree.rs` - Octree spatial subdivision (split_mesh, build_octree)
- `src/tiling/lod.rs` - LOD chain generation via meshopt simplification
- `src/tiling/simplifier.rs` - meshopt simplify wrapper + vertex compaction
- `src/tiling/triangle_clipper.rs` - Sutherland-Hodgman triangle clipping for octree split
- `src/tiling/glb_writer.rs` - GLB serialization with EXT_meshopt_compression
- `src/tiling/atlas_repacker.rs` - UV island detection, guillotine bin packing, atlas compositing
- `src/types/tile.rs` - BoundingBox, TileContent, TileNode types
- `src/types/mesh.rs` - IndexedMesh type
- `src/config.rs` - CLI args, TilingConfig, TextureConfig
- `src/pipeline.rs` - Pipeline orchestrator (4 stages)

### Config Defaults
- `max_triangles_per_tile`: 500,000 (TOO HIGH - should be 50K-100K)
- `max_depth`: 6
- `max_lod_levels`: 4 (hardcoded in pipeline.rs:84)

## Critical Issue: Flat Tileset Hierarchy (February 2026)

### Problem
The tileset.json is structured as a single-child LOD chain (3 levels) that explodes
into 1400+ flat leaf tiles. This is the opposite of what Cesium Native expects.

### Current (Wrong) Tree Shape
```
Depth 0: 1 tile  (root.glb, entire model at LOD 3)      geoError=273
Depth 1: 1 tile  (tiles/2/, entire model at LOD 2)       geoError=260
Depth 2: 1 tile  (tiles/2/2_1/, entire model at LOD 1)   geoError=208
Depth 3: 8 tiles (octree routing nodes, NO content)      geoError=27
Depth 4-8: 1400+ leaf tiles (LOD 0 spatially split)      geoError=0
```

### Root Causes in Code
1. **`build_lod_children()` (tileset_writer.rs:229-276)**: Returns `vec![single_node]` -
   always creates exactly ONE child per LOD level instead of spatially subdividing
2. **Octree only at leaf level** (tileset_writer.rs:243): `build_octree()` called only
   on the finest LOD mesh, creating flat spatial subdivision at the bottom
3. **Internal octree nodes have no content** (octree.rs:147, tileset_writer.rs:313,366):
   `mesh: IndexedMesh::default()` / `drop(mesh)` - routing nodes can't be rendered

### Correct Architecture (Per Cesium Native & 3D Tiles 1.1 Spec)
Each tree level should combine BOTH spatial subdivision AND LOD simplification:

```
Root (LOD 3, ~2K tris, entire model, geoError=50.0)
├── Octant 0 (LOD 2, ~4K tris, NW region, geoError=25.0)
│   ├── Sub-octant 0 (LOD 1, ~16K tris, geoError=12.5)
│   │   ├── Leaf 0 (LOD 0, ~30K tris, geoError=0.0)
│   │   └── Leaf 1 ...
│   └── Sub-octant 1 ...
├── Octant 1 (LOD 2, NE region, geoError=25.0)
└── ... (4-8 children per node)
```

### Cesium Native SSE Algorithm
```
SSE (pixels) = geometricError * viewportHeight / (2 * distance * tan(fov/2))
If SSE >= 16px -> REFINE (load children)
If SSE < 16px  -> RENDER this tile
```
- `maximumSimultaneousTileLoads`: 20 (flat tree = bottleneck)
- `loadingDescendantLimit`: 20 (flat tree = falls back to blurry root)

### Target Numbers
- Tree depth: 4-6 levels
- Branching factor: 4-8 per node (octree)
- Leaf tile triangles: 50K-100K
- Every internal node must have content (simplified mesh of its spatial region)
- geometricError halves at each level
- Total tiles for typical model: 100-600
