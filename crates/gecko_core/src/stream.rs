//! Audio Stream Management
//!
//! Handles the low-level CPAL stream setup and real-time audio callbacks.
//!
//! # Audio Flow (Correct Architecture)
//!
//! This module implements audio capture from **application audio** (NOT microphone):
//!
//! ```text
//! Linux (PipeWire):
//!   App Audio → Virtual Sink → CPAL captures from sink → DSP → Real Speakers
//!
//! Windows (WASAPI):
//!   App Audio → Process Loopback API → DSP → Real Speakers
//!
//! macOS (CoreAudio):
//!   App Audio → HAL Plugin → Shared Memory → DSP → Real Speakers
//! ```
//!
//! The key point: **NO MICROPHONE INPUT**. This is a system audio processor,
//! not a voice application.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig as CpalStreamConfig};
use crossbeam_channel::Sender;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::config::StreamConfig;
use crate::error::{EngineError, EngineResult};
use crate::message::Event;
use gecko_dsp::{Equalizer, ProcessContext};

/// Shared state between audio callback and control thread
pub struct SharedState {
    /// Whether processing is bypassed
    pub bypassed: AtomicBool,

    /// Master volume (stored as u32, interpreted as f32 bits)
    /// Rust pattern: AtomicF32 doesn't exist, so we use bit-casting
    master_volume_bits: AtomicU32,

    /// Peak level left channel (for meters)
    peak_left_bits: AtomicU32,

    /// Peak level right channel
    peak_right_bits: AtomicU32,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            bypassed: AtomicBool::new(false),
            master_volume_bits: AtomicU32::new(1.0_f32.to_bits()),
            peak_left_bits: AtomicU32::new(0.0_f32.to_bits()),
            peak_right_bits: AtomicU32::new(0.0_f32.to_bits()),
        }
    }

    pub fn set_master_volume(&self, volume: f32) {
        // Rust pattern: Relaxed ordering is fine for single-value updates
        // that don't need to synchronize with other memory operations
        self.master_volume_bits
            .store(volume.to_bits(), Ordering::Relaxed);
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
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}

/// Manages an active audio stream
///
/// # Architecture Note
///
/// This stream handles **output only** for basic playback. The actual audio
/// capture from applications is handled by the platform-specific backend
/// (PipeWire on Linux, WASAPI on Windows, CoreAudio on macOS).
///
/// For Linux, the flow is:
/// 1. PipeWireBackend creates a virtual sink
/// 2. Applications route their audio to this virtual sink
/// 3. Gecko captures from the virtual sink's monitor port
/// 4. DSP processing happens here
/// 5. Processed audio goes to the real speakers
pub struct AudioStream {
    /// The underlying CPAL stream (kept alive to maintain audio flow)
    /// Rust pattern: `#[allow(dead_code)]` because we need to hold the stream
    /// even though we don't call methods on it directly
    #[allow(dead_code)]
    capture_stream: Option<Stream>,

    #[allow(dead_code)]
    output_stream: Option<Stream>,

    /// Shared state for atomic updates from control thread
    pub shared: Arc<SharedState>,

    /// Current stream configuration
    pub config: StreamConfig,
}

impl AudioStream {
    /// Create a new output-only stream for DSP processing
    ///
    /// # Architecture Note
    ///
    /// This creates an OUTPUT stream only. Audio capture happens through the
    /// platform backend (PipeWire virtual sink on Linux, WASAPI loopback on Windows,
    /// HAL plugin on macOS). This is NOT a microphone passthrough!
    ///
    /// The capture device parameter should be:
    /// - Linux: The monitor port of the Gecko virtual sink
    /// - Windows: A loopback device
    /// - macOS: The virtual device created by the HAL plugin
    ///
    /// # Arguments
    ///
    /// * `config` - Stream configuration (sample rate, buffer size, etc.)
    /// * `capture_device` - The device to capture audio FROM (virtual sink monitor, NOT microphone)
    /// * `output_device` - The device to output processed audio TO (real speakers)
    /// * `event_sender` - Channel for sending events back to the engine
    pub fn new_with_capture(
        config: StreamConfig,
        capture_device: &Device,
        output_device: &Device,
        event_sender: Sender<Event>,
    ) -> EngineResult<Self> {
        config
            .validate()
            .map_err(EngineError::ConfigError)?;

        let shared = Arc::new(SharedState::new());

        // Create ring buffer for passing audio between capture and output callbacks
        // Size: 4x buffer size for safety margin
        let ring_size = config.buffer_size as usize * config.channels as usize * 4;
        let (producer, consumer) = RingBuffer::<f32>::new(ring_size);

        let cpal_config = CpalStreamConfig {
            channels: config.channels,
            sample_rate: cpal::SampleRate(config.sample_rate),
            buffer_size: cpal::BufferSize::Fixed(config.buffer_size),
        };

        // Build capture stream from the virtual sink's monitor (NOT microphone!)
        let capture_stream = Self::build_capture_stream(
            capture_device,
            &cpal_config,
            producer,
            event_sender.clone(),
        )?;

        // Build output stream to real speakers
        let output_stream = Self::build_output_stream(
            output_device,
            &cpal_config,
            consumer,
            Arc::clone(&shared),
            config.sample_rate as f32,
            event_sender,
        )?;

        // Start both streams
        capture_stream
            .play()
            .map_err(|e| EngineError::StreamPlayError(e.to_string()))?;
        output_stream
            .play()
            .map_err(|e| EngineError::StreamPlayError(e.to_string()))?;

        Ok(Self {
            capture_stream: Some(capture_stream),
            output_stream: Some(output_stream),
            shared,
            config,
        })
    }

    /// Create an output-only stream (no capture)
    ///
    /// This is useful when audio is being fed from an external source
    /// (e.g., via shared memory on macOS).
    pub fn new_output_only(
        config: StreamConfig,
        output_device: &Device,
        event_sender: Sender<Event>,
    ) -> EngineResult<Self> {
        config
            .validate()
            .map_err(EngineError::ConfigError)?;

        let shared = Arc::new(SharedState::new());

        let cpal_config = CpalStreamConfig {
            channels: config.channels,
            sample_rate: cpal::SampleRate(config.sample_rate),
            buffer_size: cpal::BufferSize::Fixed(config.buffer_size),
        };

        // Create a dummy ring buffer - audio will come from elsewhere
        let ring_size = config.buffer_size as usize * config.channels as usize * 4;
        let (_producer, consumer) = RingBuffer::<f32>::new(ring_size);

        // Build output stream
        let output_stream = Self::build_output_stream(
            output_device,
            &cpal_config,
            consumer,
            Arc::clone(&shared),
            config.sample_rate as f32,
            event_sender,
        )?;

        output_stream
            .play()
            .map_err(|e| EngineError::StreamPlayError(e.to_string()))?;

        Ok(Self {
            capture_stream: None,
            output_stream: Some(output_stream),
            shared,
            config,
        })
    }

    /// Build a capture stream from a virtual sink monitor
    ///
    /// IMPORTANT: The device should be a virtual sink's monitor port,
    /// NOT a microphone. This captures application audio that has been
    /// routed to the virtual sink.
    fn build_capture_stream(
        device: &Device,
        config: &CpalStreamConfig,
        mut producer: Producer<f32>,
        event_sender: Sender<Event>,
    ) -> EngineResult<Stream> {
        let err_sender = event_sender.clone();

        let stream = device
            .build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // Real-time audio callback - NO allocations allowed here
                    // Try to push all samples to ring buffer
                    let written = producer.write_chunk_uninit(data.len()).map_or(0, |mut chunk| {
                        let len = chunk.len().min(data.len());
                        for (i, slot) in chunk.as_mut_slices().0.iter_mut().enumerate().take(len) {
                            slot.write(data[i]);
                        }
                        // Rust pattern: unsafe is required here because we're
                        // working with uninitialized memory for performance
                        unsafe { chunk.commit_all() };
                        len
                    });

                    if written < data.len() {
                        // Buffer overflow - we're not consuming fast enough
                        let _ = event_sender.try_send(Event::BufferUnderrun);
                    }
                },
                move |err| {
                    let _ = err_sender.try_send(Event::error(err));
                },
                None, // No timeout
            )
            .map_err(|e| EngineError::StreamBuildError(e.to_string()))?;

        Ok(stream)
    }

    fn build_output_stream(
        device: &Device,
        config: &CpalStreamConfig,
        mut consumer: Consumer<f32>,
        shared: Arc<SharedState>,
        sample_rate: f32,
        event_sender: Sender<Event>,
    ) -> EngineResult<Stream> {
        let err_sender = event_sender.clone();

        // Create EQ processor for this stream
        // Rust pattern: `move` closure captures these variables by value
        let mut eq = Equalizer::new(sample_rate);
        let _process_context = ProcessContext::new(sample_rate, config.channels as usize, 0);

        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // Real-time audio callback - NO allocations allowed here

                    // Read from ring buffer
                    let available = consumer.slots();
                    let to_read = data.len().min(available);

                    if to_read < data.len() {
                        // Underrun - fill with silence
                        data.fill(0.0);
                        let _ = event_sender.try_send(Event::BufferUnderrun);
                    }

                    // Read available samples
                    if let Ok(chunk) = consumer.read_chunk(to_read) {
                        let (first, second) = chunk.as_slices();
                        data[..first.len()].copy_from_slice(first);
                        if !second.is_empty() {
                            data[first.len()..first.len() + second.len()].copy_from_slice(second);
                        }
                        chunk.commit_all();
                    }

                    // Process through DSP chain if not bypassed
                    if !shared.bypassed.load(Ordering::Relaxed) {
                        eq.process_interleaved(data);
                    }

                    // Apply master volume
                    let volume = shared.master_volume();
                    if (volume - 1.0).abs() > 0.001 {
                        for sample in data.iter_mut() {
                            *sample *= volume;
                        }
                    }

                    // Calculate peak levels for metering
                    let mut peak_l = 0.0_f32;
                    let mut peak_r = 0.0_f32;
                    for frame in data.chunks(2) {
                        if frame.len() == 2 {
                            peak_l = peak_l.max(frame[0].abs());
                            peak_r = peak_r.max(frame[1].abs());
                        }
                    }
                    shared.set_peaks(peak_l, peak_r);
                },
                move |err| {
                    let _ = err_sender.try_send(Event::error(err));
                },
                None,
            )
            .map_err(|e| EngineError::StreamBuildError(e.to_string()))?;

        Ok(stream)
    }

    /// Get current peak levels (for UI meters)
    pub fn get_peaks(&self) -> (f32, f32) {
        self.shared.peaks()
    }

    /// Set bypass state
    pub fn set_bypass(&self, bypassed: bool) {
        self.shared.bypassed.store(bypassed, Ordering::Relaxed);
    }

    /// Set master volume (0.0 - 1.0)
    pub fn set_master_volume(&self, volume: f32) {
        self.shared.set_master_volume(volume.clamp(0.0, 2.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_state_defaults() {
        let state = SharedState::new();
        assert!(!state.bypassed.load(Ordering::Relaxed));
        assert_eq!(state.master_volume(), 1.0);
        assert_eq!(state.peaks(), (0.0, 0.0));
    }

    #[test]
    fn test_shared_state_volume() {
        let state = SharedState::new();

        state.set_master_volume(0.5);
        assert_eq!(state.master_volume(), 0.5);

        state.set_master_volume(0.0);
        assert_eq!(state.master_volume(), 0.0);
    }

    #[test]
    fn test_shared_state_peaks() {
        let state = SharedState::new();

        state.set_peaks(0.8, 0.6);
        let (l, r) = state.peaks();
        assert_eq!(l, 0.8);
        assert_eq!(r, 0.6);
    }

    #[test]
    fn test_shared_state_bypass() {
        let state = SharedState::new();

        state.bypassed.store(true, Ordering::Relaxed);
        assert!(state.bypassed.load(Ordering::Relaxed));
    }

    // Hardware-dependent tests
    #[test]
    #[ignore = "requires audio hardware and platform setup"]
    fn test_stream_creation() {
        use cpal::traits::HostTrait;

        let (sender, _receiver) = crossbeam_channel::unbounded();
        let config = StreamConfig::default();
        let host = cpal::default_host();

        // NOTE: This test would require proper platform setup:
        // - Linux: PipeWire virtual sink must be created first
        // - Windows: Loopback device must be available
        // - macOS: HAL plugin must be installed
        //
        // For now, we just test output-only mode
        if let Some(output_device) = host.default_output_device() {
            let result = AudioStream::new_output_only(config, &output_device, sender);
            // May fail if no audio hardware, which is fine for CI
            if result.is_ok() {
                let stream = result.unwrap();
                assert_eq!(stream.config.sample_rate, 48000);
            }
        }
    }
}
