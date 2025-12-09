//! Audio engine benchmarks
//!
//! Measures performance of core audio processing paths.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use gecko_core::AudioProcessor;

fn benchmark_processor(c: &mut Criterion) {
    let mut group = c.benchmark_group("audio_processor");

    // Typical buffer sizes used in real-time audio
    for buffer_size in [64, 128, 256, 512, 1024].iter() {
        let sample_rate = 48000.0;
        let mut processor = AudioProcessor::new(sample_rate, *buffer_size);

        // Create test buffer (stereo interleaved)
        let mut buffer: Vec<f32> = (0..*buffer_size * 2)
            .map(|i| (i as f32 * 0.001).sin())
            .collect();

        group.throughput(Throughput::Elements(*buffer_size as u64 * 2));
        group.bench_function(format!("process_{}_samples", buffer_size), |b| {
            b.iter(|| {
                processor.process(black_box(&mut buffer));
            })
        });
    }

    group.finish();
}

fn benchmark_bypass_toggle(c: &mut Criterion) {
    let mut processor = AudioProcessor::new(48000.0, 256);

    c.bench_function("bypass_toggle", |b| {
        b.iter(|| {
            processor.set_bypass(black_box(true));
            processor.set_bypass(black_box(false));
        })
    });
}

criterion_group!(benches, benchmark_processor, benchmark_bypass_toggle);
criterion_main!(benches);
