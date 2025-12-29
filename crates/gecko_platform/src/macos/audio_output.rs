//! macOS Audio Output Stream
//!
//! Handles audio output to speakers with DSP processing.
//! Works with Process Tap captures to provide per-app EQ.
//!
//! # Architecture
//!
//! ```text
//! Process Tap Captures (per app)
//!       ↓
//! Mix all app audio
//!       ↓
//! DSP Processing (EQ, Volume)
//!       ↓
//! Soft Clip
//!       ↓
//! Output to Speakers (via cpal)
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

// Debug counters for audio callback (static so they persist across callbacks)
static CALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);
static TOTAL_SAMPLES_MIXED: AtomicUsize = AtomicUsize::new(0);

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use parking_lot::{Mutex, RwLock};
use tracing::{debug, error};

use gecko_dsp::{Equalizer, SoftClipper, SpectrumAnalyzer, NUM_BINS};

use super::process_tap::AudioRingBuffer;
use crate::error::PlatformError;

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

/// Audio source in the mixer (one per captured app)
struct AudioSource {
    /// PID of the source application
    pid: u32,
    /// Application name (for looking up per-app settings)
    app_name: String,
    /// Ring buffer containing audio data from Process Tap
    ring_buffer: Arc<AudioRingBuffer>,
}

/// Thread-safe audio mixer that combines multiple audio sources
///
/// This struct is designed to be shared between the main thread and the audio
/// callback. Sources can be added/removed dynamically.
///
/// # Per-App EQ Architecture
///
/// Each app gets its own Equalizer instance, applied BEFORE mixing:
/// ```text
/// App1 Audio → App1 EQ ──┐
///                        ├──► Mix ──► Master EQ ──► Output
/// App2 Audio → App2 EQ ──┘
/// ```
///
/// This allows TRUE per-app EQ (different settings per app) vs the wrong
/// approach of applying "additive offsets" to a single mixed signal.
pub struct AudioMixer {
    /// List of active audio sources (protected by RwLock for dynamic updates)
    sources: RwLock<Vec<AudioSource>>,

    /// Per-app Equalizer instances (app_name → Equalizer)
    ///
    /// Equalizers are created lazily when an app is first processed.
    /// Uses Mutex because Equalizer::process() requires &mut self.
    /// In audio callback, use try_lock() to avoid blocking.
    app_equalizers: Mutex<HashMap<String, Equalizer>>,

    /// Sample rate for creating new Equalizers
    sample_rate: f32,
}

impl AudioMixer {
    /// Create a new empty audio mixer with default 48kHz sample rate
    pub fn new() -> Self {
        Self::with_sample_rate(48000.0)
    }

    /// Create a new empty audio mixer with specified sample rate
    pub fn with_sample_rate(sample_rate: f32) -> Self {
        Self {
            sources: RwLock::new(Vec::new()),
            app_equalizers: Mutex::new(HashMap::new()),
            sample_rate,
        }
    }

    /// Set EQ band gain for a specific app
    ///
    /// Called from the UI thread to update per-app EQ settings.
    pub fn set_app_eq_band(&self, app_name: &str, band: usize, gain_db: f32) {
        if band >= 10 {
            return;
        }

        let mut eqs = self.app_equalizers.lock();

        // Create Equalizer if it doesn't exist
        if !eqs.contains_key(app_name) {
            eqs.insert(app_name.to_string(), Equalizer::new(self.sample_rate));
        }

        if let Some(eq) = eqs.get_mut(app_name) {
            let _ = eq.set_band_gain(band, gain_db);
        }
    }

    /// Set all EQ bands for a specific app at once
    pub fn set_app_eq_bands(&self, app_name: &str, gains: &[f32; 10]) {
        let mut eqs = self.app_equalizers.lock();

        // Create Equalizer if it doesn't exist
        if !eqs.contains_key(app_name) {
            eqs.insert(app_name.to_string(), Equalizer::new(self.sample_rate));
        }

        if let Some(eq) = eqs.get_mut(app_name) {
            for (band, &gain) in gains.iter().enumerate() {
                let _ = eq.set_band_gain(band, gain);
            }
        }
    }

    /// Get EQ band gain for a specific app (returns 0.0 if app not found)
    pub fn get_app_eq_band(&self, app_name: &str, band: usize) -> f32 {
        if band >= 10 {
            return 0.0;
        }

        let eqs = self.app_equalizers.lock();
        eqs.get(app_name)
            .map(|eq| eq.config().get_gains()[band])
            .unwrap_or(0.0)
    }

    /// Get all EQ bands for a specific app (returns [0.0; 10] if app not found)
    pub fn get_app_eq_bands(&self, app_name: &str) -> [f32; 10] {
        let eqs = self.app_equalizers.lock();
        eqs.get(app_name)
            .map(|eq| eq.config().get_gains())
            .unwrap_or([0.0; 10])
    }

    /// Add a new audio source (Process Tap capture)
    ///
    /// # Arguments
    /// * `pid` - Process ID of the audio source
    /// * `app_name` - Application name (for looking up per-app settings)
    /// * `ring_buffer` - Ring buffer containing audio data from Process Tap
    pub fn add_source(&self, pid: u32, app_name: &str, ring_buffer: Arc<AudioRingBuffer>) {
        let mut sources = self.sources.write();
        // Check if source already exists
        if !sources.iter().any(|s| s.pid == pid) {
            sources.push(AudioSource {
                pid,
                app_name: app_name.to_string(),
                ring_buffer,
            });
            debug!("AudioMixer: Added source for {} (PID {})", app_name, pid);
        }
    }

    /// Remove an audio source by PID
    pub fn remove_source(&self, pid: u32) {
        let mut sources = self.sources.write();
        if let Some(pos) = sources.iter().position(|s| s.pid == pid) {
            sources.remove(pos);
            debug!("AudioMixer: Removed source for PID {}", pid);
        }
    }

    /// Get the number of active sources
    pub fn source_count(&self) -> usize {
        self.sources.read().len()
    }

    /// Mix all sources into the output buffer (simple version without per-app processing)
    ///
    /// This is called from the audio callback. It reads from all active sources
    /// and mixes them into the output buffer.
    ///
    /// Returns the number of samples written (may be less than buffer size).
    pub fn mix_into(&self, output: &mut [f32]) -> usize {
        self.mix_into_with_state(output, None)
    }

    /// Mix all sources into the output buffer with per-app EQ and volume
    ///
    /// This is called from the audio callback. For each source, it:
    /// 1. Reads audio from the ring buffer
    /// 2. Applies per-app EQ (BEFORE mixing - TRUE per-app EQ!)
    /// 3. Applies per-app volume
    /// 4. Mixes into the output buffer
    ///
    /// # Architecture
    /// ```text
    /// App1 Audio ──► Per-App EQ ──► Per-App Volume ──┐
    ///                                                 ├──► Mix ──► Output
    /// App2 Audio ──► Per-App EQ ──► Per-App Volume ──┘
    /// ```
    ///
    /// # Arguments
    /// * `output` - Output buffer to mix into
    /// * `state` - Optional processing state for per-app settings lookup
    ///
    /// # Returns
    /// The number of samples written (may be less than buffer size).
    pub fn mix_into_with_state(
        &self,
        output: &mut [f32],
        state: Option<&AudioProcessingState>,
    ) -> usize {
        output.fill(0.0);

        let sources = self.sources.read();
        if sources.is_empty() {
            return 0;
        }

        // Try to get the per-app equalizers lock (non-blocking for real-time safety)
        // If UI is updating EQ settings, skip per-app EQ for this buffer (inaudible)
        let mut app_eqs = self.app_equalizers.try_lock();

        // Temporary buffer for reading and processing each source
        let mut source_buffer = vec![0.0f32; output.len()];
        let mut max_samples = 0;

        for source in sources.iter() {
            let samples_read = source.ring_buffer.read(&mut source_buffer);
            if samples_read > 0 {
                // Get per-app volume and bypass state (default: volume=1.0, not bypassed)
                let (app_volume, app_bypassed) = if let Some(s) = state {
                    (
                        s.get_app_volume(&source.app_name),
                        s.is_app_bypassed(&source.app_name),
                    )
                } else {
                    (1.0, false)
                };

                // Skip this source if bypassed at the app level
                // (Note: app bypass means no audio from this app, not EQ bypass)
                if app_bypassed {
                    continue;
                }

                // Apply per-app EQ BEFORE mixing (this is the key to TRUE per-app EQ!)
                if let Some(ref mut eqs) = app_eqs {
                    // Get or create Equalizer for this app
                    let eq = eqs
                        .entry(source.app_name.clone())
                        .or_insert_with(|| Equalizer::new(self.sample_rate));

                    // Sync EQ gains from AudioProcessingState to the Equalizer
                    // This ensures UI changes are reflected in the per-app EQ
                    if let Some(s) = state {
                        if let Some(gains) = s.get_app_eq_gains(&source.app_name) {
                            // Apply all gains - this is cheap if values haven't changed
                            for (band, &gain_db) in gains.iter().enumerate() {
                                let _ = eq.set_band_gain(band, gain_db);
                            }
                        }
                    }

                    // Process audio through per-app EQ (in-place)
                    eq.process_interleaved(&mut source_buffer[..samples_read]);
                }
                // If lock unavailable, skip per-app EQ for this buffer (inaudible glitch)

                // Apply per-app volume and mix into output buffer
                for (out, &sample) in output.iter_mut().zip(source_buffer[..samples_read].iter()) {
                    *out += sample * app_volume;
                }
                max_samples = max_samples.max(samples_read);
            }
        }

        max_samples
    }
}

impl Default for AudioMixer {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared state for audio processing
///
/// This struct is shared between the main thread and the audio callback.
/// All fields use atomic or lock-free access patterns for real-time safety.
pub struct AudioProcessingState {
    /// Whether processing is bypassed
    pub bypassed: AtomicBool,

    /// Master volume (stored as u32 bits, interpreted as f32)
    master_volume_bits: AtomicU32,

    /// Peak level left channel
    peak_left_bits: AtomicU32,

    /// Peak level right channel
    peak_right_bits: AtomicU32,

    /// Whether the output stream is running
    pub running: AtomicBool,

    /// Master EQ band gains (10 bands, stored as u32 bits interpreted as f32)
    master_eq_gains: [AtomicU32; 10],

    /// Per-app EQ offsets (app_name → [10 bands of offset_db])
    /// These offsets are ADDED to master EQ to get final gains
    app_eq_offsets: RwLock<std::collections::HashMap<String, [f32; 10]>>,

    /// Per-app volume (app_name → volume 0.0-2.0)
    app_volumes: RwLock<std::collections::HashMap<String, f32>>,

    /// Per-app bypass state (app_name → bypassed)
    app_bypassed: RwLock<std::collections::HashMap<String, bool>>,

    /// Spectrum analyzer for FFT visualization
    spectrum_analyzer: RwLock<SpectrumAnalyzer>,

    /// Soft clipper to prevent harsh digital distortion
    soft_clipper: RwLock<SoftClipper>,

    /// Whether soft clipping is enabled
    soft_clip_enabled: AtomicBool,

    /// Master EQ processor (Mutex for &mut access in audio callback)
    /// Uses try_lock() in callback to avoid blocking - skips EQ if locked
    equalizer: Mutex<Equalizer>,

    /// Sample rate for EQ (needed if we recreate the equalizer)
    sample_rate: AtomicU32,
}

impl AudioProcessingState {
    /// Create new AudioProcessingState with default 48kHz sample rate
    pub fn new() -> Self {
        Self::with_sample_rate(48000.0)
    }

    /// Create new AudioProcessingState with specified sample rate
    pub fn with_sample_rate(sample_rate: f32) -> Self {
        // Initialize array of atomics with default 0dB gain
        let master_eq_gains = core::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits()));

        Self {
            bypassed: AtomicBool::new(false),
            master_volume_bits: AtomicU32::new(1.0_f32.to_bits()),
            peak_left_bits: AtomicU32::new(0.0_f32.to_bits()),
            peak_right_bits: AtomicU32::new(0.0_f32.to_bits()),
            running: AtomicBool::new(false),
            master_eq_gains,
            app_eq_offsets: RwLock::new(std::collections::HashMap::new()),
            app_volumes: RwLock::new(std::collections::HashMap::new()),
            app_bypassed: RwLock::new(std::collections::HashMap::new()),
            // FFT spectrum analyzer: sample_rate, ~60fps updates
            spectrum_analyzer: RwLock::new(SpectrumAnalyzer::new(sample_rate, 60)),
            // Soft clipper: -3dB threshold
            soft_clipper: RwLock::new(SoftClipper::new(-3.0)),
            soft_clip_enabled: AtomicBool::new(true),
            // Master EQ processor
            equalizer: Mutex::new(Equalizer::new(sample_rate)),
            sample_rate: AtomicU32::new(sample_rate.to_bits()),
        }
    }

    pub fn set_master_volume(&self, volume: f32) {
        self.master_volume_bits
            .store(volume.to_bits(), Ordering::Relaxed);
    }

    pub fn master_volume(&self) -> f32 {
        f32::from_bits(self.master_volume_bits.load(Ordering::Relaxed))
    }

    pub fn set_peaks(&self, left: f32, right: f32) {
        self.peak_left_bits.store(left.to_bits(), Ordering::Relaxed);
        self.peak_right_bits
            .store(right.to_bits(), Ordering::Relaxed);
    }

    pub fn peaks(&self) -> (f32, f32) {
        (
            f32::from_bits(self.peak_left_bits.load(Ordering::Relaxed)),
            f32::from_bits(self.peak_right_bits.load(Ordering::Relaxed)),
        )
    }

    pub fn set_bypassed(&self, bypassed: bool) {
        self.bypassed.store(bypassed, Ordering::Relaxed);
    }

    pub fn is_bypassed(&self) -> bool {
        self.bypassed.load(Ordering::Relaxed)
    }

    /// Set master EQ band gain
    ///
    /// Updates both the atomic storage (for UI sync) and the actual Equalizer.
    pub fn set_eq_band(&self, band: usize, gain_db: f32) {
        if band < 10 {
            // Store in atomic for UI reads
            self.master_eq_gains[band].store(gain_db.to_bits(), Ordering::Relaxed);

            // Update the actual Equalizer (blocks briefly if audio callback is processing)
            if let Some(mut eq) = self.equalizer.try_lock() {
                // Ignore errors from invalid gains - they'll be clamped anyway
                let _ = eq.set_band_gain(band, gain_db);
            }
        }
    }

    /// Get master EQ band gain
    pub fn get_eq_band(&self, band: usize) -> f32 {
        if band < 10 {
            f32::from_bits(self.master_eq_gains[band].load(Ordering::Relaxed))
        } else {
            0.0
        }
    }

    /// Get all master EQ gains
    pub fn get_eq_gains(&self) -> [f32; 10] {
        core::array::from_fn(|i| f32::from_bits(self.master_eq_gains[i].load(Ordering::Relaxed)))
    }

    /// Set all EQ bands at once
    ///
    /// More efficient than calling set_eq_band 10 times since it only
    /// locks the Equalizer once.
    pub fn set_all_eq_bands(&self, gains: &[f32; 10]) {
        // Store in atomics
        for (i, &gain) in gains.iter().enumerate() {
            self.master_eq_gains[i].store(gain.to_bits(), Ordering::Relaxed);
        }

        // Update the Equalizer
        if let Some(mut eq) = self.equalizer.try_lock() {
            for (i, &gain) in gains.iter().enumerate() {
                let _ = eq.set_band_gain(i, gain);
            }
        }
    }

    /// Process audio through the EQ
    ///
    /// Called from audio callback. Uses try_lock() to avoid blocking -
    /// if the lock is held (UI updating), EQ is bypassed for this buffer.
    ///
    /// # Returns
    /// true if EQ was applied, false if bypassed (lock unavailable)
    #[inline]
    pub fn process_eq(&self, buffer: &mut [f32]) -> bool {
        if let Some(mut eq) = self.equalizer.try_lock() {
            eq.process_interleaved(buffer);
            true
        } else {
            // Lock held by UI - skip EQ for this buffer (inaudible)
            false
        }
    }

    /// Reset EQ filter state (clears delay lines)
    ///
    /// Call when switching audio sources to prevent filter ringing.
    pub fn reset_eq(&self) {
        if let Some(mut eq) = self.equalizer.try_lock() {
            eq.reset();
        }
    }

    /// Get the sample rate
    pub fn sample_rate(&self) -> f32 {
        f32::from_bits(self.sample_rate.load(Ordering::Relaxed))
    }

    /// Set per-app EQ offset
    pub fn set_app_eq_offset(&self, app_name: &str, band: usize, offset_db: f32) {
        if band < 10 {
            let mut offsets = self.app_eq_offsets.write();
            let gains = offsets.entry(app_name.to_string()).or_insert([0.0; 10]);
            gains[band] = offset_db;
        }
    }

    /// Get per-app EQ gains (returns None if app has no EQ settings)
    ///
    /// Used by the mixer to sync EQ settings to per-app Equalizers.
    pub fn get_app_eq_gains(&self, app_name: &str) -> Option<[f32; 10]> {
        self.app_eq_offsets.read().get(app_name).copied()
    }

    /// Set per-app volume
    pub fn set_app_volume(&self, app_name: &str, volume: f32) {
        let mut volumes = self.app_volumes.write();
        volumes.insert(app_name.to_string(), volume.clamp(0.0, 2.0));
    }

    /// Get per-app volume
    pub fn get_app_volume(&self, app_name: &str) -> f32 {
        self.app_volumes
            .read()
            .get(app_name)
            .copied()
            .unwrap_or(1.0)
    }

    /// Set per-app bypass
    pub fn set_app_bypassed(&self, app_name: &str, bypassed: bool) {
        let mut states = self.app_bypassed.write();
        states.insert(app_name.to_string(), bypassed);
    }

    /// Check if app is bypassed
    pub fn is_app_bypassed(&self, app_name: &str) -> bool {
        self.app_bypassed
            .read()
            .get(app_name)
            .copied()
            .unwrap_or(false)
    }

    /// Push stereo sample pair to spectrum analyzer (for visualization)
    ///
    /// Call this for each stereo sample pair (left, right).
    pub fn push_spectrum_sample(&self, left: f32, right: f32) {
        // SpectrumAnalyzer uses interior mutability (atomics), so read() is fine
        self.spectrum_analyzer.read().push_sample(left, right);
    }

    /// Get spectrum data for visualization
    ///
    /// Returns the smoothed spectrum as an array of NUM_BINS (32) frequency bins.
    pub fn get_spectrum(&self) -> [f32; NUM_BINS] {
        self.spectrum_analyzer.read().get_spectrum()
    }

    /// Update spectrum analyzer (call from UI thread)
    ///
    /// Returns true if new spectrum data was computed.
    pub fn update_spectrum(&self) -> bool {
        self.spectrum_analyzer.read().update()
    }

    /// Apply soft clipping to a single sample
    #[inline]
    pub fn soft_clip_sample(&self, sample: f32) -> f32 {
        if self.soft_clip_enabled.load(Ordering::Relaxed) {
            self.soft_clipper.read().process_sample(sample)
        } else {
            sample
        }
    }

    /// Apply soft clipping to a buffer of samples
    pub fn apply_soft_clip(&self, buffer: &mut [f32]) {
        if self.soft_clip_enabled.load(Ordering::Relaxed) {
            let clipper = self.soft_clipper.read();
            for sample in buffer.iter_mut() {
                *sample = clipper.process_sample(*sample);
            }
        }
    }
}

impl Default for AudioProcessingState {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio output stream manager
///
/// Manages the cpal output stream and coordinates with Process Tap captures.
pub struct AudioOutputStream {
    /// The cpal output stream
    _stream: Stream,

    /// Output device being used
    _device: Device,

    /// Stream configuration
    config: StreamConfig,

    /// Shared processing state
    state: Arc<AudioProcessingState>,

    /// Audio mixer for combining multiple sources
    mixer: Arc<AudioMixer>,
}

impl AudioOutputStream {
    /// Create a new audio output stream to the default output device
    ///
    /// # Arguments
    /// * `state` - Shared processing state for DSP settings
    /// * `mixer` - Audio mixer containing the sources to mix
    pub fn new_with_mixer(
        state: Arc<AudioProcessingState>,
        mixer: Arc<AudioMixer>,
    ) -> Result<Self, PlatformError> {
        let host = cpal::default_host();

        let device = host.default_output_device().ok_or_else(|| {
            PlatformError::Internal("No default output device found".into())
        })?;

        let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        debug!("Using output device: {}", device_name);

        // Get supported config
        let supported_config = device
            .default_output_config()
            .map_err(|e| PlatformError::Internal(format!("Failed to get output config: {}", e)))?;

        let sample_format = supported_config.sample_format();
        let config: StreamConfig = supported_config.into();

        debug!(
            "Output stream config: {} Hz, {} channels, {:?}",
            config.sample_rate.0, config.channels, sample_format
        );

        // Create the output stream based on sample format
        let stream = match sample_format {
            SampleFormat::F32 => Self::build_stream::<f32>(
                &device,
                &config,
                Arc::clone(&state),
                Arc::clone(&mixer),
            )?,
            SampleFormat::I16 => Self::build_stream::<i16>(
                &device,
                &config,
                Arc::clone(&state),
                Arc::clone(&mixer),
            )?,
            SampleFormat::U16 => Self::build_stream::<u16>(
                &device,
                &config,
                Arc::clone(&state),
                Arc::clone(&mixer),
            )?,
            _ => {
                return Err(PlatformError::Internal(format!(
                    "Unsupported sample format: {:?}",
                    sample_format
                )))
            }
        };

        // Start the stream
        stream.play().map_err(|e| {
            PlatformError::Internal(format!("Failed to start output stream: {}", e))
        })?;

        state.running.store(true, Ordering::SeqCst);
        debug!("Audio output stream started");

        Ok(Self {
            _stream: stream,
            _device: device,
            config,
            state,
            mixer,
        })
    }

    /// Create a new audio output stream with an empty mixer
    ///
    /// Convenience method that creates its own mixer. Use `new_with_mixer`
    /// if you need to share the mixer with other components.
    pub fn new(state: Arc<AudioProcessingState>) -> Result<Self, PlatformError> {
        Self::new_with_mixer(state, Arc::new(AudioMixer::new()))
    }

    /// Get a reference to the mixer (for adding/removing sources)
    pub fn mixer(&self) -> &Arc<AudioMixer> {
        &self.mixer
    }

    /// Build the output stream for a specific sample format
    fn build_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
        device: &Device,
        config: &StreamConfig,
        state: Arc<AudioProcessingState>,
        mixer: Arc<AudioMixer>,
    ) -> Result<Stream, PlatformError> {
        let channels = config.channels as usize;

        // Error callback
        let err_fn = |err| error!("Audio output error: {}", err);

        // Data callback - this is where audio processing happens
        let data_callback = move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            // Create a temporary f32 buffer for processing
            let num_frames = data.len() / channels;
            let mut process_buffer = vec![0.0f32; data.len()];

            // Mix all sources into the buffer with per-app volume support
            // Pass state reference so mixer can look up per-app volumes
            let samples_read = mixer.mix_into_with_state(&mut process_buffer, Some(&state));

            // Debug tracking (safe atomics - no logging in callback)
            CALLBACK_COUNT.fetch_add(1, Ordering::Relaxed);
            if samples_read > 0 {
                TOTAL_SAMPLES_MIXED.fetch_add(samples_read, Ordering::Relaxed);
            }

            // If we got audio, process it
            if samples_read > 0 && !state.is_bypassed() {
                // Apply EQ (10-band parametric equalizer)
                // Uses try_lock() internally - if UI is updating EQ, skip for this buffer
                state.process_eq(&mut process_buffer);

                // Apply master volume
                let volume = state.master_volume();
                for sample in process_buffer.iter_mut() {
                    *sample *= volume;
                }

                // Apply soft clipping (prevents harsh digital distortion)
                state.apply_soft_clip(&mut process_buffer);

                // Track peaks (L/R from interleaved stereo)
                let mut peak_l = 0.0f32;
                let mut peak_r = 0.0f32;
                for (i, &sample) in process_buffer.iter().enumerate() {
                    if i % 2 == 0 {
                        peak_l = peak_l.max(sample.abs());
                    } else {
                        peak_r = peak_r.max(sample.abs());
                    }
                }
                state.set_peaks(peak_l, peak_r);

                // Feed to spectrum analyzer (L channel only for simplicity)
                for frame in 0..num_frames {
                    let left = process_buffer[frame * channels];
                    let right = if channels > 1 {
                        process_buffer[frame * channels + 1]
                    } else {
                        left
                    };
                    state.push_spectrum_sample(left, right);
                }
            } else {
                // No audio or bypassed - clear peaks
                state.set_peaks(0.0, 0.0);
            }

            // Convert to output format
            for (i, sample) in data.iter_mut().enumerate() {
                *sample = T::from_sample(process_buffer[i]);
            }
        };

        let stream = device
            .build_output_stream(config, data_callback, err_fn, None)
            .map_err(|e| PlatformError::Internal(format!("Failed to build output stream: {}", e)))?;

        Ok(stream)
    }

    /// Get the sample rate
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate.0
    }

    /// Get the number of channels
    pub fn channels(&self) -> u16 {
        self.config.channels
    }

    /// Check if the stream is running
    pub fn is_running(&self) -> bool {
        self.state.running.load(Ordering::Acquire)
    }

    /// Stop the output stream
    pub fn stop(&self) {
        self.state.running.store(false, Ordering::SeqCst);
        debug!("Audio output stream stopped");
    }
}

impl Drop for AudioOutputStream {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Get the number of audio callback invocations (for debugging)
pub fn get_callback_count() -> usize {
    CALLBACK_COUNT.load(Ordering::Relaxed)
}

/// Get the total samples mixed (for debugging)
pub fn get_total_samples_mixed() -> usize {
    TOTAL_SAMPLES_MIXED.load(Ordering::Relaxed)
}

/// Get debug stats as a formatted string
pub fn get_mixer_debug_stats() -> String {
    format!(
        "Mixer: callbacks={}, total_samples_mixed={}",
        get_callback_count(),
        get_total_samples_mixed()
    )
}

/// Reset debug counters
pub fn reset_debug_counters() {
    CALLBACK_COUNT.store(0, Ordering::Relaxed);
    TOTAL_SAMPLES_MIXED.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_processing_state_creation() {
        let state = AudioProcessingState::new();
        assert!(!state.is_bypassed());
        assert!((state.master_volume() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_master_volume() {
        let state = AudioProcessingState::new();
        state.set_master_volume(0.5);
        assert!((state.master_volume() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_eq_bands() {
        let state = AudioProcessingState::new();
        state.set_eq_band(0, 3.0);
        state.set_eq_band(5, -2.0);

        assert!((state.get_eq_band(0) - 3.0).abs() < 0.001);
        assert!((state.get_eq_band(5) - (-2.0)).abs() < 0.001);
    }

    #[test]
    fn test_app_volume() {
        let state = AudioProcessingState::new();
        state.set_app_volume("Firefox", 0.8);
        assert!((state.get_app_volume("Firefox") - 0.8).abs() < 0.001);
        // Default volume for unknown app
        assert!((state.get_app_volume("Unknown") - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_peaks() {
        let state = AudioProcessingState::new();
        state.set_peaks(0.5, 0.7);
        let (left, right) = state.peaks();
        assert!((left - 0.5).abs() < 0.001);
        assert!((right - 0.7).abs() < 0.001);
    }

    // =========================================================================
    // Per-App EQ Tests for AudioProcessingState
    // =========================================================================

    #[test]
    fn test_app_eq_offset_single_band() {
        // Tests setting and getting a single per-app EQ band offset
        let state = AudioProcessingState::new();
        state.set_app_eq_offset("Firefox", 0, 3.0);
        state.set_app_eq_offset("Firefox", 5, -2.5);

        let gains = state.get_app_eq_gains("Firefox");
        assert!(gains.is_some());
        let gains = gains.unwrap();
        assert!((gains[0] - 3.0).abs() < 0.001);
        assert!((gains[5] - (-2.5)).abs() < 0.001);
        // Other bands should be 0.0 (default)
        assert!((gains[1] - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_app_eq_offset_unknown_app() {
        // Unknown apps should return None for gains
        let state = AudioProcessingState::new();
        let gains = state.get_app_eq_gains("UnknownApp");
        assert!(gains.is_none());
    }

    #[test]
    fn test_app_eq_offset_multiple_apps() {
        // Each app should have independent EQ settings
        let state = AudioProcessingState::new();
        state.set_app_eq_offset("Firefox", 0, 3.0);
        state.set_app_eq_offset("Spotify", 0, -3.0);

        let firefox_gains = state.get_app_eq_gains("Firefox").unwrap();
        let spotify_gains = state.get_app_eq_gains("Spotify").unwrap();

        assert!((firefox_gains[0] - 3.0).abs() < 0.001);
        assert!((spotify_gains[0] - (-3.0)).abs() < 0.001);
    }

    #[test]
    fn test_app_eq_offset_out_of_bounds() {
        // Setting band > 9 should be silently ignored
        let state = AudioProcessingState::new();
        state.set_app_eq_offset("Firefox", 10, 5.0); // Band 10 doesn't exist
        state.set_app_eq_offset("Firefox", 100, 5.0); // Way out of bounds

        // App entry might not even be created if only invalid bands were set
        // If it was created, band 10 and 100 shouldn't cause issues
        let gains = state.get_app_eq_gains("Firefox");
        // Either None (no valid bands set) or all zeros
        if let Some(g) = gains {
            for gain in g.iter() {
                assert!((*gain - 0.0).abs() < 0.001);
            }
        }
    }

    // =========================================================================
    // Per-App EQ Tests for AudioMixer
    // =========================================================================

    #[test]
    fn test_mixer_app_eq_band() {
        // Tests AudioMixer's set_app_eq_band and get_app_eq_band
        let mixer = AudioMixer::new();
        mixer.set_app_eq_band("Firefox", 0, 6.0);
        mixer.set_app_eq_band("Firefox", 9, -6.0);

        let gain0 = mixer.get_app_eq_band("Firefox", 0);
        let gain9 = mixer.get_app_eq_band("Firefox", 9);
        let gain5 = mixer.get_app_eq_band("Firefox", 5); // Not set, should be 0

        assert!((gain0 - 6.0).abs() < 0.001);
        assert!((gain9 - (-6.0)).abs() < 0.001);
        assert!((gain5 - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_mixer_app_eq_bands_bulk() {
        // Tests AudioMixer's set_app_eq_bands and get_app_eq_bands
        let mixer = AudioMixer::new();
        let gains: [f32; 10] = [1.0, 2.0, 3.0, 4.0, 5.0, -1.0, -2.0, -3.0, -4.0, -5.0];
        mixer.set_app_eq_bands("Spotify", &gains);

        let retrieved = mixer.get_app_eq_bands("Spotify");
        for i in 0..10 {
            assert!(
                (retrieved[i] - gains[i]).abs() < 0.001,
                "Band {} mismatch: expected {}, got {}",
                i,
                gains[i],
                retrieved[i]
            );
        }
    }

    #[test]
    fn test_mixer_app_eq_unknown_app() {
        // Getting EQ for unknown app should return zeros
        let mixer = AudioMixer::new();
        let gains = mixer.get_app_eq_bands("UnknownApp");
        for (i, gain) in gains.iter().enumerate() {
            assert!(
                (*gain - 0.0).abs() < 0.001,
                "Band {} should be 0.0, got {}",
                i,
                gain
            );
        }
    }

    #[test]
    fn test_mixer_app_eq_multiple_apps_independent() {
        // Multiple apps should have independent EQ settings
        let mixer = AudioMixer::new();

        mixer.set_app_eq_band("Firefox", 0, 6.0);
        mixer.set_app_eq_band("Spotify", 0, -6.0);
        mixer.set_app_eq_band("Discord", 5, 3.0);

        assert!((mixer.get_app_eq_band("Firefox", 0) - 6.0).abs() < 0.001);
        assert!((mixer.get_app_eq_band("Spotify", 0) - (-6.0)).abs() < 0.001);
        assert!((mixer.get_app_eq_band("Discord", 5) - 3.0).abs() < 0.001);

        // Cross-app isolation: Discord band 0 should be 0, not affected by Firefox/Spotify
        assert!((mixer.get_app_eq_band("Discord", 0) - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_mixer_app_eq_out_of_bounds_band() {
        // Out of bounds band should return 0.0 for get
        let mixer = AudioMixer::new();
        mixer.set_app_eq_band("Firefox", 0, 6.0);

        // Band 10 and 100 are out of bounds
        let gain10 = mixer.get_app_eq_band("Firefox", 10);
        let gain100 = mixer.get_app_eq_band("Firefox", 100);

        assert!((gain10 - 0.0).abs() < 0.001);
        assert!((gain100 - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_mixer_creation() {
        // Test basic mixer creation and that EQ returns expected defaults
        let mixer = AudioMixer::new();
        let gains = mixer.get_app_eq_bands("TestApp");
        assert_eq!(gains.len(), 10);
        // All gains should default to 0.0
        for gain in gains.iter() {
            assert!((*gain - 0.0).abs() < 0.001);
        }
    }
}
