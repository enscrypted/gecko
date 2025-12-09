//! Gecko DSP - Digital Signal Processing Module
//!
//! This crate provides the audio processing pipeline for Gecko, including:
//! - 10-band parametric equalizer using BiQuad filters
//! - FFT spectrum analyzer for real-time visualization
//! - Soft clipping/limiter to prevent harsh digital distortion
//! - Lock-free coefficient updates for real-time safety
//! - Zero-allocation processing path
//!
//! # Architecture
//!
//! The DSP chain follows a strict "no allocation in audio callback" rule.
//! Filter coefficients are updated atomically between buffer processing calls.

mod eq;
mod error;
mod fft;
mod presets;
mod processor;
mod soft_clip;

pub use eq::{Band, BandType, Equalizer, EqConfig, EQ_BANDS};
pub use error::DspError;
pub use fft::{SpectrumAnalyzer, FFT_SIZE, NUM_BINS};
pub use presets::{Preset, PRESETS};
pub use processor::{AudioProcessor, ProcessContext};
pub use soft_clip::SoftClipper;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crate_exports() {
        // Verify all public types are accessible
        let _config = EqConfig::default();
        let _eq = Equalizer::new(48000.0);
    }
}
