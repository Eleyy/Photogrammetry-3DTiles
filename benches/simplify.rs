use criterion::{criterion_group, criterion_main, Criterion};

fn bench_simplify(_c: &mut Criterion) {
    // TODO: Milestone 4 -- benchmark mesh simplification
}

criterion_group!(benches, bench_simplify);
criterion_main!(benches);
