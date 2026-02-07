# Photo-Tiler Usage Manual

## Table of Contents

1. [Installation](#installation)
2. [Pix4D Workflow](#pix4d-workflow)
3. [Agisoft Metashape Workflow](#agisoft-metashape-workflow)
4. [Input Formats](#input-formats)
5. [Georeferencing](#georeferencing)
6. [Tiling Options](#tiling-options)
7. [LOD and Simplification](#lod-and-simplification)
8. [Texture Handling](#texture-handling)
9. [Compression](#compression)
10. [Validation](#validation)
11. [Library API](#library-api)
12. [HTTP Service](#http-service)
13. [Docker Deployment](#docker-deployment)
14. [Troubleshooting](#troubleshooting)

---

## Installation

### Prerequisites

- **Rust** >= 1.85 (install via [rustup](https://rustup.rs))
- **PROJ** library for CRS transforms:
  - macOS: `brew install proj`
  - Ubuntu/Debian: `apt install libproj-dev`
  - Fedora: `dnf install proj-devel`

### Build from source

```bash
git clone <repository-url>
cd Photogrammetry-Tiler
cargo build --release
```

The binary is at `target/release/photo-tiler`.

### Install globally

```bash
cargo install --path .
# Now `photo-tiler` is available globally
photo-tiler --help
```

### Run without installing

```bash
cargo run --release -- --help
```

---

## Pix4D Workflow

This is the primary use case. Pix4D exports textured OBJ meshes with georeferencing metadata.

### Step 1: Locate your Pix4D export

```
pix4d_project/
  3_dsm_ortho/
    2_mesh/
      model.obj           # Geometry
      model.mtl           # Material library
      model_texture_0.jpg # Texture atlas(es)
      offset.xyz          # Georef offset (easting, northing, elevation)
      model.prj           # Coordinate system definition
```

### Step 2: Inspect the data (dry run)

```bash
photo-tiler -i ./pix4d_project/3_dsm_ortho/2_mesh/model.obj \
  --dry-run --units m
```

### Step 3: Check georeferencing detection

```bash
photo-tiler -i ./pix4d_project/3_dsm_ortho/2_mesh/model.obj --show-georef
```

### Step 4: Convert

```bash
photo-tiler -i ./pix4d_project/3_dsm_ortho/2_mesh/model.obj \
  -o ./tileset \
  --units m \
  --validate \
  -v
```

### Step 5: Manual georeferencing (if auto-detection fails)

```bash
photo-tiler -i model.obj -o ./tileset \
  --units m \
  --epsg 32636 \
  --easting 500000 \
  --northing 2800000 \
  --elevation 100 \
  --validate
```

### Step 6: View in CesiumJS

```javascript
const tileset = await Cesium.Cesium3DTileset.fromUrl('./tileset/tileset.json');
viewer.scene.primitives.add(tileset);
viewer.zoomTo(tileset);
```

---

## Agisoft Metashape Workflow

### Export from Agisoft

1. File > Export > Export Model
2. Choose OBJ or glTF format
3. Enable "Export metadata" in options

### Convert

```bash
photo-tiler -i ./agisoft_export/model.obj \
  -o ./tileset \
  --units m \
  --metadata-xml ./agisoft_export/metadata.xml \
  --validate
```

---

## Input Formats

### OBJ Files

- **Units required**: OBJ has no unit metadata. Always specify `--units`.
- **MTL file**: Automatically loaded if referenced in the OBJ header.
- **Textures**: JPEG/PNG texture files referenced by the MTL are embedded in GLB output.
- **Large files**: Memory-mapped parser handles multi-GB OBJ files efficiently.

### glTF/GLB Files

glTF 2.0 files with PBR materials. The glTF spec defines meters as the unit.

```bash
photo-tiler -i model.gltf -o ./output
```

### PLY Files

PLY files with vertex positions and optional vertex colors. No texture support.

```bash
photo-tiler -i model.ply -o ./output --units m --no-textures
```

---

## Georeferencing

### Auto-detection

Photo-Tiler searches the input directory for:

1. **`offset.xyz`** (Pix4D) -- 3 values: X, Y, Z offset in projected coordinates
2. **`metadata.xml`** (Agisoft/DJI) -- XML with EPSG, transform matrix, offset
3. **`.prj` files** (WKT) -- Coordinate system definition with EPSG

Priority: `metadata.xml` > `offset.xyz` > `.prj`

### Manual georeferencing

```bash
photo-tiler -i model.obj -o ./output --units m \
  --epsg 32636 \
  --easting 500000.0 \
  --northing 2800000.0 \
  --elevation 100.0 \
  --true-north 1.5
```

### Coordinate transform pipeline

```
Source coordinates (OBJ, mm/cm/m/ft/in)
  --> Scale to meters (f64 precision)
  --> Y-up to Z-up axis conversion
  --> True north rotation
  --> Center at local origin
  --> Cast to f32 for vertex storage
  --> Root transform: CRS --> WGS84 --> ECEF (f64 4x4 matrix in tileset.json)
```

---

## Tiling Options

### Octree subdivision

| Option | Description | Default |
|--------|-------------|---------|
| `--max-triangles <n>` | Stop subdividing when a node has fewer triangles | 100,000 |
| `--max-depth <n>` | Maximum octree depth (root = 0) | 6 |
| `-j, --threads <n>` | Worker thread count | all cores |

### Tuning for different model sizes

**Small models** (< 1M triangles):
```bash
--max-depth 3 --max-triangles 200000
```

**Medium models** (1-10M triangles):
```bash
--max-depth 5 --max-triangles 100000
```

**Large models** (> 10M triangles):
```bash
--max-depth 7 --max-triangles 50000
```

---

## LOD and Simplification

Photo-Tiler generates a multi-resolution hierarchy using the REPLACE refinement strategy.

### Pipeline order: Decimate first, then split

Unlike many tilers that split first and simplify fragments, Photo-Tiler follows the obj2tiles approach:

1. Generate LOD meshes from the **full original mesh** (preserving global topology)
2. Spatially split **each LOD independently** into octree tiles

This produces better LODs because the simplifier has full mesh context and can make globally optimal edge-collapse decisions.

### LOD levels

- **LOD 0**: Original mesh (100% triangles)
- **LOD 1**: ~50% triangles
- **LOD 2**: ~25% triangles
- **LOD N**: `0.5^N` triangles

### Geometric error

Each tile's `geometricError` tells the viewer the maximum screen-space error if this tile is rendered without loading children:

- **Root**: Computed from mesh diagonal and simplification ratio
- **Intermediate**: Proportional to LOD simplification level
- **Leaf**: 0 (full resolution, no further refinement)

---

## Texture Handling

### Per-island texture atlas repacking

Photogrammetry tools produce large texture atlases (8192x8192+) with many small UV islands scattered across the atlas. Photo-Tiler's atlas repacker:

1. Detects connected UV islands via edge adjacency graph traversal
2. Computes bounding rectangle per island
3. Bin-packs islands into a new compact per-tile atlas (guillotine packing)
4. Adds bleed-ring padding (2-5 pixels) around each island to prevent sampling artifacts
5. Remaps all UV coordinates to the new atlas

This typically reduces per-tile texture data by 70-90%.

### Texture formats

| Format | Flag | Use Case |
|--------|------|----------|
| WebP | `--texture-format webp` | Good compression, wide browser support |
| KTX2 | `--texture-format ktx2` | GPU-compressed, best for WebGL streaming |
| Original | `--texture-format original` | No re-compression |

### Quality and size limits

```bash
# WebP at 90% quality, max 4096px textures
photo-tiler -i model.obj -o ./output --units m \
  --texture-format webp --texture-quality 90 --texture-max-size 4096

# Skip textures entirely
photo-tiler -i model.obj -o ./output --units m --no-textures
```

---

## Compression

### Draco mesh compression

| Option | Description | Default |
|--------|-------------|---------|
| `--no-draco` | Disable Draco compression | enabled |
| `--draco-level <n>` | Compression level 1 (fast) to 10 (best) | 7 |

```bash
# Maximum compression
photo-tiler -i model.obj -o ./output --units m --draco-level 10

# No compression (for debugging)
photo-tiler -i model.obj -o ./output --units m --no-draco
```

---

## Validation

```bash
photo-tiler -i model.obj -o ./output --units m --validate
```

Checks:
- Asset version is 1.1
- Root tile exists with bounding volume
- All geometric errors >= 0
- All content tiles have URIs
- Bounding volumes present on all tiles
- Child tile structure is valid

---

## Library API

### Quick convert

```rust
use photo_tiler::{Pipeline, PipelineConfig, Units};

let config = PipelineConfig {
    input: "model.obj".into(),
    output: "./output".into(),
    units: Units::Meters,
    ..Default::default()
};

let result = Pipeline::run(&config)?;
println!("Generated {} tiles in {:?}", result.tile_count, result.duration);
```

### Full pipeline control

```rust
use photo_tiler::{Pipeline, PipelineConfig, Units, Georeference, TilingConfig};

let config = PipelineConfig {
    input: "model.obj".into(),
    output: "./tileset".into(),
    units: Units::Meters,
    georeference: Some(Georeference {
        epsg: 32636,
        easting: 500000.0,
        northing: 2800000.0,
        elevation: 100.0,
        ..Default::default()
    }),
    tiling: TilingConfig {
        max_triangles_per_tile: 50000,
        max_depth: 8,
        ..Default::default()
    },
    ..Default::default()
};

let result = Pipeline::run(&config)?;
```

---

## HTTP Service

Build with the `server` feature:

```bash
cargo build --release --features server
photo-tiler serve --port 8080
```

### Submit a conversion job

```bash
curl -X POST http://localhost:8080/convert \
  -F "input=@model.obj" \
  -F "mtl=@model.mtl" \
  -F "texture=@model_texture_0.jpg" \
  -F 'config={"units":"m","epsg":32636}'
```

### Poll job status

```bash
curl http://localhost:8080/jobs/<job-id>
```

### Download result

```bash
curl -o tileset.zip http://localhost:8080/jobs/<job-id>/result
```

---

## Docker Deployment

### Build

```bash
docker build -t photo-tiler .
```

### Run as CLI

```bash
docker run --rm -v ./data:/data photo-tiler \
  -i /data/model.obj -o /data/output --units m
```

### Run as service

```bash
docker run -d -p 8080:8080 photo-tiler serve --port 8080
```

Image size: ~15MB. Cold start: <10ms.

---

## Troubleshooting

### "Units are required for OBJ input"

OBJ files have no unit metadata. Specify `--units`:

```bash
photo-tiler -i model.obj -o ./output --units m
```

Common conventions: Pix4D = meters, RealityCapture = meters or centimeters.

### Tileset not positioned correctly in Cesium

1. Check georeferencing: `photo-tiler -i model.obj --show-georef`
2. Verify the EPSG code matches your CRS
3. Verify easting/northing match the offset.xyz values

### Textures appear wrong

- Check that texture files are in the same directory as the MTL file
- Verify UV coordinates are in 0-1 range (use `--dry-run` to inspect)
- Try `--texture-format original` to rule out compression issues

### Large model performance

For models > 5M triangles, ensure you have sufficient RAM (roughly 5-8GB for 169M vertices). Tune with:

```bash
photo-tiler -i model.obj -o ./output --units m \
  --max-depth 6 --max-triangles 100000 \
  --texture-max-size 1024 \
  -j 8
```
