//! PipeWire Audio Stream - Native Audio Capture and Playback
//!
//! This module implements audio streaming using PipeWire's native stream API.
//! It captures audio from a virtual sink's monitor and outputs to real speakers.
//!
//! # Architecture
//!
//! ```text
//! App Audio (Firefox, etc.)
//!       ↓ (user routes to virtual sink)
//! Virtual Sink (created by PipeWireBackend)
//!       ↓ (monitor port)
//! Capture Stream ──→ DSP Processing ──→ Soft Clip ──→ Playback Stream
//!                          │                               ↓
//!                          └──→ FFT Analyzer         Real Speakers
//!                                    ↓
//!                              Spectrum Data → UI
//! ```
//!
//! IMPORTANT: This does NOT use microphone input! Audio comes from applications
//! routed through the virtual sink.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

// Note: These imports will be used when we implement actual streaming
#[allow(unused_imports)]
use pipewire as pw;

use gecko_dsp::{SoftClipper, SpectrumAnalyzer, NUM_BINS};

/// Audio format configuration
#[derive(Debug, Clone, Copy)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u32,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
        }
    }
}

/// Shared state for audio processing between streams
pub struct AudioProcessingState {
    /// Whether processing is bypassed
    pub bypassed: AtomicBool,

    /// Master volume (stored as u32 bits, interpreted as f32)
    master_volume_bits: AtomicU32,

    /// Peak level left channel
    peak_left_bits: AtomicU32,

    /// Peak level right channel
    peak_right_bits: AtomicU32,

    /// Whether streams are running
    pub running: AtomicBool,

    /// Master EQ band gains (10 bands, stored as u32 bits interpreted as f32)
    /// These are the base/global EQ settings
    master_eq_gains: [AtomicU32; 10],

    /// Per-stream EQ offsets (stream_id → [10 bands of offset_db])
    /// These offsets are ADDED to master EQ to get final gains
    stream_eq_offsets: parking_lot::RwLock<std::collections::HashMap<String, [f32; 10]>>,

    /// Combined EQ gains (master + sum of all active stream offsets)
    /// This is what the audio callback actually uses
    combined_eq_gains: [AtomicU32; 10],

    /// Counter that increments whenever EQ changes (master or stream)
    /// The audio callback checks this to know when to update its local EQ state
    eq_update_counter: AtomicU32,

    /// Set of apps that have active capture streams in per-app mode
    /// This is updated by the PipeWire thread when apps are captured/released
    captured_apps: parking_lot::RwLock<std::collections::HashSet<String>>,

    /// Counter that increments whenever captured_apps changes
    /// The engine polls this to know when to emit stream discovery/removal events
    captured_apps_version: AtomicU32,

    /// Per-stream volume (stream_id → volume 0.0-2.0)
    stream_volumes: parking_lot::RwLock<std::collections::HashMap<String, f32>>,

    /// Per-stream bypass state (stream_id → bypassed)
    stream_bypassed: parking_lot::RwLock<std::collections::HashMap<String, bool>>,

    /// Spectrum analyzer for FFT visualization
    /// Accumulates samples and computes FFT at ~30fps for UI display
    spectrum_analyzer: SpectrumAnalyzer,

    /// Soft clipper to prevent harsh digital distortion
    /// Applied after all processing, before final output
    soft_clipper: SoftClipper,

    /// Whether soft clipping is enabled
    soft_clip_enabled: AtomicBool,
}

impl AudioProcessingState {
    pub fn new() -> Self {
        // Rust pattern: Initialize array of atomics with default 0dB gain
        let master_eq_gains = core::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits()));
        let combined_eq_gains = core::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits()));

        Self {
            bypassed: AtomicBool::new(false),
            master_volume_bits: AtomicU32::new(1.0_f32.to_bits()),
            peak_left_bits: AtomicU32::new(0.0_f32.to_bits()),
            peak_right_bits: AtomicU32::new(0.0_f32.to_bits()),
            running: AtomicBool::new(false),
            master_eq_gains,
            stream_eq_offsets: parking_lot::RwLock::new(std::collections::HashMap::new()),
            combined_eq_gains,
            eq_update_counter: AtomicU32::new(0),
            captured_apps: parking_lot::RwLock::new(std::collections::HashSet::new()),
            captured_apps_version: AtomicU32::new(0),
            stream_volumes: parking_lot::RwLock::new(std::collections::HashMap::new()),
            stream_bypassed: parking_lot::RwLock::new(std::collections::HashMap::new()),
            // FFT spectrum analyzer: 48kHz sample rate, ~60fps updates for smoother visuals
            spectrum_analyzer: SpectrumAnalyzer::new(48000.0, 60),
            // Soft clipper: -3dB threshold (starts limiting at ~0.71)
            soft_clipper: SoftClipper::new(-3.0),
            soft_clip_enabled: AtomicBool::new(true),
        }
    }

    pub fn set_master_volume(&self, volume: f32) {
        self.master_volume_bits.store(volume.to_bits(), Ordering::Relaxed);
    }

    pub fn master_volume(&self) -> f32 {
        f32::from_bits(self.master_volume_bits.load(Ordering::Relaxed))
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

    /// Set master EQ band gain and recalculate combined
    pub fn set_eq_band_gain(&self, band: usize, gain_db: f32) {
        if band < 10 {
            self.master_eq_gains[band].store(gain_db.to_bits(), Ordering::Relaxed);
            self.recalculate_combined_eq();
        }
    }

    /// Set per-stream EQ offset and recalculate combined
    pub fn set_stream_eq_offset(&self, stream_id: &str, band: usize, offset_db: f32) {
        if band < 10 {
            let mut offsets = self.stream_eq_offsets.write();
            let entry = offsets.entry(stream_id.to_string()).or_insert([0.0; 10]);
            entry[band] = offset_db;
            drop(offsets); // Release lock before recalculating
            self.recalculate_combined_eq();
        }
    }

    /// Set all bands for a stream at once
    pub fn set_stream_eq_all(&self, stream_id: &str, gains: [f32; 10]) {
        let mut offsets = self.stream_eq_offsets.write();
        offsets.insert(stream_id.to_string(), gains);
        drop(offsets);
        self.recalculate_combined_eq();
    }

    /// Remove a stream's EQ offsets (when stream ends)
    pub fn remove_stream_eq(&self, stream_id: &str) {
        let mut offsets = self.stream_eq_offsets.write();
        if offsets.remove(stream_id).is_some() {
            drop(offsets);
            self.recalculate_combined_eq();
        }
    }

    /// Get master EQ band gain
    pub fn get_eq_band_gain(&self, band: usize) -> f32 {
        if band < 10 {
            f32::from_bits(self.master_eq_gains[band].load(Ordering::Relaxed))
        } else {
            0.0
        }
    }

    /// Get stream EQ offset for a specific band
    pub fn get_stream_eq_offset(&self, stream_id: &str, band: usize) -> f32 {
        if band < 10 {
            let offsets = self.stream_eq_offsets.read();
            offsets.get(stream_id).map(|o| o[band]).unwrap_or(0.0)
        } else {
            0.0
        }
    }

    /// Get all stream EQ offsets for a stream
    pub fn get_stream_eq_all(&self, stream_id: &str) -> [f32; 10] {
        let offsets = self.stream_eq_offsets.read();
        offsets.get(stream_id).copied().unwrap_or([0.0; 10])
    }

    /// Get all master EQ band gains as an array
    pub fn get_all_eq_gains(&self) -> [f32; 10] {
        core::array::from_fn(|i| f32::from_bits(self.combined_eq_gains[i].load(Ordering::Relaxed)))
    }

    /// Recalculate combined EQ from master + all stream offsets
    fn recalculate_combined_eq(&self) {
        let offsets = self.stream_eq_offsets.read();
        
        for band in 0..10 {
            let master = f32::from_bits(self.master_eq_gains[band].load(Ordering::Relaxed));
            
            // Sum all stream offsets for this band
            let total_offset: f32 = offsets.values().map(|o| o[band]).sum();
            
            // Combined = master + sum of all offsets (clamped to valid range)
            let combined = (master + total_offset).clamp(-24.0, 24.0);
            self.combined_eq_gains[band].store(combined.to_bits(), Ordering::Relaxed);
        }
        
        // Increment counter to signal change to the audio callback
        self.eq_update_counter.fetch_add(1, Ordering::Release);
    }

    /// Get the current EQ update counter value
    /// Audio callback compares this to its local copy to detect changes
    pub fn eq_update_counter(&self) -> u32 {
        self.eq_update_counter.load(Ordering::Acquire)
    }

    // === Per-App Capture Tracking ===

    /// Add an app to the captured set
    /// Called by PipeWire thread when a capture stream is created for an app
    pub fn add_captured_app(&self, app_name: &str) {
        let mut apps = self.captured_apps.write();
        if apps.insert(app_name.to_string()) {
            drop(apps); // Release lock before incrementing counter
            self.captured_apps_version.fetch_add(1, Ordering::Release);
        }
    }

    /// Remove an app from the captured set
    /// Called by PipeWire thread when a capture stream is destroyed
    pub fn remove_captured_app(&self, app_name: &str) {
        let mut apps = self.captured_apps.write();
        if apps.remove(app_name) {
            drop(apps);
            self.captured_apps_version.fetch_add(1, Ordering::Release);
        }
    }

    /// Get the list of currently captured apps
    pub fn get_captured_apps(&self) -> Vec<String> {
        self.captured_apps.read().iter().cloned().collect()
    }

    /// Get the version counter for captured apps list
    /// Engine compares this to detect changes and emit events
    pub fn captured_apps_version(&self) -> u32 {
        self.captured_apps_version.load(Ordering::Acquire)
    }

    // === Per-Stream Volume ===

    /// Set volume for a specific stream
    pub fn set_stream_volume(&self, stream_id: &str, volume: f32) {
        let mut volumes = self.stream_volumes.write();
        volumes.insert(stream_id.to_string(), volume);
    }

    /// Get volume for a specific stream (defaults to 1.0)
    pub fn get_stream_volume(&self, stream_id: &str) -> f32 {
        let volumes = self.stream_volumes.read();
        volumes.get(stream_id).copied().unwrap_or(1.0)
    }

    // === Per-Stream Bypass ===

    /// Set bypass for a specific stream
    pub fn set_stream_bypass(&self, stream_id: &str, bypassed: bool) {
        let mut bypass_map = self.stream_bypassed.write();
        if bypassed {
            bypass_map.insert(stream_id.to_string(), true);
        } else {
            bypass_map.remove(stream_id);
        }
    }

    /// Check if a stream is bypassed (defaults to false)
    pub fn is_stream_bypassed(&self, stream_id: &str) -> bool {
        let bypass_map = self.stream_bypassed.read();
        bypass_map.get(stream_id).copied().unwrap_or(false)
    }

    // === Spectrum Analyzer ===

    /// Push a stereo sample pair to the spectrum analyzer
    ///
    /// Call this from the audio callback for each processed sample.
    /// This is lock-free and safe to call from real-time context.
    #[inline]
    pub fn push_spectrum_sample(&self, left: f32, right: f32) {
        self.spectrum_analyzer.push_sample(left, right);
    }

    /// Update the spectrum analyzer and check if new data is ready
    ///
    /// Call this from the UI thread at your desired frame rate.
    /// Returns true if spectrum was computed.
    pub fn update_spectrum(&self) -> bool {
        static UPDATE_CALL_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        static UPDATE_TRUE_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

        let result = self.spectrum_analyzer.update();

        let call_count = UPDATE_CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if result {
            let true_count = UPDATE_TRUE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // Log EVERY successful update for debugging
            tracing::debug!("FFT update #{} succeeded (total calls: {})", true_count, call_count);
        } else if call_count % 50 == 0 {
            // Log every 50th call that returned false (at debug level for visibility)
            tracing::debug!("FFT update returned false at call #{} (success count: {})",
                call_count, UPDATE_TRUE_COUNT.load(std::sync::atomic::Ordering::Relaxed));
        }

        result
    }

    /// Get the current spectrum data
    ///
    /// Returns NUM_BINS values (0.0-1.0) representing magnitude
    /// in logarithmically-spaced frequency bands.
    pub fn get_spectrum(&self) -> [f32; NUM_BINS] {
        self.spectrum_analyzer.get_spectrum()
    }

    // === Soft Clipping ===

    /// Process a sample through the soft clipper
    ///
    /// Call this from the audio callback after all other processing.
    #[inline]
    pub fn soft_clip_sample(&self, sample: f32) -> f32 {
        if self.soft_clip_enabled.load(Ordering::Relaxed) {
            self.soft_clipper.process_sample(sample)
        } else {
            sample
        }
    }

    /// Process an interleaved buffer through the soft clipper
    #[inline]
    pub fn soft_clip_buffer(&self, buffer: &mut [f32]) {
        if self.soft_clip_enabled.load(Ordering::Relaxed) {
            self.soft_clipper.process_interleaved(buffer);
        }
    }

    /// Enable or disable soft clipping
    pub fn set_soft_clip_enabled(&self, enabled: bool) {
        self.soft_clip_enabled.store(enabled, Ordering::Relaxed);
    }

    /// Check if soft clipping is enabled
    pub fn is_soft_clip_enabled(&self) -> bool {
        self.soft_clip_enabled.load(Ordering::Relaxed)
    }

    /// Set soft clipping threshold in dB below 0dBFS
    pub fn set_soft_clip_threshold(&self, threshold_db: f32) {
        self.soft_clipper.set_threshold_db(threshold_db);
    }
}

impl Default for AudioProcessingState {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for creating audio streams
#[derive(Debug, Clone)]
pub struct StreamConfig {
    /// Name for the capture stream
    pub capture_name: String,

    /// Name for the playback stream
    pub playback_name: String,

    /// Target node ID for capture (virtual sink's monitor)
    pub capture_target: Option<u32>,

    /// Target node ID for playback (real speakers)
    pub playback_target: Option<u32>,

    /// Audio format
    pub format: AudioFormat,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            capture_name: "Gecko Capture".to_string(),
            playback_name: "Gecko Playback".to_string(),
            capture_target: None,
            playback_target: None,
            format: AudioFormat::default(),
        }
    }
}

/// User data for PipeWire stream callbacks
/// Note: This will be used when we implement actual PipeWire streaming
#[allow(dead_code)]
struct StreamData {
    /// Ring buffer for transferring audio between capture and playback
    buffer: Vec<f32>,
    /// Write position in the ring buffer
    write_pos: usize,
    /// Read position in the ring buffer
    read_pos: usize,
    /// Buffer capacity in samples
    capacity: usize,
    /// Processing state (volume, bypass, etc.)
    state: Arc<AudioProcessingState>,
}

#[allow(dead_code)]
impl StreamData {
    fn new(capacity: usize, state: Arc<AudioProcessingState>) -> Self {
        Self {
            buffer: vec![0.0; capacity],
            write_pos: 0,
            read_pos: 0,
            capacity,
            state,
        }
    }

    /// Write samples to the ring buffer
    fn write(&mut self, samples: &[f32]) -> usize {
        let mut written = 0;
        for &sample in samples {
            let next_pos = (self.write_pos + 1) % self.capacity;
            if next_pos != self.read_pos {
                self.buffer[self.write_pos] = sample;
                self.write_pos = next_pos;
                written += 1;
            } else {
                // Buffer full
                break;
            }
        }
        written
    }

    /// Read samples from the ring buffer
    fn read(&mut self, output: &mut [f32]) -> usize {
        let mut read = 0;
        for sample in output.iter_mut() {
            if self.read_pos != self.write_pos {
                *sample = self.buffer[self.read_pos];
                self.read_pos = (self.read_pos + 1) % self.capacity;
                read += 1;
            } else {
                // Buffer empty, output silence
                *sample = 0.0;
            }
        }
        read
    }

    /// Get available samples to read
    fn available(&self) -> usize {
        if self.write_pos >= self.read_pos {
            self.write_pos - self.read_pos
        } else {
            self.capacity - self.read_pos + self.write_pos
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_default() {
        let format = AudioFormat::default();
        assert_eq!(format.sample_rate, 48000);
        assert_eq!(format.channels, 2);
    }

    #[test]
    fn test_processing_state() {
        let state = AudioProcessingState::new();

        assert!(!state.bypassed.load(Ordering::Relaxed));
        assert_eq!(state.master_volume(), 1.0);
        assert_eq!(state.peaks(), (0.0, 0.0));

        state.set_master_volume(0.5);
        assert_eq!(state.master_volume(), 0.5);

        state.set_peaks(0.8, 0.6);
        assert_eq!(state.peaks(), (0.8, 0.6));
    }

    #[test]
    fn test_stream_config_default() {
        let config = StreamConfig::default();
        assert_eq!(config.capture_name, "Gecko Capture");
        assert_eq!(config.playback_name, "Gecko Playback");
        assert!(config.capture_target.is_none());
    }

    #[test]
    fn test_stream_data_ring_buffer() {
        let state = Arc::new(AudioProcessingState::new());
        let mut data = StreamData::new(16, state);

        // Write some samples
        let input = [1.0, 2.0, 3.0, 4.0];
        let written = data.write(&input);
        assert_eq!(written, 4);
        assert_eq!(data.available(), 4);

        // Read them back
        let mut output = [0.0; 4];
        let read = data.read(&mut output);
        assert_eq!(read, 4);
        assert_eq!(output, input);
        assert_eq!(data.available(), 0);
    }

    #[test]
    fn test_eq_band_gains_default_to_zero() {
        let state = AudioProcessingState::new();

        // All bands should start at 0dB
        for band in 0..10 {
            assert_eq!(
                state.get_eq_band_gain(band),
                0.0,
                "Band {} should default to 0dB",
                band
            );
        }

        // Counter should start at 0
        assert_eq!(state.eq_update_counter(), 0);
    }

    #[test]
    fn test_eq_band_gain_update_increments_counter() {
        let state = AudioProcessingState::new();

        // Initial counter value
        let initial_counter = state.eq_update_counter();
        assert_eq!(initial_counter, 0);

        // Set band 5 to +6dB
        state.set_eq_band_gain(5, 6.0);

        // Counter should have incremented
        assert_eq!(state.eq_update_counter(), 1);
        assert_eq!(state.get_eq_band_gain(5), 6.0);

        // Set another band
        state.set_eq_band_gain(2, -3.0);
        assert_eq!(state.eq_update_counter(), 2);
        assert_eq!(state.get_eq_band_gain(2), -3.0);

        // Previous band should still have its value
        assert_eq!(state.get_eq_band_gain(5), 6.0);
    }

    #[test]
    fn test_eq_get_all_gains() {
        let state = AudioProcessingState::new();

        // Set some bands
        state.set_eq_band_gain(0, -12.0);
        state.set_eq_band_gain(5, 6.0);
        state.set_eq_band_gain(9, 3.0);

        let gains = state.get_all_eq_gains();

        assert_eq!(gains[0], -12.0);
        assert_eq!(gains[5], 6.0);
        assert_eq!(gains[9], 3.0);

        // Untouched bands should be 0
        assert_eq!(gains[1], 0.0);
        assert_eq!(gains[4], 0.0);
    }

    #[test]
    fn test_eq_invalid_band_index_ignored() {
        let state = AudioProcessingState::new();

        // Setting invalid band should not panic and not change counter
        let counter_before = state.eq_update_counter();
        state.set_eq_band_gain(10, 5.0); // Invalid - only 0-9 exist
        state.set_eq_band_gain(100, 5.0); // Also invalid

        // Counter should NOT have changed for invalid bands
        assert_eq!(state.eq_update_counter(), counter_before);

        // Getting invalid band should return 0
        assert_eq!(state.get_eq_band_gain(10), 0.0);
        assert_eq!(state.get_eq_band_gain(100), 0.0);
    }

    #[test]
    fn test_eq_counter_detects_changes_across_threads() {
        use std::thread;

        let state = Arc::new(AudioProcessingState::new());
        let state_clone = Arc::clone(&state);

        // Simulate audio callback checking counter
        let initial_counter = state.eq_update_counter();

        // Simulate UI thread updating EQ
        let handle = thread::spawn(move || {
            state_clone.set_eq_band_gain(3, 12.0);
        });

        handle.join().unwrap();

        // Audio callback should detect the change
        let new_counter = state.eq_update_counter();
        assert!(
            new_counter > initial_counter,
            "Counter should have increased: {} -> {}",
            initial_counter,
            new_counter
        );

        // And be able to read the new gain
        assert_eq!(state.get_eq_band_gain(3), 12.0);
    }
}
