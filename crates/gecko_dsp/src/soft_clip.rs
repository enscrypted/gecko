//! Soft Clipping / Limiter
//!
//! Prevents hard digital clipping by applying a smooth saturation curve
//! when audio exceeds the threshold. This produces a more pleasant-sounding
//! distortion compared to hard clipping at 0dBFS.
//!
//! # Algorithm
//!
//! Uses a simple tanh-based soft clipper:
//! - Below threshold: linear (unity gain)
//! - Above threshold: smooth saturation via tanh()
//!
//! The knee region provides a gradual transition to avoid harsh artifacts.

use std::sync::atomic::{AtomicU32, Ordering};

/// Soft clipper that prevents hard clipping with smooth saturation
///
/// Thread-safe: threshold can be updated atomically while processing.
pub struct SoftClipper {
    /// Threshold where soft clipping begins (linear, 0.0 to 1.0)
    /// Stored as f32 bits for atomic access
    threshold_bits: AtomicU32,
    /// Whether soft clipping is enabled
    enabled: std::sync::atomic::AtomicBool,
}

impl SoftClipper {
    /// Create a new soft clipper
    ///
    /// # Arguments
    /// * `threshold_db` - Threshold in dB below 0dBFS where soft clipping begins.
    ///   Default is -3dB, meaning clipping starts at ~0.71
    pub fn new(threshold_db: f32) -> Self {
        let threshold_linear = db_to_linear(threshold_db);
        Self {
            threshold_bits: AtomicU32::new(threshold_linear.to_bits()),
            enabled: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// Set the threshold in dB below 0dBFS
    ///
    /// Common values:
    /// - -1dB: Very subtle, only catches peaks
    /// - -3dB: Moderate, good default
    /// - -6dB: Aggressive, noticeable warmth
    pub fn set_threshold_db(&self, db: f32) {
        let linear = db_to_linear(db);
        self.threshold_bits.store(linear.to_bits(), Ordering::Relaxed);
    }

    /// Get current threshold in linear scale
    pub fn threshold(&self) -> f32 {
        f32::from_bits(self.threshold_bits.load(Ordering::Relaxed))
    }

    /// Enable or disable soft clipping
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// Check if soft clipping is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Process a single sample through the soft clipper
    ///
    /// # Real-time Safety
    /// No allocations, no syscalls, O(1) time.
    #[inline]
    pub fn process_sample(&self, sample: f32) -> f32 {
        if !self.enabled.load(Ordering::Relaxed) {
            return sample;
        }

        let threshold = f32::from_bits(self.threshold_bits.load(Ordering::Relaxed));
        soft_clip(sample, threshold)
    }

    /// Process an interleaved stereo buffer in-place
    ///
    /// # Real-time Safety
    /// No allocations, O(n) time.
    #[inline]
    pub fn process_interleaved(&self, buffer: &mut [f32]) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }

        let threshold = f32::from_bits(self.threshold_bits.load(Ordering::Relaxed));
        for sample in buffer.iter_mut() {
            *sample = soft_clip(*sample, threshold);
        }
    }
}

impl Default for SoftClipper {
    fn default() -> Self {
        Self::new(-3.0) // -3dB threshold by default
    }
}

/// Convert decibels to linear amplitude
#[inline]
fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Soft clipping function using tanh-based saturation
///
/// This implements a smooth saturation curve that:
/// 1. Passes signal unchanged below threshold
/// 2. Applies tanh saturation above threshold
/// 3. Preserves signal polarity
/// 4. Guarantees output never exceeds ±1.0
///
/// The formula scales the signal so that:
/// - Input at threshold maps to output at threshold
/// - Signal asymptotically approaches ±1.0
#[inline]
fn soft_clip(sample: f32, threshold: f32) -> f32 {
    let abs_sample = sample.abs();

    if abs_sample <= threshold {
        // Below threshold: pass through unchanged
        sample
    } else {
        // Above threshold: apply tanh saturation
        // Scale so threshold maps correctly and output approaches 1.0
        let sign = sample.signum();
        let excess = abs_sample - threshold;
        let headroom = 1.0 - threshold;

        // Apply tanh to the excess, scaled by remaining headroom
        // tanh approaches 1.0 as input approaches infinity, so:
        // saturated_excess approaches headroom as excess approaches infinity
        // Therefore output = threshold + headroom = 1.0 at limit
        let normalized_excess = excess / headroom.max(0.001); // Avoid division by zero
        let saturated_excess = headroom * normalized_excess.tanh();

        sign * (threshold + saturated_excess)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_threshold() {
        let clipper = SoftClipper::default();
        // -3dB ≈ 0.708
        let threshold = clipper.threshold();
        assert!((threshold - 0.708).abs() < 0.01);
    }

    #[test]
    fn test_below_threshold_passthrough() {
        let clipper = SoftClipper::new(-3.0);
        let threshold = clipper.threshold();

        // Signal below threshold should pass through unchanged
        let input = threshold * 0.5;
        let output = clipper.process_sample(input);
        assert_eq!(input, output);

        // Negative too
        let output_neg = clipper.process_sample(-input);
        assert_eq!(-input, output_neg);
    }

    #[test]
    fn test_above_threshold_limited() {
        let clipper = SoftClipper::new(-6.0); // ~0.5 threshold

        // Signal well above threshold should be limited
        let input = 2.0;
        let output = clipper.process_sample(input);

        assert!(output < input, "Output should be less than input");
        assert!(output < 1.0, "Output should be less than 1.0");
        assert!(output > 0.5, "Output should be above threshold");
    }

    #[test]
    fn test_preserves_polarity() {
        let clipper = SoftClipper::new(-3.0);

        // Positive stays positive
        let out_pos = clipper.process_sample(1.5);
        assert!(out_pos > 0.0);

        // Negative stays negative
        let out_neg = clipper.process_sample(-1.5);
        assert!(out_neg < 0.0);

        // Symmetric response
        assert!((out_pos.abs() - out_neg.abs()).abs() < 0.001);
    }

    #[test]
    fn test_disabled_passthrough() {
        let clipper = SoftClipper::new(-3.0);
        clipper.set_enabled(false);

        // Should pass through even extreme values
        let input = 5.0;
        let output = clipper.process_sample(input);
        assert_eq!(input, output);
    }

    #[test]
    fn test_buffer_processing() {
        let clipper = SoftClipper::new(-6.0);

        let mut buffer = vec![0.3, -0.3, 0.8, -0.8, 1.5, -1.5];
        let original = buffer.clone();

        clipper.process_interleaved(&mut buffer);

        // Low values unchanged
        assert_eq!(buffer[0], original[0]);
        assert_eq!(buffer[1], original[1]);

        // High values limited
        assert!(buffer[4] < original[4]);
        assert!(buffer[5] > original[5]); // Negative, so "greater" means closer to 0
    }

    #[test]
    fn test_continuous_at_threshold() {
        let clipper = SoftClipper::new(-3.0);
        let threshold = clipper.threshold();

        // Output should be continuous at threshold
        let just_below = clipper.process_sample(threshold - 0.001);
        let just_above = clipper.process_sample(threshold + 0.001);

        // Should be very close (within ~0.01)
        assert!(
            (just_above - just_below).abs() < 0.01,
            "Should be continuous at threshold: {} vs {}",
            just_below,
            just_above
        );
    }

    #[test]
    fn test_never_exceeds_one() {
        let clipper = SoftClipper::new(-3.0);

        // Even extreme inputs should not exceed ±1.0
        for input in [10.0, 100.0, 1000.0, -10.0, -100.0, -1000.0] {
            let output = clipper.process_sample(input);
            assert!(
                output.abs() <= 1.0,
                "Output {} exceeds ±1.0 for input {}",
                output,
                input
            );
        }
    }

    #[test]
    fn test_threshold_update() {
        let clipper = SoftClipper::new(-3.0);

        // Update threshold
        clipper.set_threshold_db(-6.0);
        let new_threshold = clipper.threshold();

        // -6dB ≈ 0.5
        assert!((new_threshold - 0.5).abs() < 0.01);
    }
}
