use criterion::{criterion_group, criterion_main, Criterion};
use photo_tiler::tiling::lod::generate_lod_chain;
use photo_tiler::tiling::simplifier::simplify_mesh;
use photo_tiler::types::{BoundingBox, IndexedMesh};

/// Generate a flat grid mesh with `n x n` quads (2 triangles each).
fn make_grid(n: usize) -> IndexedMesh {
    let verts_per_side = n + 1;
    let vertex_count = verts_per_side * verts_per_side;
    let mut positions = Vec::with_capacity(vertex_count * 3);

    for y in 0..verts_per_side {
        for x in 0..verts_per_side {
            let fx = x as f32 / n as f32;
            let fy = y as f32 / n as f32;
            positions.extend_from_slice(&[fx, fy, 0.0]);
        }
    }

    let mut indices = Vec::with_capacity(n * n * 6);
    for y in 0..n {
        for x in 0..n {
            let tl = (y * verts_per_side + x) as u32;
            let tr = tl + 1;
            let bl = tl + verts_per_side as u32;
            let br = bl + 1;
            indices.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
        }
    }

    IndexedMesh {
        positions,
        indices,
        ..Default::default()
    }
}

fn bench_simplify(c: &mut Criterion) {
    // ~100K triangles: 224x224 grid = 50176 quads = 100352 triangles
    let mesh = make_grid(224);

    c.bench_function("simplify_mesh_50pct_100k", |b| {
        b.iter(|| simplify_mesh(&mesh, 0.5, false));
    });

    c.bench_function("simplify_mesh_25pct_100k", |b| {
        b.iter(|| simplify_mesh(&mesh, 0.25, true));
    });
}

fn bench_lod_chain(c: &mut Criterion) {
    let mesh = make_grid(224);
    let bounds = BoundingBox {
        min: [0.0, 0.0, 0.0],
        max: [1.0, 1.0, 0.0],
    };

    c.bench_function("lod_chain_4_levels_100k", |b| {
        b.iter(|| generate_lod_chain(mesh.clone(), &bounds, 4));
    });
}

criterion_group!(benches, bench_simplify, bench_lod_chain);
criterion_main!(benches);
