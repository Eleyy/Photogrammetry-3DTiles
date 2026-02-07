# Photo-Tiler

High-performance photogrammetry mesh to OGC 3D Tiles 1.1 converter. Built in Rust for speed, reliability, and minimal resource usage.

Takes OBJ, glTF/GLB, or PLY meshes from photogrammetry software (Pix4D, Agisoft Metashape, RealityCapture, DJI Terra) and outputs optimized, georeferenced 3D Tiles with LOD hierarchy, Draco compression, and per-tile texture atlas repacking.

## Features

- **Multi-format input** -- OBJ (with MTL/textures), glTF/GLB, PLY (vertex colors)
- **Memory-mapped I/O** -- processes 10GB+ meshes without loading everything into RAM
- **Always-correct triangle clipping** -- Sutherland-Hodgman clipping at every tile boundary, no centroid fallback
- **Per-island texture atlas repacking** -- connected-component UV island detection, bin packing, bleed padding, and UV remapping per tile
- **Native mesh simplification** -- meshoptimizer at full native speed with SIMD, quadric error metrics with UV seam preservation
- **Full parallelism** -- rayon data-parallel processing across all pipeline stages
- **Auto-georeferencing** -- detects `offset.xyz`, `metadata.xml`, `.prj` files automatically
- **ECEF output** -- transforms from any projected CRS (UTM, State Plane, etc.) to WGS84/ECEF
- **3D Tiles 1.1** -- compliant output with validation, direct GLB content (no legacy b3dm)
- **Three deployment modes** -- CLI binary, Rust library crate, HTTP/gRPC service

## Requirements

- Rust >= 1.85 (install via [rustup](https://rustup.rs))
- PROJ library (for CRS transforms): `brew install proj` / `apt install libproj-dev`

## Installation

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
```

## Quick Start

### Pix4D OBJ with georeferencing

```bash
photo-tiler -i model.obj -o ./tileset --units m \
  --epsg 32636 --offset-file offset.xyz
```

### glTF file (meters assumed)

```bash
photo-tiler -i model.gltf -o ./output
```

### PLY with vertex colors

```bash
photo-tiler -i pointcloud.ply -o ./tileset --units m --no-textures
```

### Dry run (inspect without processing)

```bash
photo-tiler -i model.obj --dry-run --units m
```

## CLI Reference

| Flag | Description | Default |
|------|-------------|---------|
| `-i, --input <path>` | Input file (OBJ, glTF, GLB, PLY) | required |
| `-o, --output <dir>` | Output directory | required |
| `--units <unit>` | Input units: `mm`, `cm`, `m`, `ft`, `in` | auto-detect |
| `--epsg <code>` | EPSG code (e.g., 32636) | auto-detect |
| `--easting <m>` | Origin easting | auto-detect |
| `--northing <m>` | Origin northing | auto-detect |
| `--elevation <m>` | Origin elevation | 0 |
| `--true-north <deg>` | True north rotation | 0 |
| `--offset-file <path>` | Path to offset.xyz | auto-detect |
| `--metadata-xml <path>` | Path to metadata.xml | auto-detect |
| `--show-georef` | Display detected georeferencing and exit | |
| `--max-triangles <n>` | Max triangles per leaf tile | 100000 |
| `--max-depth <n>` | Max octree depth | 6 |
| `--no-draco` | Disable Draco mesh compression | |
| `--draco-level <n>` | Draco compression level (1-10) | 7 |
| `--no-textures` | Exclude textures from output | |
| `--texture-format <fmt>` | `webp`, `ktx2`, or `original` | webp |
| `--texture-quality <n>` | Compression quality (0-100) | 85 |
| `--texture-max-size <n>` | Max texture dimension in px | 2048 |
| `--validate` | Run tileset validation after conversion | |
| `--dry-run` | Scan input and report stats only | |
| `-v, --verbose` | Enable verbose logging | |
| `-j, --threads <n>` | Worker thread count | auto (all cores) |

## Supported Photogrammetry Software

| Software | Export Format | Georef Files | Status |
|----------|-------------|--------------|--------|
| **Pix4D** | OBJ + MTL + textures | `offset.xyz`, `.prj` | Fully supported |
| **Agisoft Metashape** | OBJ, glTF | `metadata.xml` | Fully supported |
| **RealityCapture** | OBJ, glTF | `.prj` | Supported |
| **DJI Terra** | OBJ | `metadata.xml` | Supported |
| **Any glTF exporter** | glTF/GLB | -- | Supported |

## Library API

```rust
use photo_tiler::{Pipeline, PipelineConfig, Units, Georeference};

let config = PipelineConfig {
    input: "model.obj".into(),
    output: "./tileset".into(),
    units: Units::Meters,
    georeference: Some(Georeference {
        epsg: 32636,
        easting: 500000.0,
        northing: 2800000.0,
        elevation: 0.0,
        ..Default::default()
    }),
    ..Default::default()
};

let result = Pipeline::run(&config)?;
println!("Generated {} tiles in {:?}", result.tile_count, result.duration);
```

## HTTP Service Mode

Build with the `server` feature:

```bash
cargo build --release --features server
photo-tiler serve --port 8080
```

Submit jobs via POST:

```bash
curl -X POST http://localhost:8080/convert \
  -F "input=@model.obj" \
  -F "config={\"units\":\"m\",\"epsg\":32636}"
```

## Docker

```dockerfile
FROM rust:1.85-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --features server

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /app/target/release/photo-tiler /
ENTRYPOINT ["/photo-tiler"]
```

Image size: ~15MB. Cold start: <10ms.

## Output Structure

```
output/
  tileset.json          # 3D Tiles 1.1 tileset descriptor
  tiles/
    root.glb            # Root LOD (most simplified)
    0/
      tile.glb          # Octant 0
      0_0/
        tile.glb        # Leaf tile (full resolution)
      ...
```

## Viewing Output

**CesiumJS:**
```javascript
const tileset = await Cesium.Cesium3DTileset.fromUrl('./output/tileset.json');
viewer.scene.primitives.add(tileset);
viewer.zoomTo(tileset);
```

**Included test viewer** (Three.js):
```bash
cd viewer && npm install && npm run dev
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md) -- system design, module reference, algorithms
- [Roadmap](docs/ROADMAP.md) -- implementation plan and milestones
- [Usage Manual](docs/USAGE_MANUAL.md) -- detailed workflows and troubleshooting
- [Reference Analysis](docs/REFERENCE_ANALYSIS.md) -- deep comparison of obj2tiles, mago3d-tiler, and cesium-native
- [UML Workflows](docs/WORKFLOW_UML.md) -- pipeline diagrams and data flow

## License

MIT
