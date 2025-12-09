//! Performance benchmarks for the DSP module
//!
//! Run with: cargo bench -p gecko_dsp

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use gecko_dsp::Equalizer;

fn benchmark_eq_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("equalizer");

    // Common buffer sizes in audio applications
    let buffer_sizes = [64, 128, 256, 512, 1024, 2048];

    for size in buffer_sizes {
        // Stereo buffer (interleaved)
        let sample_count = size * 2;

        group.throughput(Throughput::Elements(size as u64));

        group.bench_function(format!("process_interleaved_{}_frames", size), |b| {
            let mut eq = Equalizer::new(48000.0);
            let mut buffer: Vec<f32> = (0..sample_count)
                .map(|i| (i as f32 * 0.001).sin())
                .collect();

            b.iter(|| {
                eq.process_interleaved(black_box(&mut buffer));
            });
        });

        group.bench_function(format!("process_planar_{}_frames", size), |b| {
            let mut eq = Equalizer::new(48000.0);
            let mut left: Vec<f32> = (0..size).map(|i| (i as f32 * 0.001).sin()).collect();
            let mut right: Vec<f32> = (0..size).map(|i| (i as f32 * 0.002).sin()).collect();

            b.iter(|| {
                eq.process_planar(black_box(&mut left), black_box(&mut right));
            });
        });
    }

    group.finish();
}

fn benchmark_eq_coefficient_update(c: &mut Criterion) {
    c.bench_function("eq_set_band_gain", |b| {
        let mut eq = Equalizer::new(48000.0);
        let mut band = 0;
        let mut gain = 0.0_f32;

        b.iter(|| {
            // Simulate changing a slider
            eq.set_band_gain(band, gain).unwrap();
            band = (band + 1) % 10;
            gain = (gain + 1.0) % 24.0;
        });
    });
}

fn benchmark_eq_sample_single(c: &mut Criterion) {
    c.bench_function("eq_process_single_sample", |b| {
        let mut eq = Equalizer::new(48000.0);

        b.iter(|| {
            black_box(eq.process_sample(black_box(0.5), black_box(-0.5)));
        });
    });
}

criterion_group!(
    benches,
    benchmark_eq_processing,
    benchmark_eq_coefficient_update,
    benchmark_eq_sample_single
);

criterion_main!(benches);
