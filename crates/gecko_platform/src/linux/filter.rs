//! PipeWire Filter Node - Audio Processing Pipeline
//!
//! This module implements a PipeWire filter node that processes audio.
//! Apps route their audio to this filter, it processes through DSP,
//! then outputs to the real speakers.
//!
//! # Architecture
//!
//! ```text
//! App Audio (Firefox, etc.)
//!       ↓ (user routes to Gecko filter)
//! Gecko Filter Node
//!   ├── Input Port (receives audio)
//!   ├── DSP Processing (EQ, volume)
//!   └── Output Port (sends to speakers)
//!       ↓
//! Real Speakers
//! ```
//!
//! This approach is simpler than virtual sink + capture because:
//! 1. No separate capture/playback streams to manage
//! 2. Filter nodes are native PipeWire citizens
//! 3. Audio routing is handled by PipeWire session manager

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Shared state for the audio filter
///
/// This is Send+Sync so it can be shared between the main thread
/// and the PipeWire filter callbacks.
pub struct FilterState {
    /// Whether processing is bypassed (pass-through mode)
    pub bypassed: AtomicBool,

    /// Master volume (0.0 - 2.0, stored as f32 bits in u32)
    volume_bits: AtomicU32,

    /// Left channel peak level for metering
    peak_left_bits: AtomicU32,

    /// Right channel peak level for metering
    peak_right_bits: AtomicU32,

    /// Whether the filter is currently active
    pub active: AtomicBool,
}

impl FilterState {
    pub fn new() -> Self {
        Self {
            bypassed: AtomicBool::new(false),
            volume_bits: AtomicU32::new(1.0_f32.to_bits()),
            peak_left_bits: AtomicU32::new(0.0_f32.to_bits()),
            peak_right_bits: AtomicU32::new(0.0_f32.to_bits()),
            active: AtomicBool::new(false),
        }
    }

    pub fn set_volume(&self, volume: f32) {
        let clamped = volume.clamp(0.0, 2.0);
        self.volume_bits.store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn set_peaks(&self, left: f32, right: f32) {
        self.peak_left_bits.store(left.to_bits(), Ordering::Relaxed);
        self.peak_right_bits.store(right.to_bits(), Ordering::Relaxed);
    }

    pub fn peaks(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_left_bits.load(Ordering::Relaxed)),
            f32::from_bits(self.peak_right_bits.load(Ordering::Relaxed)),
        )
    }

    /// Process audio samples in-place
    ///
    /// This is called from the real-time audio callback.
    /// MUST NOT allocate or block!
    #[inline]
    pub fn process_audio(&self, samples: &mut [f32]) {
        // Skip if bypassed
        if self.bypassed.load(Ordering::Relaxed) {
            return;
        }

        let volume = self.volume();

        // Apply volume and calculate peaks
        let mut peak_l = 0.0_f32;
        let mut peak_r = 0.0_f32;

        // Rust pattern: process in chunks of 2 for stereo
        for chunk in samples.chunks_mut(2) {
            if chunk.len() == 2 {
                // Apply volume
                chunk[0] *= volume;
                chunk[1] *= volume;

                // Track peaks
                peak_l = peak_l.max(chunk[0].abs());
                peak_r = peak_r.max(chunk[1].abs());
            }
        }

        // Update peak meters
        self.set_peaks(peak_l, peak_r);
    }
}

impl Default for FilterState {
    fn default() -> Self {
        Self::new()
    }
}

// Rust pattern: FilterState is Send+Sync because all fields are atomic
unsafe impl Send for FilterState {}
unsafe impl Sync for FilterState {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_state_defaults() {
        let state = FilterState::new();
        assert!(!state.bypassed.load(Ordering::Relaxed));
        assert_eq!(state.volume(), 1.0);
        assert_eq!(state.peaks(), (0.0, 0.0));
        assert!(!state.active.load(Ordering::Relaxed));
    }

    #[test]
    fn test_filter_state_volume() {
        let state = FilterState::new();

        state.set_volume(0.5);
        assert_eq!(state.volume(), 0.5);

        // Test clamping
        state.set_volume(3.0);
        assert_eq!(state.volume(), 2.0);

        state.set_volume(-1.0);
        assert_eq!(state.volume(), 0.0);
    }

    #[test]
    fn test_process_audio_volume() {
        let state = FilterState::new();
        state.set_volume(0.5);

        let mut samples = [1.0, 1.0, 0.5, 0.5];
        state.process_audio(&mut samples);

        assert_eq!(samples[0], 0.5);
        assert_eq!(samples[1], 0.5);
        assert_eq!(samples[2], 0.25);
        assert_eq!(samples[3], 0.25);
    }

    #[test]
    fn test_process_audio_bypass() {
        let state = FilterState::new();
        state.set_volume(0.5);
        state.bypassed.store(true, Ordering::Relaxed);

        let mut samples = [1.0, 1.0];
        state.process_audio(&mut samples);

        // Should be unchanged when bypassed
        assert_eq!(samples[0], 1.0);
        assert_eq!(samples[1], 1.0);
    }

    #[test]
    fn test_process_audio_peaks() {
        let state = FilterState::new();

        let mut samples = [0.8, -0.6, 0.4, 0.3];
        state.process_audio(&mut samples);

        let (l, r) = state.peaks();
        assert_eq!(l, 0.8);
        assert_eq!(r, 0.6);
    }
}
