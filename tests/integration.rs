//! End-to-end integration tests.
//!
//! These tests create synthetic input files, run the full pipeline,
//! and validate the output tileset.

use std::fs;
use std::path::Path;

use photo_tiler::config::{PipelineConfig, TextureConfig, TextureFormat, TilingConfig};
use photo_tiler::Pipeline;

/// Write a minimal OBJ + MTL + PNG texture to `dir`.
///
/// Creates a textured 10x10 grid (200 triangles) so the tiler has
/// something meaningful to work with.
fn write_synthetic_obj(dir: &Path) {
    let n = 10usize;
    let verts = n + 1;

    // Build OBJ content
    let mut obj = String::from("mtllib material.mtl\nusemtl textured\n");

    // Vertices + UVs
    for y in 0..verts {
        for x in 0..verts {
            let fx = x as f32 / n as f32;
            let fy = y as f32 / n as f32;
            // Y-up coordinates
            obj.push_str(&format!("v {} {} {}\n", fx, fy * 0.5, 0.0));
            obj.push_str(&format!("vt {} {}\n", fx, fy));
            obj.push_str(&format!("vn 0 0 1\n"));
        }
    }

    // Faces (1-indexed, with vertex/uv/normal)
    for y in 0..n {
        for x in 0..n {
            let tl = y * verts + x + 1; // 1-indexed
            let tr = tl + 1;
            let bl = tl + verts;
            let br = bl + 1;
            obj.push_str(&format!(
                "f {tl}/{tl}/{tl} {bl}/{bl}/{bl} {tr}/{tr}/{tr}\n"
            ));
            obj.push_str(&format!(
                "f {tr}/{tr}/{tr} {bl}/{bl}/{bl} {br}/{br}/{br}\n"
            ));
        }
    }

    fs::write(dir.join("model.obj"), &obj).unwrap();

    // MTL file
    let mtl = "\
newmtl textured
Kd 0.8 0.8 0.8
map_Kd texture.png
";
    fs::write(dir.join("material.mtl"), mtl).unwrap();

    // 16x16 checkerboard PNG texture
    let img = image::RgbaImage::from_fn(16, 16, |x, y| {
        if (x / 4 + y / 4) % 2 == 0 {
            image::Rgba([200, 60, 60, 255])
        } else {
            image::Rgba([60, 60, 200, 255])
        }
    });
    img.save(dir.join("texture.png")).unwrap();
}

/// Write a minimal OBJ without textures or materials.
fn write_plain_obj(dir: &Path) {
    let mut obj = String::new();
    let n = 4usize;
    let verts = n + 1;

    for y in 0..verts {
        for x in 0..verts {
            let fx = x as f32 / n as f32;
            let fy = y as f32 / n as f32;
            obj.push_str(&format!("v {} {} 0\n", fx, fy));
        }
    }
    for y in 0..n {
        for x in 0..n {
            let tl = y * verts + x + 1;
            let tr = tl + 1;
            let bl = tl + verts;
            let br = bl + 1;
            obj.push_str(&format!("f {tl} {bl} {tr}\n"));
            obj.push_str(&format!("f {tr} {bl} {br}\n"));
        }
    }

    fs::write(dir.join("model.obj"), &obj).unwrap();
}

#[test]
fn full_pipeline_textured_obj() {
    let tmp = tempfile::tempdir().unwrap();
    let input_dir = tmp.path().join("input");
    let output_dir = tmp.path().join("output");
    fs::create_dir_all(&input_dir).unwrap();

    write_synthetic_obj(&input_dir);

    let config = PipelineConfig {
        input: input_dir.join("model.obj"),
        output: output_dir.clone(),
        texture: TextureConfig {
            format: TextureFormat::Original, // PNG -- gltf crate can decode PNG but not WebP
            quality: 100,
            max_size: 512,
            enabled: true,
        },
        tiling: TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        },
        validate: true,
        ..Default::default()
    };

    let result = Pipeline::run(&config).expect("pipeline should succeed");
    assert!(result.tile_count >= 1, "should produce at least 1 tile");

    // tileset.json should exist
    let tileset_path = output_dir.join("tileset.json");
    assert!(tileset_path.exists(), "tileset.json should exist");

    // Parse and verify tileset.json
    let json_str = fs::read_to_string(&tileset_path).unwrap();
    let tileset: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(tileset["asset"]["version"], "1.1");
    assert!(tileset["root"].is_object());
    assert!(tileset["root"]["boundingVolume"]["box"].is_array());

    // tiles/ directory should exist with GLB files
    let tiles_dir = output_dir.join("tiles");
    assert!(tiles_dir.exists(), "tiles directory should exist");

    // Count GLB files
    fn count_glbs(dir: &Path) -> usize {
        let mut n = 0;
        for entry in fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.is_dir() {
                n += count_glbs(&p);
            } else if p.extension().is_some_and(|e| e == "glb") {
                n += 1;
            }
        }
        n
    }

    let glb_count = count_glbs(&tiles_dir);
    assert_eq!(
        glb_count, result.tile_count,
        "GLB file count should match tile_count"
    );

    // Verify at least one GLB contains a texture (the root tile)
    let root_glb_path = tiles_dir.join("root.glb");
    assert!(root_glb_path.exists(), "root.glb should exist");
    let root_glb_data = fs::read(&root_glb_path).unwrap();

    // Use from_slice_without_validation because compressed GLBs use
    // EXT_meshopt_compression which the gltf crate doesn't support in validation.
    let gltf_data = gltf::Gltf::from_slice_without_validation(&root_glb_data).unwrap();
    let buffers =
        gltf::import_buffers(&gltf_data.document, None, gltf_data.blob.clone()).unwrap();
    let images = gltf::import_images(&gltf_data.document, None, &buffers).unwrap();
    let doc = gltf_data.document;

    // Root should have a mesh
    assert!(doc.meshes().next().is_some(), "root GLB should have a mesh");

    // Root should have a texture (atlas repacking is enabled)
    assert!(
        !images.is_empty(),
        "root GLB should have an embedded texture from atlas repacking"
    );
    assert!(images[0].width > 0);
    assert!(images[0].height > 0);

    // Material should reference the texture
    if let Some(mat) = doc.materials().next() {
        let pbr = mat.pbr_metallic_roughness();
        assert!(
            pbr.base_color_texture().is_some(),
            "material should reference base color texture"
        );
    }
}

#[test]
fn full_pipeline_plain_obj_no_textures() {
    let tmp = tempfile::tempdir().unwrap();
    let input_dir = tmp.path().join("input");
    let output_dir = tmp.path().join("output");
    fs::create_dir_all(&input_dir).unwrap();

    write_plain_obj(&input_dir);

    let config = PipelineConfig {
        input: input_dir.join("model.obj"),
        output: output_dir.clone(),
        texture: TextureConfig {
            enabled: false,
            ..Default::default()
        },
        tiling: TilingConfig {
            max_triangles_per_tile: 100_000,
            max_depth: 4,
        },
        validate: true,
        ..Default::default()
    };

    let result = Pipeline::run(&config).expect("pipeline should succeed");
    assert!(result.tile_count >= 1);

    let tileset_path = output_dir.join("tileset.json");
    assert!(tileset_path.exists());

    let json_str = fs::read_to_string(&tileset_path).unwrap();
    let tileset: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(tileset["asset"]["version"], "1.1");
}

#[test]
fn full_pipeline_with_validation_passes() {
    let tmp = tempfile::tempdir().unwrap();
    let input_dir = tmp.path().join("input");
    let output_dir = tmp.path().join("output");
    fs::create_dir_all(&input_dir).unwrap();

    write_synthetic_obj(&input_dir);

    let config = PipelineConfig {
        input: input_dir.join("model.obj"),
        output: output_dir.clone(),
        texture: TextureConfig {
            format: TextureFormat::Original, // PNG for lossless roundtrip
            quality: 100,
            max_size: 256,
            enabled: true,
        },
        tiling: TilingConfig {
            max_triangles_per_tile: 50,
            max_depth: 3,
        },
        validate: true,
        ..Default::default()
    };

    // Pipeline with validation should succeed without panicking
    let result = Pipeline::run(&config).expect("pipeline with validation should succeed");
    assert!(result.tile_count >= 1);
}

#[test]
fn pipeline_missing_input_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let config = PipelineConfig {
        input: tmp.path().join("nonexistent.obj"),
        output: tmp.path().join("output"),
        ..Default::default()
    };

    let err = Pipeline::run(&config);
    assert!(err.is_err(), "missing input should return error");
}
