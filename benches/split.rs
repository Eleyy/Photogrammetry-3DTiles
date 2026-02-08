use criterion::{criterion_group, criterion_main, Criterion};
use photo_tiler::tiling::octree::{build_octree, split_mesh};
use photo_tiler::types::{BoundingBox, IndexedMesh};

/// Generate a 3D grid mesh spanning [0,1]^3 with triangles on XY faces at each Z layer.
fn make_3d_grid(n: usize) -> IndexedMesh {
    let verts_per_side = n + 1;
    let total_verts = verts_per_side * verts_per_side * verts_per_side;
    let mut positions = Vec::with_capacity(total_verts * 3);

    for z in 0..verts_per_side {
        for y in 0..verts_per_side {
            for x in 0..verts_per_side {
                let fx = x as f32 / n as f32;
                let fy = y as f32 / n as f32;
                let fz = z as f32 / n as f32;
                positions.extend_from_slice(&[fx, fy, fz]);
            }
        }
    }

    let mut indices = Vec::new();
    for z in 0..verts_per_side {
        for y in 0..n {
            for x in 0..n {
                let v = |x: usize, y: usize, z: usize| -> u32 {
                    (z * verts_per_side * verts_per_side + y * verts_per_side + x) as u32
                };
                let tl = v(x, y, z);
                let tr = v(x + 1, y, z);
                let bl = v(x, y + 1, z);
                let br = v(x + 1, y + 1, z);
                indices.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
            }
        }
    }

    IndexedMesh {
        positions,
        indices,
        ..Default::default()
    }
}

/// Generate a 3D grid with normals and UVs to exercise the full clipper path.
fn make_3d_grid_with_attrs(n: usize) -> IndexedMesh {
    let verts_per_side = n + 1;
    let total_verts = verts_per_side * verts_per_side * verts_per_side;
    let mut positions = Vec::with_capacity(total_verts * 3);
    let mut normals = Vec::with_capacity(total_verts * 3);
    let mut uvs = Vec::with_capacity(total_verts * 2);

    for z in 0..verts_per_side {
        for y in 0..verts_per_side {
            for x in 0..verts_per_side {
                let fx = x as f32 / n as f32;
                let fy = y as f32 / n as f32;
                let fz = z as f32 / n as f32;
                positions.extend_from_slice(&[fx, fy, fz]);
                normals.extend_from_slice(&[0.0, 0.0, 1.0]);
                uvs.extend_from_slice(&[fx, fy]);
            }
        }
    }

    let mut indices = Vec::new();
    for z in 0..verts_per_side {
        for y in 0..n {
            for x in 0..n {
                let v = |x: usize, y: usize, z: usize| -> u32 {
                    (z * verts_per_side * verts_per_side + y * verts_per_side + x) as u32
                };
                let tl = v(x, y, z);
                let tr = v(x + 1, y, z);
                let bl = v(x, y + 1, z);
                let br = v(x + 1, y + 1, z);
                indices.extend_from_slice(&[tl, bl, tr, tr, bl, br]);
            }
        }
    }

    IndexedMesh {
        positions,
        normals,
        uvs,
        indices,
        ..Default::default()
    }
}

fn bench_split(c: &mut Criterion) {
    // ~88K triangles: exercises both fast-path (interior) and slow-path (boundary) clipping
    let mesh = make_3d_grid(35);
    let bounds = BoundingBox {
        min: [0.0, 0.0, 0.0],
        max: [1.0, 1.0, 1.0],
    };

    c.bench_function("split_mesh_clipping_88k", |b| {
        b.iter(|| split_mesh(&mesh, &bounds));
    });
}

fn bench_split_with_attrs(c: &mut Criterion) {
    // Same grid but with normals + UVs — exercises attribute interpolation in clipper
    let mesh = make_3d_grid_with_attrs(20);
    let bounds = BoundingBox {
        min: [0.0, 0.0, 0.0],
        max: [1.0, 1.0, 1.0],
    };

    c.bench_function("split_mesh_clipping_with_attrs_17k", |b| {
        b.iter(|| split_mesh(&mesh, &bounds));
    });
}

fn bench_octree(c: &mut Criterion) {
    let mesh = make_3d_grid(35);
    let bounds = BoundingBox {
        min: [0.0, 0.0, 0.0],
        max: [1.0, 1.0, 1.0],
    };

    c.bench_function("build_octree_depth4_88k", |b| {
        b.iter(|| build_octree(&mesh, &bounds, 4, 10_000));
    });
}

criterion_group!(benches, bench_split, bench_split_with_attrs, bench_octree);
criterion_main!(benches);
