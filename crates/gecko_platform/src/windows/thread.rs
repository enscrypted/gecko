//! WASAPI Audio Thread
//!
//! Dedicated thread for WASAPI audio operations. Handles:
//! - System-wide loopback capture
//! - Per-process loopback capture (Windows 10 Build 20348+)
//! - Audio output (playback)
//! - DSP processing (EQ) with real-time safety
//!
//! # Architecture
//!
//! ```text
//! Main Thread                    WASAPI Thread
//! ───────────                    ────────────
//! WasapiBackend                  WasapiThread
//!   │                              │
//!   ├── command_tx ────────────►   │ process commands
//!   │                              │
//!   └── state (Arc) ◄────────────► │ shared atomics
//! ```
//!
//! # Real-Time Safety
//!
//! The audio callback path follows strict rules:
//! - NO heap allocations (buffers pre-allocated)
//! - NO blocking operations (atomics only)
//! - NO syscalls (no logging, no I/O)
//! - O(n) time complexity where n = buffer size

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, TryRecvError};

use crate::error::PlatformError;
use super::message::{AudioProcessingState, WasapiCommand, WasapiResponse};

#[cfg(target_os = "windows")]
use gecko_dsp::AudioProcessor;

/// WASAPI thread controller
///
/// Manages the dedicated audio thread and provides communication channels.
pub struct WasapiThreadHandle {
    /// Command sender (main thread → WASAPI thread)
    command_tx: Sender<WasapiCommand>,
    /// Response receiver (WASAPI thread → main thread)
    response_rx: Receiver<WasapiResponse>,
    /// Shared audio processing state
    state: Arc<AudioProcessingState>,
    /// Thread handle
    thread_handle: Option<JoinHandle<()>>,
    /// Shutdown flag
    shutdown: Arc<AtomicBool>,
}

impl WasapiThreadHandle {
    /// Spawn a new WASAPI thread
    ///
    /// Returns a handle for communication and a shared state reference.
    pub fn spawn() -> Result<Self, PlatformError> {
        let (command_tx, command_rx) = crossbeam_channel::bounded(64);
        let (response_tx, response_rx) = crossbeam_channel::bounded(64);

        let state = Arc::new(AudioProcessingState::new());
        let state_clone = Arc::clone(&state);

        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let thread_handle = thread::Builder::new()
            .name("gecko-wasapi".into())
            .spawn(move || {
                wasapi_thread_main(command_rx, response_tx, state_clone, shutdown_clone);
            })
            .map_err(|e| {
                PlatformError::InitializationFailed(format!("Failed to spawn WASAPI thread: {}", e))
            })?;

        tracing::info!("WASAPI thread spawned");

        Ok(Self {
            command_tx,
            response_rx,
            state,
            thread_handle: Some(thread_handle),
            shutdown,
        })
    }

    /// Send a command to the WASAPI thread
    pub fn send_command(&self, cmd: WasapiCommand) -> Result<(), PlatformError> {
        self.command_tx.send(cmd).map_err(|e| {
            PlatformError::Internal(format!("Failed to send command: {}", e))
        })
    }

    /// Try to receive a response (non-blocking)
    pub fn try_recv_response(&self) -> Option<WasapiResponse> {
        self.response_rx.try_recv().ok()
    }

    /// Receive a response (blocking with timeout)
    pub fn recv_response_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Option<WasapiResponse> {
        self.response_rx.recv_timeout(timeout).ok()
    }

    /// Get shared audio processing state
    pub fn state(&self) -> &Arc<AudioProcessingState> {
        &self.state
    }

    /// Shutdown the WASAPI thread
    pub fn shutdown(&mut self) -> Result<(), PlatformError> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Ok(());
        }

        tracing::info!("Shutting down WASAPI thread");

        self.shutdown.store(true, Ordering::SeqCst);

        // Send shutdown command
        let _ = self.command_tx.send(WasapiCommand::Shutdown);

        // Wait for thread to finish
        if let Some(handle) = self.thread_handle.take() {
            handle.join().map_err(|_| {
                PlatformError::Internal("WASAPI thread panicked".into())
            })?;
        }

        tracing::info!("WASAPI thread shutdown complete");

        Ok(())
    }
}

impl Drop for WasapiThreadHandle {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

// ============================================================================
// WASAPI Thread Implementation
// ============================================================================

/// Main entry point for WASAPI thread
#[cfg(target_os = "windows")]
fn wasapi_thread_main(
    command_rx: Receiver<WasapiCommand>,
    response_tx: Sender<WasapiResponse>,
    state: Arc<AudioProcessingState>,
    shutdown: Arc<AtomicBool>,
) {
    use super::com::ComGuard;

    tracing::debug!("WASAPI thread starting");

    // Initialize COM for this thread (required for WASAPI)
    let _com = match ComGuard::new() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to initialize COM in WASAPI thread: {}", e);
            let _ = response_tx.send(WasapiResponse::Error(e.to_string()));
            return;
        }
    };

    // Create thread state
    let mut thread_state = WasapiThreadState::new(state, response_tx);

    // Main loop
    while !shutdown.load(Ordering::SeqCst) {
        // Process commands (non-blocking)
        match command_rx.try_recv() {
            Ok(cmd) => {
                if !thread_state.handle_command(cmd) {
                    break; // Shutdown requested
                }
            }
            Err(TryRecvError::Empty) => {
                // No commands, continue
            }
            Err(TryRecvError::Disconnected) => {
                tracing::warn!("Command channel disconnected");
                break;
            }
        }

        // Process audio if streams are active
        if thread_state.is_streaming() {
            thread_state.process_audio();
        }

        // Small sleep to prevent busy-waiting when idle
        // When streaming, WASAPI events will wake us
        if !thread_state.is_streaming() {
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    // Cleanup
    thread_state.stop_all();

    tracing::debug!("WASAPI thread exiting");
}

#[cfg(not(target_os = "windows"))]
fn wasapi_thread_main(
    _command_rx: Receiver<WasapiCommand>,
    response_tx: Sender<WasapiResponse>,
    _state: Arc<AudioProcessingState>,
    _shutdown: Arc<AtomicBool>,
) {
    let _ = response_tx.send(WasapiResponse::Error(
        "WASAPI only available on Windows".into(),
    ));
}

// ============================================================================
// Thread State
// ============================================================================

/// Internal state for the WASAPI thread
#[cfg(target_os = "windows")]
struct WasapiThreadState {
    /// Shared audio processing state
    state: Arc<AudioProcessingState>,
    /// Response sender
    response_tx: Sender<WasapiResponse>,
    /// Capture stream (loopback)
    capture: Option<LoopbackCapture>,
    /// Output stream (playback)
    output: Option<AudioOutput>,
    /// Ring buffer for capture → output transfer
    /// Using rtrb for lock-free SPSC
    ring_producer: Option<rtrb::Producer<f32>>,
    ring_consumer: Option<rtrb::Consumer<f32>>,
    /// EQ processor
    equalizer: gecko_dsp::Equalizer,
    /// Local EQ update counter
    eq_update_counter: u32,
    /// Pre-allocated processing buffer (avoids allocation in audio path)
    process_buffer: Vec<f32>,
    /// Pre-allocated output buffer
    output_buffer: Vec<f32>,
}

#[cfg(target_os = "windows")]
impl WasapiThreadState {
    fn new(state: Arc<AudioProcessingState>, response_tx: Sender<WasapiResponse>) -> Self {
        // Pre-allocate buffers for real-time safety
        // 8192 samples = ~170ms at 48kHz stereo
        let buffer_size = 8192;

        // Create ring buffer (64k samples ≈ 1.3 seconds at 48kHz stereo)
        let (producer, consumer) = rtrb::RingBuffer::new(65536);

        Self {
            state,
            response_tx,
            capture: None,
            output: None,
            ring_producer: Some(producer),
            ring_consumer: Some(consumer),
            equalizer: gecko_dsp::Equalizer::new(48000.0),
            eq_update_counter: 0,
            process_buffer: vec![0.0; buffer_size],
            output_buffer: vec![0.0; buffer_size],
        }
    }

    fn handle_command(&mut self, cmd: WasapiCommand) -> bool {
        match cmd {
            WasapiCommand::StartCapture { pid, app_name } => {
                tracing::info!("Starting capture for {:?} ({})", pid, app_name);
                match self.start_capture(pid) {
                    Ok(()) => {
                        let _ = self.response_tx.send(WasapiResponse::CaptureStarted {
                            pid,
                            app_name,
                        });
                    }
                    Err(e) => {
                        let _ = self.response_tx.send(WasapiResponse::Error(e.to_string()));
                    }
                }
            }
            WasapiCommand::StopCapture { pid } => {
                tracing::info!("Stopping capture for PID {}", pid);
                self.stop_capture();
                let _ = self.response_tx.send(WasapiResponse::CaptureStopped { pid });
            }
            WasapiCommand::StartOutput => {
                tracing::info!("Starting audio output");
                match self.start_output() {
                    Ok(()) => {
                        let _ = self.response_tx.send(WasapiResponse::OutputStarted);
                    }
                    Err(e) => {
                        let _ = self.response_tx.send(WasapiResponse::Error(e.to_string()));
                    }
                }
            }
            WasapiCommand::StopOutput => {
                tracing::info!("Stopping audio output");
                self.stop_output();
                let _ = self.response_tx.send(WasapiResponse::OutputStopped);
            }
            WasapiCommand::SetMasterVolume(vol) => {
                self.state.set_master_volume(vol);
            }
            WasapiCommand::SetMasterBypass(bypass) => {
                self.state.set_bypass(bypass);
            }
            WasapiCommand::SetMasterEqGains(gains) => {
                self.state.set_master_eq_gains(&gains);
            }
            WasapiCommand::Shutdown => {
                tracing::info!("Shutdown command received");
                return false;
            }
            // Other commands not yet implemented
            _ => {
                tracing::warn!("Unhandled command: {:?}", cmd);
            }
        }
        true
    }

    fn start_capture(&mut self, pid: Option<u32>) -> Result<(), PlatformError> {
        if self.capture.is_some() {
            // Already capturing, stop first
            self.stop_capture();
        }

        let capture = if let Some(target_pid) = pid {
            // Per-process capture (requires Build 20348+)
            LoopbackCapture::new_process(target_pid)?
        } else {
            // System-wide loopback
            LoopbackCapture::new_system()?
        };

        self.capture = Some(capture);
        self.state.running.store(true, Ordering::SeqCst);

        Ok(())
    }

    fn stop_capture(&mut self) {
        if let Some(mut capture) = self.capture.take() {
            capture.stop();
        }
    }

    fn start_output(&mut self) -> Result<(), PlatformError> {
        if self.output.is_some() {
            self.stop_output();
        }

        let output = AudioOutput::new()?;
        self.output = Some(output);

        Ok(())
    }

    fn stop_output(&mut self) {
        if let Some(mut output) = self.output.take() {
            output.stop();
        }
        self.state.running.store(false, Ordering::SeqCst);
    }

    fn stop_all(&mut self) {
        self.stop_capture();
        self.stop_output();
    }

    fn is_streaming(&self) -> bool {
        self.capture.is_some() || self.output.is_some()
    }

    /// Process audio: capture → DSP → output
    ///
    /// This is the hot path - must follow real-time safety rules:
    /// - No allocations
    /// - No blocking
    /// - No syscalls
    fn process_audio(&mut self) {
        // Check for EQ updates (lock-free via atomic)
        let current_counter = self.state.master_eq_update_counter.load(Ordering::Relaxed);
        if current_counter != self.eq_update_counter {
            let gains = self.state.get_master_eq_gains();
            // Set each band gain individually
            for (i, &gain) in gains.iter().enumerate() {
                let _ = self.equalizer.set_band_gain(i, gain);
            }
            self.eq_update_counter = current_counter;
        }

        // Capture audio
        if let Some(ref mut capture) = self.capture {
            // Get captured samples into process_buffer
            let samples_read = capture.read(&mut self.process_buffer);

            if samples_read > 0 {
                let buffer = &mut self.process_buffer[..samples_read];

                // Apply DSP (EQ) unless bypassed
                if !self.state.is_bypassed() {
                    // Process in-place
                    let context = gecko_dsp::ProcessContext {
                        sample_rate: 48000.0,
                        channels: 2,
                        buffer_size: samples_read,
                    };
                    self.equalizer.process(buffer, &context);
                }

                // Apply master volume
                let volume = self.state.get_master_volume();
                if (volume - 1.0).abs() > f32::EPSILON {
                    for sample in buffer.iter_mut() {
                        *sample *= volume;
                    }
                }

                // Calculate peak levels for meters (simple peak detection)
                let mut peak_l = 0.0f32;
                let mut peak_r = 0.0f32;
                for (i, &sample) in buffer.iter().enumerate() {
                    let abs = sample.abs();
                    if i % 2 == 0 {
                        peak_l = peak_l.max(abs);
                    } else {
                        peak_r = peak_r.max(abs);
                    }
                }
                self.state.update_peak_levels(peak_l, peak_r);

                // Write processed audio to ring buffer for output
                if let Some(ref mut producer) = self.ring_producer {
                    for &sample in buffer.iter() {
                        // Best-effort write - drop samples if buffer is full
                        let _ = producer.push(sample);
                    }
                }
            }
        }

        // Output audio
        if let Some(ref mut output) = self.output {
            // Read from ring buffer into output
            if let Some(ref mut consumer) = self.ring_consumer {
                let available = consumer.slots();
                if available > 0 {
                    let to_read = available.min(self.output_buffer.len());
                    // Read samples from ring buffer
                    for i in 0..to_read {
                        if let Ok(sample) = consumer.pop() {
                            self.output_buffer[i] = sample;
                        } else {
                            break;
                        }
                    }
                    // Write to output device
                    output.write(&self.output_buffer[..to_read]);
                }
            }
        }
    }
}

// ============================================================================
// Loopback Capture
// ============================================================================

/// WASAPI loopback capture stream
#[cfg(target_os = "windows")]
struct LoopbackCapture {
    client: windows::Win32::Media::Audio::IAudioClient,
    capture_client: windows::Win32::Media::Audio::IAudioCaptureClient,
    #[allow(dead_code)] // May be used for buffer management optimizations
    buffer_frame_count: u32,
    channels: u16,
    started: bool,
}

#[cfg(target_os = "windows")]
impl LoopbackCapture {
    /// Create system-wide loopback capture
    fn new_system() -> Result<Self, PlatformError> {
        use windows::Win32::Media::Audio::{
            eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
            MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

        tracing::debug!("Creating system-wide loopback capture");

        // Get default render device (we capture from its loopback)
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?
        };

        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? };

        // Activate audio client
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };

        // Get mix format - keep pointer alive during Initialize
        let format_ptr = unsafe { client.GetMixFormat()? };
        let channels = unsafe { (*format_ptr).nChannels };

        tracing::debug!(
            "Loopback format: {} channels, {} Hz, {} bits",
            channels,
            unsafe { (*format_ptr).nSamplesPerSec },
            unsafe { (*format_ptr).wBitsPerSample }
        );

        // Initialize for loopback capture - use default buffer duration
        let result = unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                0, // Default buffer duration
                0, // Must be 0 for shared mode
                format_ptr,
                None,
            )
        };

        // Free format pointer after Initialize
        unsafe { CoTaskMemFree(Some(format_ptr as *mut _)) };

        // Check result
        result?;

        // Get buffer size
        let buffer_frame_count = unsafe { client.GetBufferSize()? };

        // Get capture client
        let capture_client: IAudioCaptureClient = unsafe { client.GetService()? };

        // Start the stream
        unsafe {
            client.Start()?;
        }

        tracing::info!(
            "Loopback capture started: {} channels, {} frames buffer",
            channels,
            buffer_frame_count
        );

        Ok(Self {
            client,
            capture_client,
            buffer_frame_count,
            channels,
            started: true,
        })
    }

    /// Create per-process loopback capture (Windows 10 Build 20348+)
    fn new_process(pid: u32) -> Result<Self, PlatformError> {
        // Check Windows version
        let version = super::version::WindowsVersion::current()?;
        if !version.supports_process_loopback() {
            return Err(PlatformError::FeatureNotAvailable(format!(
                "Per-process loopback requires Windows 10 Build {}+ (current: {})",
                super::version::WindowsVersion::MIN_PROCESS_LOOPBACK_BUILD,
                version.build
            )));
        }

        tracing::debug!("Creating per-process loopback for PID {}", pid);

        // TODO: Implement ActivateAudioInterfaceAsync with AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS
        // For now, fall back to system-wide loopback with a warning
        tracing::warn!(
            "Per-process loopback not yet implemented, using system-wide capture for PID {}",
            pid
        );
        Self::new_system()
    }

    /// Read captured audio into buffer
    ///
    /// Returns number of samples read (not frames).
    /// Real-time safe: no allocations, bounded loop.
    fn read(&mut self, buffer: &mut [f32]) -> usize {
        if !self.started {
            return 0;
        }

        let mut total_samples = 0;
        let max_samples = buffer.len();

        // Get next packet (may be called multiple times per callback)
        loop {
            // GetNextPacketSize returns Result<u32> in windows 0.58
            let packet_length = match unsafe { self.capture_client.GetNextPacketSize() } {
                Ok(len) => len,
                Err(_) => break,
            };

            if packet_length == 0 {
                break;
            }

            let frames_to_read = packet_length.min(
                ((max_samples - total_samples) / self.channels as usize) as u32,
            );

            if frames_to_read == 0 {
                break;
            }

            // GetBuffer takes output parameters in windows 0.58
            let mut data_ptr: *mut u8 = std::ptr::null_mut();
            let mut frames_available: u32 = 0;
            let mut flags: u32 = 0;

            let result = unsafe {
                self.capture_client.GetBuffer(
                    &mut data_ptr,
                    &mut frames_available,
                    &mut flags,
                    None,
                    None,
                )
            };

            if result.is_err() {
                break;
            }

            let samples_count = frames_available as usize * self.channels as usize;
            let samples_to_copy = samples_count.min(max_samples - total_samples);

            if samples_to_copy > 0 {
                // Check if data is silent (AUDCLNT_BUFFERFLAGS_SILENT = 0x2)
                if flags & 0x2 != 0 {
                    // Fill with silence
                    buffer[total_samples..total_samples + samples_to_copy].fill(0.0);
                } else {
                    // Copy audio data
                    // WASAPI provides data as f32 in shared mode with float format
                    unsafe {
                        let src = std::slice::from_raw_parts(data_ptr as *const f32, samples_count);
                        buffer[total_samples..total_samples + samples_to_copy]
                            .copy_from_slice(&src[..samples_to_copy]);
                    }
                }
                total_samples += samples_to_copy;
            }

            // Release buffer
            let _ = unsafe { self.capture_client.ReleaseBuffer(frames_available) };

            // Don't read more than we can hold
            if total_samples >= max_samples {
                break;
            }
        }

        total_samples
    }

    fn stop(&mut self) {
        if self.started {
            unsafe {
                let _ = self.client.Stop();
            }
            self.started = false;
            tracing::debug!("Loopback capture stopped");
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for LoopbackCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

// ============================================================================
// Audio Output
// ============================================================================

/// WASAPI audio output (render) stream
#[cfg(target_os = "windows")]
struct AudioOutput {
    client: windows::Win32::Media::Audio::IAudioClient,
    render_client: windows::Win32::Media::Audio::IAudioRenderClient,
    buffer_frame_count: u32,
    channels: u16,
    started: bool,
}

#[cfg(target_os = "windows")]
impl AudioOutput {
    fn new() -> Result<Self, PlatformError> {
        use windows::Win32::Media::Audio::{
            eConsole, eRender, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator,
            MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

        tracing::debug!("Creating audio output stream");

        // Get default render device
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?
        };

        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole)? };

        // Activate audio client
        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None)? };

        // Get mix format - keep pointer alive during Initialize
        let format_ptr = unsafe { client.GetMixFormat()? };
        let channels = unsafe { (*format_ptr).nChannels };

        tracing::debug!(
            "Output format: {} channels, {} Hz, {} bits",
            channels,
            unsafe { (*format_ptr).nSamplesPerSec },
            unsafe { (*format_ptr).wBitsPerSample }
        );

        // Initialize with default buffer duration (let WASAPI decide)
        // Using 0 for buffer duration lets WASAPI use its default
        let result = unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                0, // No special flags
                0, // Default buffer duration (WASAPI decides)
                0, // Must be 0 for shared mode
                format_ptr,
                None,
            )
        };

        // Free format pointer after Initialize
        unsafe { CoTaskMemFree(Some(format_ptr as *mut _)) };

        // Check Initialize result
        result?;

        // Get buffer size
        let buffer_frame_count = unsafe { client.GetBufferSize()? };

        // Get render client
        let render_client: IAudioRenderClient = unsafe { client.GetService()? };

        // Pre-fill with silence
        unsafe {
            let data_ptr = render_client.GetBuffer(buffer_frame_count)?;
            let sample_count = buffer_frame_count as usize * channels as usize;
            let buffer = std::slice::from_raw_parts_mut(data_ptr as *mut f32, sample_count);
            buffer.fill(0.0);
            render_client.ReleaseBuffer(buffer_frame_count, 0)?;
        }

        // Start
        unsafe {
            client.Start()?;
        }

        tracing::info!(
            "Audio output started: {} channels, {} frames buffer",
            channels,
            buffer_frame_count
        );

        Ok(Self {
            client,
            render_client,
            buffer_frame_count,
            channels,
            started: true,
        })
    }

    /// Write audio samples to output
    ///
    /// Real-time safe: no allocations, bounded operations.
    fn write(&mut self, samples: &[f32]) {
        if !self.started {
            return;
        }

        // Get current padding (how much is already in buffer)
        let padding = match unsafe { self.client.GetCurrentPadding() } {
            Ok(p) => p,
            Err(_) => return,
        };

        let frames_available = self.buffer_frame_count.saturating_sub(padding);
        if frames_available == 0 {
            return;
        }

        let frames_to_write =
            ((samples.len() / self.channels as usize) as u32).min(frames_available);
        if frames_to_write == 0 {
            return;
        }

        // Get buffer
        let data_ptr = match unsafe { self.render_client.GetBuffer(frames_to_write) } {
            Ok(ptr) => ptr,
            Err(_) => return,
        };

        // Copy samples
        let sample_count = frames_to_write as usize * self.channels as usize;
        let samples_to_copy = sample_count.min(samples.len());

        unsafe {
            let dest = std::slice::from_raw_parts_mut(data_ptr as *mut f32, sample_count);
            dest[..samples_to_copy].copy_from_slice(&samples[..samples_to_copy]);
            // Fill remainder with silence if needed
            if samples_to_copy < sample_count {
                dest[samples_to_copy..].fill(0.0);
            }
        }

        // Release buffer
        let _ = unsafe { self.render_client.ReleaseBuffer(frames_to_write, 0) };
    }

    fn stop(&mut self) {
        if self.started {
            unsafe {
                let _ = self.client.Stop();
            }
            self.started = false;
            tracing::debug!("Audio output stopped");
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.stop();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasapi_thread_handle_creation() {
        // Note: This will fail on non-Windows
        #[cfg(target_os = "windows")]
        {
            let handle = WasapiThreadHandle::spawn();
            assert!(handle.is_ok(), "Should spawn WASAPI thread");

            let mut h = handle.unwrap();
            h.shutdown().expect("Should shutdown cleanly");
        }
    }

    #[test]
    fn test_audio_processing_state() {
        let state = AudioProcessingState::new();

        // Test volume
        state.set_master_volume(0.75);
        assert!((state.get_master_volume() - 0.75).abs() < f32::EPSILON);

        // Test bypass
        state.set_bypass(true);
        assert!(state.is_bypassed());

        // Test peak levels
        state.update_peak_levels(0.5, 0.6);
        let peaks = state.get_peak_levels();
        assert!((peaks[0] - 0.5).abs() < f32::EPSILON);
        assert!((peaks[1] - 0.6).abs() < f32::EPSILON);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_loopback_capture_creation() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");

        let capture = LoopbackCapture::new_system();
        if capture.is_ok() {
            let mut cap = capture.unwrap();
            // Read a small buffer
            let mut buffer = vec![0.0f32; 1024];
            let _read = cap.read(&mut buffer);
            cap.stop();
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_audio_output_creation() {
        use super::super::com::ComGuard;

        let _com = ComGuard::new().expect("COM init failed");

        let output = AudioOutput::new();
        if output.is_ok() {
            let mut out = output.unwrap();
            // Write silence
            let buffer = vec![0.0f32; 1024];
            out.write(&buffer);
            out.stop();
        }
    }
}
