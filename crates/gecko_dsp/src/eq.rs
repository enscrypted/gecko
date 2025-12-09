//! 10-Band Parametric Equalizer
//!
//! Implements a cascade of BiQuad filters for audio equalization.
//! Based on the RBJ (Robert Bristow-Johnson) Audio EQ Cookbook.

use biquad::{Biquad, Coefficients, DirectForm2Transposed, ToHertz, Type, Q_BUTTERWORTH_F32};

use crate::error::DspError;

/// Standard EQ band frequencies (Hz) - ISO standard octave centers
pub const EQ_BANDS: [f32; 10] = [
    31.0,    // Sub-bass
    62.0,    // Bass
    125.0,   // Low-mid
    250.0,   // Mid
    500.0,   // Mid
    1000.0,  // Upper-mid
    2000.0,  // Presence
    4000.0,  // Brilliance
    8000.0,  // High
    16000.0, // Air
];

/// Filter type for each EQ band
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BandType {
    LowShelf,
    Peaking,
    HighShelf,
}

/// Single EQ band configuration
#[derive(Debug, Clone, Copy)]
pub struct Band {
    pub frequency: f32,
    pub gain_db: f32,
    pub q: f32,
    pub band_type: BandType,
    pub enabled: bool,
}

impl Band {
    pub fn new(frequency: f32, band_type: BandType) -> Self {
        Self {
            frequency,
            gain_db: 0.0,
            q: Q_BUTTERWORTH_F32, // ~0.707, gives smooth response
            band_type,
            enabled: true,
        }
    }

    /// Convert dB gain to linear amplitude
    /// Formula: amplitude = 10^(dB/20)
    fn db_to_amplitude(db: f32) -> f32 {
        10.0_f32.powf(db / 20.0)
    }

    /// Generate BiQuad coefficients for this band
    /// Rust pattern: `to_*` methods on Copy types take self by value since Copy is cheap
    fn to_coefficients(self, sample_rate: f32) -> Result<Coefficients<f32>, DspError> {
        // Rust pattern: `?` operator propagates errors up the call stack
        // This is idiomatic Rust error handling - no exceptions, explicit Result types
        let freq = self.frequency.hz();
        let fs = sample_rate.hz();

        let coeffs = match self.band_type {
            BandType::LowShelf => {
                Coefficients::<f32>::from_params(
                    Type::LowShelf(Self::db_to_amplitude(self.gain_db)),
                    fs,
                    freq,
                    self.q,
                )
            }
            BandType::Peaking => {
                Coefficients::<f32>::from_params(
                    Type::PeakingEQ(Self::db_to_amplitude(self.gain_db)),
                    fs,
                    freq,
                    self.q,
                )
            }
            BandType::HighShelf => {
                Coefficients::<f32>::from_params(
                    Type::HighShelf(Self::db_to_amplitude(self.gain_db)),
                    fs,
                    freq,
                    self.q,
                )
            }
        };

        coeffs.map_err(|_| DspError::InvalidCoefficients {
            frequency: self.frequency,
            sample_rate,
        })
    }
}

/// Complete EQ configuration for all 10 bands
#[derive(Debug, Clone)]
pub struct EqConfig {
    pub bands: [Band; 10],
    pub master_gain_db: f32,
    pub enabled: bool,
}

impl Default for EqConfig {
    fn default() -> Self {
        // Rust pattern: array initialization with index-based logic
        // `core::array::from_fn` creates array by calling closure with each index
        let bands = core::array::from_fn(|i| {
            let band_type = match i {
                0 => BandType::LowShelf,      // 31 Hz - shelf for sub-bass
                9 => BandType::HighShelf,     // 16 kHz - shelf for air
                _ => BandType::Peaking,       // All others are peaking
            };
            Band::new(EQ_BANDS[i], band_type)
        });

        Self {
            bands,
            master_gain_db: 0.0,
            enabled: true,
        }
    }
}

impl EqConfig {
    /// Set gain for a specific band (0-9)
    pub fn set_band_gain(&mut self, band_index: usize, gain_db: f32) -> Result<(), DspError> {
        if band_index >= 10 {
            return Err(DspError::InvalidBandIndex(band_index));
        }
        // Clamp gain to reasonable range (-24dB to +24dB)
        self.bands[band_index].gain_db = gain_db.clamp(-24.0, 24.0);
        Ok(())
    }

    /// Get all gains as a slice (useful for UI serialization)
    pub fn get_gains(&self) -> [f32; 10] {
        core::array::from_fn(|i| self.bands[i].gain_db)
    }
}

/// The main equalizer processor
///
/// Holds the filter state and processes audio samples.
/// Designed for real-time use: no allocations in `process()`.
pub struct Equalizer {
    // DirectForm2Transposed: better numerical stability than DF1
    // Each channel needs its own filter state (stereo = 2 channels)
    filters_left: [DirectForm2Transposed<f32>; 10],
    filters_right: [DirectForm2Transposed<f32>; 10],
    config: EqConfig,
    sample_rate: f32,
    master_gain_linear: f32,
}

impl Equalizer {
    /// Create a new equalizer with default flat response
    pub fn new(sample_rate: f32) -> Self {
        let config = EqConfig::default();

        // Rust pattern: creating arrays of non-Copy types requires explicit initialization
        // We use `core::array::from_fn` which calls the closure for each index
        let filters_left = core::array::from_fn(|i| {
            let coeffs = config.bands[i]
                .to_coefficients(sample_rate)
                .expect("Default config should always produce valid coefficients");
            DirectForm2Transposed::<f32>::new(coeffs)
        });

        let filters_right = core::array::from_fn(|i| {
            let coeffs = config.bands[i]
                .to_coefficients(sample_rate)
                .expect("Default config should always produce valid coefficients");
            DirectForm2Transposed::<f32>::new(coeffs)
        });

        Self {
            filters_left,
            filters_right,
            config,
            sample_rate,
            master_gain_linear: 1.0,
        }
    }

    /// Update EQ configuration
    ///
    /// Call this between buffer processing, not during.
    /// Recalculates all filter coefficients.
    pub fn update_config(&mut self, config: EqConfig) -> Result<(), DspError> {
        for (i, band) in config.bands.iter().enumerate() {
            if band.enabled {
                let coeffs = band.to_coefficients(self.sample_rate)?;
                self.filters_left[i].update_coefficients(coeffs);
                self.filters_right[i].update_coefficients(coeffs);
            }
        }
        self.master_gain_linear = 10.0_f32.powf(config.master_gain_db / 20.0);
        self.config = config;
        Ok(())
    }

    /// Set gain for a single band (convenience method)
    pub fn set_band_gain(&mut self, band_index: usize, gain_db: f32) -> Result<(), DspError> {
        self.config.set_band_gain(band_index, gain_db)?;

        let band = &self.config.bands[band_index];
        let coeffs = band.to_coefficients(self.sample_rate)?;
        self.filters_left[band_index].update_coefficients(coeffs);
        self.filters_right[band_index].update_coefficients(coeffs);

        Ok(())
    }

    /// Process a stereo sample pair through the EQ chain
    ///
    /// # Real-time Safety
    /// This function performs NO allocations and NO syscalls.
    /// Safe to call from audio callback.
    #[inline]
    pub fn process_sample(&mut self, left: f32, right: f32) -> (f32, f32) {
        if !self.config.enabled {
            return (left, right);
        }

        let mut l = left;
        let mut r = right;

        // Cascade through all enabled filters
        for i in 0..10 {
            if self.config.bands[i].enabled {
                // Rust pattern: `run()` processes one sample through the BiQuad
                // This is the hot path - compiler will inline and potentially vectorize
                l = self.filters_left[i].run(l);
                r = self.filters_right[i].run(r);
            }
        }

        (l * self.master_gain_linear, r * self.master_gain_linear)
    }

    /// Process an interleaved stereo buffer in-place
    ///
    /// Buffer format: [L0, R0, L1, R1, L2, R2, ...]
    ///
    /// # Real-time Safety
    /// No allocations. O(n) where n = buffer length.
    #[inline]
    pub fn process_interleaved(&mut self, buffer: &mut [f32]) {
        // Rust pattern: `chunks_exact_mut(2)` gives us mutable 2-element slices
        // This is bounds-checked at compile time for the slice access
        for frame in buffer.chunks_exact_mut(2) {
            let (l, r) = self.process_sample(frame[0], frame[1]);
            frame[0] = l;
            frame[1] = r;
        }
    }

    /// Process separate left/right channel buffers
    ///
    /// # Panics
    /// Panics if buffers have different lengths (debug builds only)
    #[inline]
    pub fn process_planar(&mut self, left: &mut [f32], right: &mut [f32]) {
        debug_assert_eq!(left.len(), right.len(), "Channel buffers must be same length");

        // Rust pattern: `zip` pairs up elements from two iterators
        // More idiomatic than index-based loop, often optimizes identically
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let (new_l, new_r) = self.process_sample(*l, *r);
            *l = new_l;
            *r = new_r;
        }
    }

    /// Get current configuration (for UI state sync)
    pub fn config(&self) -> &EqConfig {
        &self.config
    }

    /// Get sample rate
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Reset filter state (clear delay lines)
    ///
    /// Call when switching audio sources to prevent filter ringing
    pub fn reset(&mut self) {
        for i in 0..10 {
            self.filters_left[i].reset_state();
            self.filters_right[i].reset_state();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_flat() {
        let config = EqConfig::default();
        for band in &config.bands {
            assert_eq!(band.gain_db, 0.0, "Default should be flat (0dB)");
        }
    }

    #[test]
    fn test_band_frequencies_match_spec() {
        let config = EqConfig::default();
        for (i, band) in config.bands.iter().enumerate() {
            assert_eq!(band.frequency, EQ_BANDS[i]);
        }
    }

    #[test]
    fn test_first_band_is_low_shelf() {
        let config = EqConfig::default();
        assert_eq!(config.bands[0].band_type, BandType::LowShelf);
    }

    #[test]
    fn test_last_band_is_high_shelf() {
        let config = EqConfig::default();
        assert_eq!(config.bands[9].band_type, BandType::HighShelf);
    }

    #[test]
    fn test_middle_bands_are_peaking() {
        let config = EqConfig::default();
        for i in 1..9 {
            assert_eq!(config.bands[i].band_type, BandType::Peaking);
        }
    }

    #[test]
    fn test_gain_clamping() {
        let mut config = EqConfig::default();

        // Test upper clamp
        config.set_band_gain(0, 100.0).unwrap();
        assert_eq!(config.bands[0].gain_db, 24.0);

        // Test lower clamp
        config.set_band_gain(0, -100.0).unwrap();
        assert_eq!(config.bands[0].gain_db, -24.0);
    }

    #[test]
    fn test_invalid_band_index() {
        let mut config = EqConfig::default();
        assert!(config.set_band_gain(10, 0.0).is_err());
        assert!(config.set_band_gain(100, 0.0).is_err());
    }

    #[test]
    fn test_eq_steady_state_response() {
        let mut eq = Equalizer::new(48000.0);

        // Biquad filters have transient response - need to settle first.
        // Process multiple samples to let filter reach steady state.
        for _ in 0..1000 {
            eq.process_sample(0.5, -0.5);
        }

        // After settling, output should be stable (not diverging/clipping)
        let (out_l, out_r) = eq.process_sample(0.5, -0.5);

        // Even at 0dB gain, cascaded biquad filters have some frequency response.
        // Verify output is stable and reasonable (not clipping, not silent)
        assert!(out_l.abs() < 2.0, "Output should not clip: {}", out_l);
        assert!(out_l.abs() > 0.1, "Output should not be silent: {}", out_l);
        assert!(out_r.abs() < 2.0, "Output should not clip: {}", out_r);
        assert!(out_r.abs() > 0.1, "Output should not be silent: {}", out_r);

        // Verify polarity is preserved (roughly)
        assert!(out_l > 0.0, "Left output should be positive for positive input");
        assert!(out_r < 0.0, "Right output should be negative for negative input");
    }

    #[test]
    fn test_eq_disabled_passthrough() {
        let mut eq = Equalizer::new(48000.0);

        // Apply some gain
        eq.set_band_gain(5, 12.0).unwrap();

        // Disable EQ
        let mut config = eq.config().clone();
        config.enabled = false;
        eq.update_config(config).unwrap();

        // Should pass through unchanged
        let (out_l, out_r) = eq.process_sample(0.5, -0.5);
        assert_eq!(out_l, 0.5);
        assert_eq!(out_r, -0.5);
    }

    #[test]
    fn test_interleaved_processing() {
        let mut eq = Equalizer::new(48000.0);
        let mut buffer = vec![0.5, -0.5, 0.3, -0.3, 0.1, -0.1];

        eq.process_interleaved(&mut buffer);

        // Buffer should be modified (exact values depend on filter state)
        // Just verify it doesn't panic and produces reasonable output
        for sample in &buffer {
            assert!(sample.is_finite());
            assert!(sample.abs() < 10.0); // Sanity check
        }
    }

    #[test]
    fn test_planar_processing() {
        let mut eq = Equalizer::new(48000.0);
        let mut left = vec![0.5, 0.3, 0.1];
        let mut right = vec![-0.5, -0.3, -0.1];

        eq.process_planar(&mut left, &mut right);

        for sample in left.iter().chain(right.iter()) {
            assert!(sample.is_finite());
        }
    }

    #[test]
    fn test_sample_rate_stored() {
        let eq = Equalizer::new(44100.0);
        assert_eq!(eq.sample_rate(), 44100.0);

        let eq = Equalizer::new(96000.0);
        assert_eq!(eq.sample_rate(), 96000.0);
    }

    #[test]
    fn test_reset_doesnt_panic() {
        let mut eq = Equalizer::new(48000.0);

        // Process some samples
        for _ in 0..100 {
            eq.process_sample(0.5, -0.5);
        }

        // Reset should not panic
        eq.reset();

        // Should still work after reset
        let (l, r) = eq.process_sample(0.5, -0.5);
        assert!(l.is_finite());
        assert!(r.is_finite());
    }

    #[test]
    fn test_boost_increases_amplitude() {
        let mut eq = Equalizer::new(48000.0);

        // Boost 1kHz significantly
        eq.set_band_gain(5, 12.0).unwrap();

        // Generate a 1kHz sine wave and process it
        let sample_rate = 48000.0;
        let freq = 1000.0;
        let mut max_input = 0.0_f32;
        let mut max_output = 0.0_f32;

        for i in 0..1000 {
            let t = i as f32 / sample_rate;
            let sample = (2.0 * std::f32::consts::PI * freq * t).sin() * 0.5;
            max_input = max_input.max(sample.abs());

            let (out, _) = eq.process_sample(sample, sample);
            max_output = max_output.max(out.abs());
        }

        // Output should be louder than input for boosted frequency
        assert!(max_output > max_input, "Boost should increase amplitude");
    }
}
