# 10-Band Equalizer Implementation

**Last Updated**: December 2024
**Context**: Read when working on the equalizer, DSP filters, or frequency response

## Overview

Gecko implements a 10-band parametric equalizer using cascaded BiQuad filters. The implementation prioritizes real-time safety (zero allocations) and audio quality.

## Filter Specification

| Band | Frequency | Type | Q Factor | Notes |
|------|-----------|------|----------|-------|
| 1 | 31 Hz | Low Shelf | 0.707 | Sub-bass |
| 2 | 62 Hz | Peaking | 1.41 | Bass |
| 3 | 125 Hz | Peaking | 1.41 | Low-mid |
| 4 | 250 Hz | Peaking | 1.41 | Mid |
| 5 | 500 Hz | Peaking | 1.41 | Mid |
| 6 | 1000 Hz | Peaking | 1.41 | Upper-mid |
| 7 | 2000 Hz | Peaking | 1.41 | Presence |
| 8 | 4000 Hz | Peaking | 1.41 | Brilliance |
| 9 | 8000 Hz | Peaking | 1.41 | High |
| 10 | 16000 Hz | High Shelf | 0.707 | Air |

### Why These Frequencies?
- Based on ISO standard octave centers
- Covers full audible spectrum (20Hz - 20kHz)
- Shelf filters at extremes prevent harsh cutoffs

### Why These Q Values?
- 0.707 (Butterworth) for shelves: Smooth transition, no peaking
- 1.41 for peaking: Moderate bandwidth, musical sound

## Library: biquad crate

```rust
use biquad::{Biquad, Coefficients, DirectForm2Transposed, ToHertz, Type, Q_BUTTERWORTH_F32};
```

### Key Features
- `no_std` compatible - no allocations
- Pre-calculated RBJ cookbook coefficients
- `DirectForm2Transposed` - better numerical stability

## Implementation Structure

### Band Configuration

```rust
pub struct Band {
    pub frequency: f32,
    pub gain_db: f32,
    pub q: f32,
    pub band_type: BandType,
    pub enabled: bool,
}

pub enum BandType {
    LowShelf,
    Peaking,
    HighShelf,
}
```

### Equalizer Struct

```rust
pub struct Equalizer {
    // Separate filter state per channel (stereo)
    filters_left: [DirectForm2Transposed<f32>; 10],
    filters_right: [DirectForm2Transposed<f32>; 10],
    config: EqConfig,
    sample_rate: f32,
    master_gain_linear: f32,
}
```

## Processing Pipeline

### Per-Sample Processing

```rust
#[inline]  // Hot path - inline for performance
pub fn process_sample(&mut self, left: f32, right: f32) -> (f32, f32) {
    if !self.config.enabled {
        return (left, right);  // Bypass
    }

    let mut l = left;
    let mut r = right;

    // Cascade through all 10 filters
    for i in 0..10 {
        if self.config.bands[i].enabled {
            l = self.filters_left[i].run(l);
            r = self.filters_right[i].run(r);
        }
    }

    // Apply master gain
    (l * self.master_gain_linear, r * self.master_gain_linear)
}
```

### Buffer Processing

```rust
// Interleaved: [L0, R0, L1, R1, ...]
pub fn process_interleaved(&mut self, buffer: &mut [f32]) {
    for frame in buffer.chunks_exact_mut(2) {
        let (l, r) = self.process_sample(frame[0], frame[1]);
        frame[0] = l;
        frame[1] = r;
    }
}

// Planar: separate L/R arrays
pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
    for (l, r) in left.iter_mut().zip(right.iter_mut()) {
        let (new_l, new_r) = self.process_sample(*l, *r);
        *l = new_l;
        *r = new_r;
    }
}
```

## Coefficient Updates

Coefficients are recalculated ONLY when user changes a parameter:

```rust
pub fn set_band_gain(&mut self, band_index: usize, gain_db: f32) -> Result<(), DspError> {
    // Validate and clamp
    self.config.set_band_gain(band_index, gain_db)?;

    // Recalculate coefficients
    let band = &self.config.bands[band_index];
    let coeffs = band.to_coefficients(self.sample_rate)?;

    // Update both channels atomically (between buffers)
    self.filters_left[band_index].update_coefficients(coeffs);
    self.filters_right[band_index].update_coefficients(coeffs);

    Ok(())
}
```

### dB to Amplitude Conversion

```rust
fn db_to_amplitude(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

// Examples:
// 0 dB → 1.0
// +6 dB → 2.0 (double)
// -6 dB → 0.5 (half)
// +12 dB → 4.0
```

## Gain Clamping

```rust
// Limit to ±24 dB to prevent extreme responses
self.bands[band_index].gain_db = gain_db.clamp(-24.0, 24.0);
```

## Filter Reset

Called when switching audio sources to prevent filter ringing:

```rust
pub fn reset(&mut self) {
    for i in 0..10 {
        self.filters_left[i].reset_state();
        self.filters_right[i].reset_state();
    }
}
```

## Performance Characteristics

- **CPU Usage**: < 0.5% at 48kHz stereo (10 BiQuad operations per sample × 2 channels)
- **Latency**: Zero (IIR filters have no lookahead)
- **Memory**: ~2KB for filter states

## Test Coverage

Located in `crates/gecko_dsp/src/eq.rs`:

- `test_default_config_is_flat` - Default gains are 0 dB
- `test_band_frequencies_match_spec` - Frequencies match constants
- `test_first_band_is_low_shelf` - Correct filter types
- `test_gain_clamping` - ±24 dB limits enforced
- `test_eq_steady_state_response` - Filter stability
- `test_eq_disabled_passthrough` - Bypass works
- `test_boost_increases_amplitude` - Gain actually boosts

## Related Files

- `crates/gecko_dsp/src/eq.rs` - Main implementation (485 lines)
- `crates/gecko_dsp/src/processor.rs` - AudioProcessor trait impl
- `crates/gecko_dsp/src/error.rs` - DspError definitions
