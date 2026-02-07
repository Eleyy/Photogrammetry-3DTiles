use criterion::{criterion_group, criterion_main, Criterion};

fn bench_split(_c: &mut Criterion) {
    // TODO: Milestone 4 -- benchmark octree splitting
}

criterion_group!(benches, bench_split);
criterion_main!(benches);
