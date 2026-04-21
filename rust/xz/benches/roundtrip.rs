//! Throughput benchmarks for `rust-xz`.
//!
//! Measures the time to compress and decompress a fixed payload at
//! every preset level the CLI exposes. Run with:
//!
//! ```text
//! cargo bench -p rust-xz
//! ```
//!
//! The benchmarks are intentionally small (256 KiB) so they run in
//! a few seconds; bump `PAYLOAD_BYTES` for a more realistic
//! throughput measurement against system `xz`.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};

use rust_xz::codec::{compress_stream, decompress_stream};
use rust_xz::options::Format;

const PAYLOAD_BYTES: usize = 256 * 1024;

fn make_payload() -> Vec<u8> {
    // Mildly compressible data: a counter pattern. Pure random
    // would basically test memory bandwidth; pure zeros would test
    // best-case ratio. The counter is somewhere in between and is
    // the same "synthetic but realistic" pattern xz's own bench
    // scripts use.
    (0..PAYLOAD_BYTES)
        .map(|i| ((i.wrapping_mul(1103515245).wrapping_add(12345)) >> 16) as u8)
        .collect()
}

fn bench_compress(c: &mut Criterion) {
    let payload = make_payload();
    let mut group = c.benchmark_group("compress");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    for level in [0u32, 3, 6, 9] {
        group.bench_function(format!("xz/preset-{level}"), |b| {
            b.iter(|| {
                let mut out = Vec::with_capacity(payload.len());
                compress_stream(&payload[..], &mut out, level, Format::Xz, None).unwrap();
                out
            });
        });
    }
    group.finish();
}

fn bench_decompress(c: &mut Criterion) {
    let payload = make_payload();
    let mut group = c.benchmark_group("decompress");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    for level in [0u32, 3, 6, 9] {
        let mut compressed = Vec::new();
        compress_stream(&payload[..], &mut compressed, level, Format::Xz, None).unwrap();
        group.bench_function(format!("xz/preset-{level}"), |b| {
            b.iter(|| {
                let mut out = Vec::with_capacity(payload.len());
                decompress_stream(&compressed[..], &mut out).unwrap();
                out
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_compress, bench_decompress);
criterion_main!(benches);
