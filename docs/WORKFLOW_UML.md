# Photo-Tiler UML Workflow Diagrams

All diagrams use Mermaid syntax and can be rendered in GitHub, VS Code, or any Mermaid-compatible viewer.

---

## 1. Pipeline Sequence Diagram

```mermaid
sequenceDiagram
    participant User
    participant CLI as main.rs
    participant Pipeline as pipeline.rs
    participant Ingest as ingestion/
    participant Transform as transform/
    participant Tiling as tiling/
    participant Output as tileset_writer

    User->>CLI: photo-tiler -i model.obj -o ./output --units m
    CLI->>CLI: parse args (clap)
    CLI->>Pipeline: Pipeline::run(config)

    Note over Pipeline: Stage 1: Ingestion
    Pipeline->>Ingest: ingest(config)
    Ingest->>Ingest: mmap + scan (pass 1)
    Ingest->>Ingest: parse geometry (pass 2, rayon parallel)
    Ingest->>Ingest: load MTL + textures
    Ingest-->>Pipeline: Vec<IndexedMesh> + MaterialLibrary

    Note over Pipeline: Stage 1.5: Georeferencing
    Pipeline->>Ingest: detect_georef(input_dir)
    Ingest-->>Pipeline: Georeference

    Note over Pipeline: Stage 2: Transform (f64 precision)
    Pipeline->>Transform: transform_meshes(meshes, georef)
    Transform->>Transform: scale to meters
    Transform->>Transform: Y-up to Z-up
    Transform->>Transform: center at origin
    Transform->>Transform: compute ECEF root transform
    Transform-->>Pipeline: transformed meshes + root_transform

    Note over Pipeline: Stage 3: Tiling (decimate first, then split)
    Pipeline->>Tiling: generate_tileset(meshes, materials, config)

    par LOD Generation (rayon parallel)
        Tiling->>Tiling: LOD 0 = original
        Tiling->>Tiling: LOD 1 = simplify 50% (meshoptimizer native)
        Tiling->>Tiling: LOD 2 = simplify 25%
    end

    par Per-LOD Spatial Split (rayon parallel)
        Tiling->>Tiling: octree subdivision + triangle clipping
        Tiling->>Tiling: vertex dedup at boundaries
    end

    par Per-Tile Processing (rayon parallel)
        Tiling->>Tiling: UV island detection + atlas bin packing
        Tiling->>Tiling: texture compression
        Tiling->>Tiling: GLB generation
        Tiling->>Output: write tile.glb
    end

    Tiling->>Output: write tileset.json
    Output-->>Pipeline: ProcessingResult

    Note over Pipeline: Stage 4: Validate (optional)
    Pipeline->>Output: validate_tileset(tileset.json)
    Pipeline-->>CLI: Result
    CLI-->>User: Summary
```

---

## 2. Module Dependency Graph

```mermaid
graph TD
    CLI[main.rs] --> Pipeline[pipeline.rs]
    CLI --> Config[config.rs]

    Pipeline --> Ingestion[ingestion/mod.rs]
    Pipeline --> Transform[transform/mod.rs]
    Pipeline --> Tiling[tiling/mod.rs]

    subgraph Ingestion Module
        Ingestion --> ObjParser[obj_parser.rs]
        Ingestion --> GltfLoader[gltf_loader.rs]
        Ingestion --> PlyLoader[ply_loader.rs]
        Ingestion --> MtlParser[mtl_parser.rs]
        Ingestion --> Georef[georef.rs]
    end

    subgraph Transform Module
        Transform --> Coordinates[coordinates.rs]
        Transform --> ECEF[ecef.rs]
        Transform --> Projection[projection.rs]
    end

    subgraph Tiling Module
        Tiling --> Octree[octree.rs]
        Tiling --> Clipper[triangle_clipper.rs]
        Tiling --> Simplifier[simplifier.rs]
        Tiling --> AtlasRepacker[atlas_repacker.rs]
        Tiling --> GlbWriter[glb_writer.rs]
        Tiling --> TilesetWriter[tileset_writer.rs]
        Tiling --> TexCompress[texture_compress.rs]
        Octree --> Clipper
    end

    subgraph Types
        Mesh[mesh.rs]
        Material[material.rs]
        Tile[tile.rs]
    end

    Ingestion --> Mesh
    Ingestion --> Material
    Transform --> Mesh
    Tiling --> Mesh
    Tiling --> Material
    Tiling --> Tile
```

---

## 3. Data Flow Diagram

```mermaid
flowchart LR
    subgraph Input
        OBJ[model.obj + .mtl + textures]
        GLTF[model.gltf/.glb]
        PLY[model.ply]
        GEO[offset.xyz / metadata.xml / .prj]
    end

    subgraph "Stage 1: Ingest"
        MMAP[mmap + scan<br/>counts + offsets]
        PARSE[Parse geometry<br/>rayon parallel]
        MTL[Parse MTL<br/>load textures]
    end

    subgraph "Stage 2: Transform (f64)"
        UNIT[Scale to meters]
        AXIS[Y-up to Z-up]
        CENTER[Center at origin]
        ECEF_T[ECEF root transform]
    end

    subgraph "Stage 3: Tile"
        DECIMATE["Decimate (meshopt native)<br/>LOD 0, 1, 2, ..."]
        SPLIT[Octree subdivision<br/>+ triangle clipping<br/>per LOD]
        REPACK[Per-island atlas<br/>bin packing + UV remap]
        GLB[GLB generation]
        JSON[tileset.json]
    end

    subgraph Output
        TILESET[tileset.json]
        TILES[tiles/*.glb]
    end

    OBJ --> MMAP --> PARSE
    OBJ --> MTL
    GLTF --> PARSE
    PLY --> PARSE
    GEO --> ECEF_T

    PARSE -->|"Vec<IndexedMesh>"| UNIT
    MTL -->|MaterialLibrary| REPACK

    UNIT --> AXIS --> CENTER
    CENTER -->|"f32 meshes"| DECIMATE
    DECIMATE -->|"LOD meshes"| SPLIT
    SPLIT -->|"per-tile meshes"| REPACK
    REPACK -->|"repacked textures"| GLB
    GLB --> TILES

    ECEF_T -->|"f64 4x4 matrix"| JSON
    SPLIT -->|"tile hierarchy"| JSON
    JSON --> TILESET
```

---

## 4. Decimate-First Pipeline (Key Architectural Decision)

```mermaid
flowchart TD
    ORIG[Original Mesh<br/>169M triangles<br/>full topology]

    subgraph "Step 1: LOD Generation (parallel)"
        LOD0[LOD 0<br/>169M tri<br/>original]
        LOD1[LOD 1<br/>~84M tri<br/>50% simplification]
        LOD2[LOD 2<br/>~42M tri<br/>25% simplification]
        LOD3[LOD 3<br/>~21M tri<br/>12.5% simplification]
    end

    subgraph "Step 2: Spatial Split (parallel per LOD)"
        S0[Split LOD 0<br/>into octree tiles]
        S1[Split LOD 1<br/>into octree tiles]
        S2[Split LOD 2<br/>into octree tiles]
        S3[Split LOD 3<br/>into octree tiles]
    end

    ORIG --> LOD0
    ORIG -->|"meshopt::simplify(50%)"| LOD1
    ORIG -->|"meshopt::simplify(25%)"| LOD2
    ORIG -->|"meshopt::simplify(12.5%)"| LOD3

    LOD0 -->|"Sutherland-Hodgman clip"| S0
    LOD1 -->|"Sutherland-Hodgman clip"| S1
    LOD2 -->|"Sutherland-Hodgman clip"| S2
    LOD3 -->|"Sutherland-Hodgman clip"| S3

    style ORIG fill:#E3F2FD
    style LOD0 fill:#4CAF50,color:#fff
    style LOD1 fill:#FF9800,color:#fff
    style LOD2 fill:#FF5722,color:#fff
    style LOD3 fill:#F44336,color:#fff
```

**Why decimate first?** The simplifier sees the full watertight mesh and can make globally optimal edge-collapse decisions. Splitting after simplification preserves boundary continuity.

---

## 5. Triangle Clipping at Tile Boundaries

```mermaid
flowchart TD
    TRI[Input Triangle<br/>spans 2 octants] --> CLASS{Classify vertices<br/>vs split plane}

    CLASS -->|All inside| KEEP[Assign to this octant]
    CLASS -->|All outside| SKIP[Assign to other octant]
    CLASS -->|Crossing| CLIP[Sutherland-Hodgman clip]

    CLIP --> INTERSECT[Compute intersection points<br/>on split plane]
    INTERSECT --> INTERP[Interpolate attributes at t:<br/>position, normal, UV, color]
    INTERP --> FAN[Fan-triangulate<br/>clipped polygon]
    FAN --> DEDUP[Deduplicate boundary<br/>vertices via hash]
    DEDUP --> ASSIGN[Assign sub-triangles<br/>to correct octants]

    style TRI fill:#FFCDD2
    style CLIP fill:#FFF9C4
    style ASSIGN fill:#C8E6C9
```

**No centroid fallback.** Every triangle crossing a boundary is clipped precisely, regardless of mesh size. This prevents gaps.

---

## 6. Per-Island Texture Atlas Repacking

```mermaid
flowchart TD
    subgraph "Input (per tile)"
        ATLAS[Source Atlas<br/>8192x8192 JPEG<br/>scattered UV islands]
        MESH[Tile mesh with UVs<br/>referencing multiple islands]
    end

    subgraph "Island Detection"
        EDGE[Build edge adjacency map]
        BFS[BFS connected components<br/>group faces into UV islands]
        BOUNDS[Compute UV bounding rect<br/>per island + padding]
    end

    subgraph "Bin Packing"
        PACK[Guillotine bin packing<br/>minimize atlas area]
        BLEED[Add 2-5px bleed ring<br/>per island]
    end

    subgraph "Output"
        COMPOSITE[Composite island pixels<br/>into new compact atlas]
        REMAP[Remap all UVs to<br/>new atlas coordinates]
        TILE_TEX[Per-tile atlas<br/>~200KB vs 20MB]
    end

    MESH --> EDGE --> BFS --> BOUNDS
    ATLAS --> COMPOSITE
    BOUNDS --> PACK --> BLEED --> COMPOSITE
    COMPOSITE --> TILE_TEX
    PACK --> REMAP

    style ATLAS fill:#FFCDD2
    style TILE_TEX fill:#C8E6C9
```

---

## 7. 3D Tiles Output Structure

```mermaid
graph TD
    ROOT["tileset.json<br/>geometricError: 150<br/>transform: ECEF 4x4"]

    ROOT --> T_ROOT["root.glb (LOD 3)<br/>12.5% resolution<br/>refine: REPLACE"]

    T_ROOT --> T0["tiles/0/tile.glb (LOD 2)<br/>25% resolution<br/>refine: REPLACE"]
    T_ROOT --> T1["tiles/1/tile.glb (LOD 2)<br/>25% resolution<br/>refine: REPLACE"]

    T0 --> T00["tiles/0_0/tile.glb (LOD 1)<br/>50% resolution<br/>refine: REPLACE"]
    T0 --> T01["tiles/0_1/tile.glb (LOD 1)<br/>50% resolution<br/>refine: REPLACE"]

    T00 --> T000["tiles/0_0_0/tile.glb (LOD 0)<br/>100% resolution<br/>geometricError: 0"]
    T00 --> T001["tiles/0_0_1/tile.glb (LOD 0)<br/>100% resolution<br/>geometricError: 0"]

    style T_ROOT fill:#F44336,color:#fff
    style T0 fill:#FF5722,color:#fff
    style T1 fill:#FF5722,color:#fff
    style T00 fill:#FF9800,color:#fff
    style T01 fill:#FF9800,color:#fff
    style T000 fill:#4CAF50,color:#fff
    style T001 fill:#4CAF50,color:#fff
```

**Viewer behavior:** Load coarsest LOD first, progressively replace with higher-res children as camera approaches, unload when camera moves away.

---

## 8. Parallelism Model

```mermaid
flowchart LR
    subgraph "rayon thread pool (all CPU cores)"
        direction TB
        S1["Stage 1: OBJ Parse<br/>par_chunks on mmap regions"]
        S2["Stage 2: Transform<br/>par_iter over vertices"]
        S3a["Stage 3a: Simplify<br/>par_iter over LOD levels"]
        S3b["Stage 3b: Split<br/>par_iter over octants"]
        S3c["Stage 3c: Repack<br/>par_iter over tiles"]
        S3d["Stage 3d: GLB Write<br/>par_iter over tiles"]
    end

    S1 --> S2 --> S3a --> S3b --> S3c --> S3d
```

All stages use rayon's work-stealing scheduler. No manual thread management. CPU utilization: ~90% on all cores.
