# DSP Pipeline and Technology Stack

## 1. Digital Signal Processing (DSP) Specification

The core feature is a high-fidelity **10-Band Equalizer**.

### 1.1 Library

- **Crate**: `biquad` v0.4+
- **Precision**: `f32` (Single Precision Float). Double precision is unnecessary for audio playback EQ and incurs SIMD penalties.

### 1.2 Filter Definition

The EQ consists of 10 serially cascaded BiQuad filters per channel.

| Band | Freq (Hz) | Type | Q Factor |
|------|-----------|------|----------|
| 1 | 31 | Low Shelf | 0.707 |
| 2 | 62 | Peaking | 1.41 |
| 3 | 125 | Peaking | 1.41 |
| 4 | 250 | Peaking | 1.41 |
| 5 | 500 | Peaking | 1.41 |
| 6 | 1k | Peaking | 1.41 |
| 7 | 2k | Peaking | 1.41 |
| 8 | 4k | Peaking | 1.41 |
| 9 | 8k | Peaking | 1.41 |
| 10 | 16k | High Shelf | 0.707 |

### 1.3 Coefficient Management

- Coefficients are recalculated **only when the user changes a slider**
- **Atomic Update**: Coefficients are replaced at the start of a processing block, not mid-buffer

## 2. Processing Pipeline

```rust
// Per-sample processing (hot path)
let mut sample = input_sample;
for filter in &mut filters {
    sample = filter.run(sample);
}
output_sample = sample * master_gain;
```

### 2.1 Real-Time Safety Rules

The `process()` function MUST follow these rules:
- **NO heap allocations** (no `Vec::push`, no `Box::new`, no `String`)
- **NO syscalls** (no file I/O, no network, no mutex locks)
- **NO unbounded loops**
- **Constant or O(n) time complexity** where n = buffer size

Violating these rules causes audio dropouts ("glitches").

## 3. The AudioProcessor Trait

```rust
pub trait AudioProcessor: Send {
    fn process(&mut self, buffer: &mut [f32], context: &ProcessContext);
    fn reset(&mut self);
    fn name(&self) -> &'static str;
    fn is_enabled(&self) -> bool;
}
```

This trait allows different effects (EQ, Compressor, Limiter) to be chained dynamically.

## 4. Threading Model

```
┌────────────────┐     ┌────────────────┐     ┌────────────────┐
│  UI Thread     │     │  Audio Thread  │     │ Analysis Thread│
│  (Tauri)       │────▶│  (Real-time)   │────▶│  (Low priority)│
│                │     │                │     │                │
│ Commands:      │     │ - Capture      │     │ - FFT          │
│ - Set EQ gain  │     │ - DSP Process  │     │ - Peak detect  │
│ - Start/Stop   │     │ - Output       │     │ - UI events    │
└────────────────┘     └────────────────┘     └────────────────┘
        │                      │                      │
        │    crossbeam-channel │         rtrb         │
        └──────────────────────┴──────────────────────┘
```

## 5. Lock-Free Data Structures

### Ring Buffer (rtrb)
- SPSC (Single Producer Single Consumer)
- Used for audio data between threads
- `no_std` compatible

### Channels (crossbeam-channel)
- MPMC (Multi Producer Multi Consumer)
- Used for control messages
- `try_recv()` in audio callback (never blocks)

## 6. Implementation Roadmap

### Phase 1: The Skeleton
- [x] Initialize Tauri project
- [x] Set up CPAL for default device I/O
- [x] Implement Ring Buffer (rtrb)

### Phase 2: The DSP Core
- [x] Implement the biquad filter chain
- [x] Connect Tauri frontend sliders to Rust backend
- [ ] Verify CPU usage remains < 1% on a single core

### Phase 3: Platform Specifics
- [ ] Linux: Implement PipeWire linking
- [ ] Windows: Implement windows-rs Process Loopback
- [ ] macOS: Compile BlackHole fork and implement Shared Memory reader

### Phase 4: Visualization & Polish
- [ ] Implement FFT using spectrum-analyzer crate
- [ ] Send frequency bin data to UI via Tauri Events (30fps cap)
- [ ] Packaging and Installers (MSI, Deb/RPM, DMG/Pkg)

## 7. Performance Targets

| Metric | Target | Measurement |
|--------|--------|-------------|
| DSP CPU | < 1% | Single core at 48kHz stereo |
| Latency | < 10ms | Round-trip capture to output |
| Memory | < 50MB | Resident set size |
| Binary | < 20MB | Compressed installer |

## 8. Testing Strategy

```bash
# Run all tests
cargo test --workspace

# Run benchmarks
cargo bench -p gecko_dsp

# Run with hardware (ignored by default)
cargo test -- --ignored
```
