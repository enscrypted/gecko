//! macOS 14.4+ Process Tap API
//!
//! The Process Tap API (`AudioHardwareCreateProcessTap`) introduced in macOS 14.4
//! enables per-application audio capture WITHOUT requiring a HAL plugin installation.
//!
//! # How It Works
//!
//! 1. Create a tap description targeting a specific process ID
//! 2. Call `AudioHardwareCreateProcessTap` to create the tap
//! 3. Create an aggregate device that includes the tap
//! 4. Read audio from the aggregate device via IO proc callback
//!
//! # Permissions
//!
//! Requires Screen Recording permission (NSScreenCaptureUsageDescription) on macOS 14.4+.
//! The Process Tap API requires Screen Recording because Apple considers capturing
//! other app's audio as a form of "screen recording" for privacy purposes.
//!
//! # References
//!
//! - Apple docs: https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps
//! - Sample code: https://github.com/insidegui/AudioCap

use crate::error::PlatformError;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use tracing::{debug, info, trace, warn};

// CoreGraphics Screen Capture permission APIs
// These are linked from the CoreGraphics framework
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    /// Check if the app has Screen Recording permission without prompting
    /// Returns true if permission was previously granted
    fn CGPreflightScreenCaptureAccess() -> bool;

    /// Request Screen Recording permission - shows the system dialog if not already granted
    /// Returns true if permission is granted (either already had it or user approved)
    /// Note: On macOS 10.15+, this opens System Preferences if permission wasn't granted
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Check if Screen Recording permission has been granted.
///
/// This checks the current permission status without prompting the user.
/// Use this to determine if you need to call `request_screen_recording_permission()`.
///
/// # Returns
///
/// `true` if Screen Recording permission has been granted, `false` otherwise.
///
/// # Example
///
/// ```ignore
/// if !has_screen_recording_permission() {
///     request_screen_recording_permission();
/// }
/// ```
pub fn has_screen_recording_permission() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// Request Screen Recording permission from the user.
///
/// This will show the system permission dialog if the user hasn't already
/// granted or denied permission. If the user has already denied permission,
/// this will open System Preferences > Privacy & Security > Screen Recording
/// where they can manually enable it.
///
/// # Returns
///
/// `true` if permission is granted (either already had it or user just approved).
/// `false` if permission is denied or the user hasn't granted it yet.
///
/// # Important
///
/// After calling this function, you may need to restart the app for the
/// permission to take effect. The Process Tap API requires Screen Recording
/// permission even though it captures audio, not video.
///
/// # Example
///
/// ```ignore
/// if !request_screen_recording_permission() {
///     println!("Please grant Screen Recording permission and restart the app");
/// }
/// ```
pub fn request_screen_recording_permission() -> bool {
    info!("Requesting Screen Recording permission for Process Tap API");
    let granted = unsafe { CGRequestScreenCaptureAccess() };
    if granted {
        info!("Screen Recording permission granted");
    } else {
        warn!(
            "Screen Recording permission not granted. \
             Process Tap API will not work until permission is granted. \
             Please go to System Settings > Privacy & Security > Screen Recording \
             and enable Gecko, then restart the app."
        );
    }
    granted
}

/// Probe for System Audio Recording permission by attempting to create a test tap.
///
/// On macOS 14.4+, the Process Tap API requires "System Audio Recording Only" permission.
/// This permission is DIFFERENT from Screen Recording - it's prompted automatically when
/// we call `AudioHardwareCreateProcessTap`.
///
/// # Returns
///
/// - `Ok(true)` if permission is already granted
/// - `Ok(false)` if permission was just prompted (user should restart after granting)
/// - `Err` if there was an error checking permission
///
/// # Important
///
/// If this returns `Ok(false)`, the calling code should NOT proceed with tap creation.
/// Granting permission mid-session can crash the app because macOS invalidates running
/// audio devices. The user should restart after granting permission.
pub fn probe_system_audio_permission() -> Result<bool, PlatformError> {
    use super::permissions::request_microphone_permission;

    // First request mic permission (required) - this will prompt if not granted
    if !request_microphone_permission() {
        warn!("Microphone permission denied - cannot probe System Audio Recording");
        return Ok(false);
    }

    info!("Probing System Audio Recording permission...");

    // Try to create a global tap (excludes nothing) - this will trigger permission prompt
    // if not already granted. We use global tap for probing because it doesn't require
    // a specific PID and should always succeed if permission is granted.
    let probe_desc = TapDescription::stereo_global_tap_excluding(&[])
        .ok_or_else(|| PlatformError::Internal("Failed to create probe tap description".into()))?;

    unsafe {
        let mut tap_id: AudioHardwareTapID = 0;
        let status = AudioHardwareCreateProcessTap(probe_desc.as_ptr(), &mut tap_id);

        if status == 0 {
            // Permission already granted - clean up probe tap
            AudioHardwareDestroyProcessTap(tap_id);
            info!("System Audio Recording permission already granted");
            Ok(true)
        } else {
            // Check if it's a permission error
            let is_permission_error = match status as u32 {
                0x77686F34 => true, // 'who4' - not authorized
                0x77686174 => true, // 'what' - often permission related
                _ => false,
            };

            if is_permission_error {
                warn!(
                    "System Audio Recording permission not granted. \
                     Please allow the permission prompt and RESTART Gecko."
                );
                Ok(false)
            } else {
                // Some other error
                Err(PlatformError::Internal(format!(
                    "Failed to probe audio permission: OSStatus {} (0x{:08x})",
                    status, status as u32
                )))
            }
        }
    }
}

// Import FFI bindings
use super::process_tap_ffi::{
    create_aggregate_device_description, get_tap_stream_format, get_tap_uid,
    AudioBufferList, AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID,
    AudioDeviceIOProcID, AudioDeviceStart, AudioDeviceStop,
    AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap,
    AudioHardwareDestroyAggregateDevice, AudioHardwareDestroyProcessTap,
    AudioHardwareTapID, AudioTimeStamp, CFRelease, CFTypeRef,
};
// Import the new TapDescription Objective-C wrapper
use super::tap_description::TapDescription;
// Import permission helpers
use super::permissions::request_microphone_permission;
use coreaudio_sys::AudioDeviceID;

/// Wrapper for AudioDeviceIOProcID to make it Send + Sync.
///
/// # Safety
///
/// The CoreAudio AudioDeviceIOProcID is a handle to an IO proc registration.
/// CoreAudio's device control functions (Start, Stop, DestroyIOProcID) are
/// thread-safe and can be called from any thread. We ensure:
/// - The proc ID is only accessed when valid (between Create and Destroy)
/// - Destroy is only called once (in stop() or Drop)
/// - The context pointer remains valid while the IO proc is registered
#[derive(Debug)]
struct IOProcHandle(AudioDeviceIOProcID);

// Safety: CoreAudio device control functions are thread-safe
unsafe impl Send for IOProcHandle {}
unsafe impl Sync for IOProcHandle {}

impl IOProcHandle {
    fn new(id: AudioDeviceIOProcID) -> Self {
        Self(id)
    }

    fn is_null(&self) -> bool {
        self.0.is_null()
    }

    fn get(&self) -> AudioDeviceIOProcID {
        self.0
    }

    fn clear(&mut self) {
        self.0 = std::ptr::null_mut();
    }
}

/// Check if the Process Tap API is available on this system.
///
/// Returns `true` on macOS 14.4 (Sonoma) or later.
///
/// # Implementation Note
///
/// We check the macOS version at runtime because:
/// - The API exists in headers but fails at runtime on older versions
/// - We want to gracefully fall back to HAL plugin mode
pub fn is_process_tap_available() -> bool {
    let version = macos_version();

    // Process Tap API requires macOS 14.4+
    // macOS 14 = Sonoma
    match version {
        (major, _minor, _) if major > 14 => true,
        (14, minor, _) if minor >= 4 => true,
        _ => false,
    }
}

/// Get the current macOS version as (major, minor, patch).
///
/// Uses `sw_vers -productVersion` which returns strings like "14.4.1".
pub fn macos_version() -> (u32, u32, u32) {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let output = Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok();

        if let Some(output) = output {
            if output.status.success() {
                let version_str = String::from_utf8_lossy(&output.stdout);
                let parts: Vec<&str> = version_str.trim().split('.').collect();

                let major = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

                return (major, minor, patch);
            }
        }

        // Fallback: assume old version
        (10, 15, 0)
    }

    #[cfg(not(target_os = "macos"))]
    {
        // Not on macOS - return version that triggers HAL plugin path
        (0, 0, 0)
    }
}

/// Ring buffer for audio data transfer from IO proc to reader
///
/// Lock-free SPSC (Single Producer Single Consumer) ring buffer.
/// The IO proc callback writes audio data, and the reader reads it.
pub struct AudioRingBuffer {
    /// Buffer storage (using UnsafeCell for interior mutability in IO proc)
    buffer: std::cell::UnsafeCell<Vec<f32>>,
    /// Write position (updated by producer/IO proc)
    write_pos: AtomicUsize,
    /// Read position (updated by consumer)
    read_pos: AtomicUsize,
    /// Buffer capacity in samples
    capacity: usize,
}

// Safety: AudioRingBuffer is designed for single-producer single-consumer use
// The producer (IO proc) only writes, the consumer only reads
unsafe impl Send for AudioRingBuffer {}
unsafe impl Sync for AudioRingBuffer {}

impl AudioRingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: std::cell::UnsafeCell::new(vec![0.0; capacity]),
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            capacity,
        }
    }

    /// Get number of samples available to read
    pub fn available(&self) -> usize {
        let write = self.write_pos.load(Ordering::Acquire);
        let read = self.read_pos.load(Ordering::Acquire);
        if write >= read {
            write - read
        } else {
            self.capacity - read + write
        }
    }

    /// Get free space in buffer
    pub fn free_space(&self) -> usize {
        self.capacity - self.available() - 1 // -1 to distinguish full from empty
    }

    /// Write samples to the buffer (called from IO proc)
    ///
    /// Returns number of samples actually written.
    ///
    /// # Safety
    ///
    /// Must only be called from the single producer (IO proc callback).
    pub unsafe fn write(&self, input: &[f32]) -> usize {
        let free = self.free_space();
        let to_write = input.len().min(free);

        if to_write == 0 {
            return 0;
        }

        let write_pos = self.write_pos.load(Ordering::Acquire);
        let buffer = &mut *self.buffer.get();

        // Rust pattern: Handle wrap-around in ring buffer
        // Note: Index-based loop is intentional here for modulo wrap-around logic
        #[allow(clippy::needless_range_loop)]
        for i in 0..to_write {
            let idx = (write_pos + i) % self.capacity;
            buffer[idx] = input[i];
        }

        // Update write position
        let new_pos = (write_pos + to_write) % self.capacity;
        self.write_pos.store(new_pos, Ordering::Release);

        to_write
    }

    /// Read samples from the buffer
    ///
    /// Returns number of samples actually read.
    pub fn read(&self, output: &mut [f32]) -> usize {
        let available = self.available();
        let to_read = output.len().min(available);

        if to_read == 0 {
            return 0;
        }

        let read_pos = self.read_pos.load(Ordering::Acquire);

        // Rust pattern: Handle wrap-around in ring buffer
        // Safety: We're the only reader, IO proc only writes
        let buffer = unsafe { &*self.buffer.get() };
        // Note: Index-based loop is intentional here for modulo wrap-around logic
        #[allow(clippy::needless_range_loop)]
        for i in 0..to_read {
            let idx = (read_pos + i) % self.capacity;
            output[i] = buffer[idx];
        }

        // Update read position
        let new_pos = (read_pos + to_read) % self.capacity;
        self.read_pos.store(new_pos, Ordering::Release);

        to_read
    }
}

/// Context passed to the IO proc callback
///
/// This struct is passed as client data to the CoreAudio IO proc.
/// It must remain valid for the lifetime of the IO proc.
struct IOProcContext {
    /// Ring buffer to write audio data to
    ring_buffer: Arc<AudioRingBuffer>,
    /// Counter for callback invocations (debugging)
    callback_count: Arc<AtomicUsize>,
    /// Counter for samples received (debugging)
    samples_received: Arc<AtomicUsize>,
    /// Counter for callbacks with mNumberBuffers == 0 (debugging)
    zero_buffers_count: Arc<AtomicUsize>,
    /// Counter for buffers with null mData (debugging)
    null_data_count: Arc<AtomicUsize>,
    /// Counter for buffers with mDataByteSize == 0 (debugging)
    zero_size_count: Arc<AtomicUsize>,
}

/// Process Tap audio capture for a specific application.
///
/// Uses the macOS 14.4+ `AudioHardwareCreateProcessTap` API to capture
/// audio from a specific process without needing a HAL plugin.
///
/// # Example
///
/// ```ignore
/// // Capture audio from Firefox (PID 12345)
/// let mut tap = ProcessTapCapture::new(12345)?;
/// tap.start()?;
///
/// // Read audio samples in your audio callback
/// let mut buffer = vec![0.0f32; 1024];
/// let samples_read = tap.read_samples(&mut buffer);
/// ```
pub struct ProcessTapCapture {
    /// The tap identifier returned by AudioHardwareCreateProcessTap
    tap_id: AudioHardwareTapID,

    /// The aggregate device ID that includes the tap
    aggregate_device_id: AudioDeviceID,

    /// The IO proc handle (wrapped for thread safety)
    io_proc_handle: IOProcHandle,

    /// Target process ID
    target_pid: u32,

    /// Sample rate of the capture
    sample_rate: u32,

    /// Number of channels (typically 2 for stereo)
    channels: u32,

    /// Whether the tap is currently active
    active: Arc<AtomicBool>,

    /// Ring buffer for audio data (shared with IO proc)
    ring_buffer: Arc<AudioRingBuffer>,

    /// IO proc context (must be kept alive)
    io_proc_context: Option<Box<IOProcContext>>,

    /// The tap description (MUST be kept alive - CoreAudio may reference it)
    _tap_description: Option<TapDescription>,

    /// Whether the tap was successfully created
    tap_created: bool,

    /// Whether the aggregate device was created
    aggregate_created: bool,

    /// Counter for callback invocations (debugging)
    callback_count: Arc<AtomicUsize>,

    /// Counter for samples received (debugging)
    samples_received: Arc<AtomicUsize>,

    /// Counter for callbacks with mNumberBuffers == 0 (debugging)
    zero_buffers_count: Arc<AtomicUsize>,

    /// Counter for buffers with null mData (debugging)
    null_data_count: Arc<AtomicUsize>,

    /// Counter for buffers with mDataByteSize == 0 (debugging)
    zero_size_count: Arc<AtomicUsize>,
}

/// The IO proc callback function
///
/// This is called by CoreAudio when audio data is available from the tap.
/// It runs on the audio thread and must be real-time safe (no allocations, no blocking).
extern "C" fn audio_io_proc(
    _in_device: AudioDeviceID,
    _in_now: *const AudioTimeStamp,
    in_input_data: *const AudioBufferList,
    _in_input_time: *const AudioTimeStamp,
    _out_output_data: *mut AudioBufferList,
    _in_output_time: *const AudioTimeStamp,
    in_client_data: *mut std::ffi::c_void,
) -> i32 {
    // Safety: Validate pointers before dereferencing
    // Use atomic counters for debugging instead of printing (real-time safe)
    if in_client_data.is_null() {
        // Can't do anything without context - silently return
        return 0;
    }
    if in_input_data.is_null() {
        // No input data - increment null_data counter and return
        unsafe {
            let context = &*(in_client_data as *const IOProcContext);
            context.null_data_count.fetch_add(1, Ordering::Relaxed);
        }
        return 0;
    }

    unsafe {
        let context = &*(in_client_data as *const IOProcContext);
        let buffer_list = &*in_input_data;

        // Increment callback counter
        context.callback_count.fetch_add(1, Ordering::Relaxed);

        // Process each buffer in the list
        let mut total_samples = 0usize;
        let num_buffers = buffer_list.mNumberBuffers;

        // Track callbacks with no buffers
        if num_buffers == 0 {
            context.zero_buffers_count.fetch_add(1, Ordering::Relaxed);
        }

        for i in 0..num_buffers {
            // IMPORTANT: On 64-bit systems, AudioBufferList has alignment padding!
            //
            // AudioBufferList layout (repr(C) on 64-bit):
            //   offset 0: mNumberBuffers (UInt32, 4 bytes)
            //   offset 4: padding (4 bytes for 8-byte alignment of AudioBuffer)
            //   offset 8: mBuffers[0] (AudioBuffer, 16 bytes each)
            //
            // AudioBuffer layout (repr(C) on 64-bit):
            //   offset 0: mNumberChannels (UInt32, 4 bytes)
            //   offset 4: mDataByteSize (UInt32, 4 bytes)
            //   offset 8: mData (void*, 8 bytes)
            //
            // mBuffers starts at offset 8 (NOT 4) due to alignment!
            let list_ptr = buffer_list as *const _ as *const u8;
            let buffer_ptr = list_ptr.add(8 + (i as usize * 16));

            // Read fields with unaligned reads
            let num_channels = std::ptr::read_unaligned(
                buffer_ptr.add(0) as *const u32
            );
            let data_byte_size = std::ptr::read_unaligned(
                buffer_ptr.add(4) as *const u32
            );
            let data_ptr = std::ptr::read_unaligned(
                buffer_ptr.add(8) as *const *mut std::ffi::c_void
            );

            // Log actual values periodically for debugging (every 100 callbacks)
            let cb_count = context.callback_count.load(Ordering::Relaxed);
            if cb_count == 1 || cb_count == 100 || cb_count == 500 {
                // Use warn! level so it always shows in logs
                tracing::warn!(
                    "IO_PROC BUFFER DEBUG: cb={}, num_buffers={}, channels={}, byte_size={}, data_ptr={:?}",
                    cb_count, num_buffers, num_channels, data_byte_size, data_ptr
                );
            }

            if data_ptr.is_null() {
                // Track null data pointers
                context.null_data_count.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            if data_byte_size == 0 {
                // Track zero-size buffers
                context.zero_size_count.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            // Convert raw bytes to f32 samples
            let sample_count = data_byte_size as usize / std::mem::size_of::<f32>();
            let samples = std::slice::from_raw_parts(
                data_ptr as *const f32,
                sample_count,
            );

            // Write to ring buffer
            context.ring_buffer.write(samples);
            total_samples += sample_count;
        }

        // Track total samples received (for debugging)
        if total_samples > 0 {
            context.samples_received.fetch_add(total_samples, Ordering::Relaxed);
        }
    }

    0 // noErr
}

impl ProcessTapCapture {
    /// Create a new Process Tap capture for the given process ID.
    ///
    /// # Arguments
    ///
    /// * `pid` - The process ID to capture audio from
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Process Tap API is not available (macOS < 14.4)
    /// - The process doesn't exist or isn't producing audio
    /// - Audio capture permission is denied
    pub fn new(pid: u32) -> Result<Self, PlatformError> {
        if !is_process_tap_available() {
            return Err(PlatformError::FeatureNotAvailable(
                "Process Tap API requires macOS 14.4 or later".into(),
            ));
        }

        // Check/Request Microphone permission (Required for Process Tap)
        if !request_microphone_permission() {
            warn!("Microphone permission denied - Process Tap will fail");
            return Err(PlatformError::PermissionDenied(
                "Microphone permission required for audio capture. Please grant access in System Settings > Privacy & Security > Microphone.".into(),
            ));
        }

        // Process Tap API requires Screen Recording permission ("System Audio Recording").
        // Without it, macOS delivers silent buffers even though the API succeeds.
        // We must explicitly request this permission before creating taps.
        if !has_screen_recording_permission() {
            info!("Screen Recording permission not granted - requesting...");
            let granted = request_screen_recording_permission();
            if !granted {
                // CGRequestScreenCaptureAccess opens System Settings but doesn't block.
                // User must manually toggle permission and RESTART the app.
                warn!(
                    "Screen Recording permission required for audio capture. \
                     System Settings has been opened - please enable Gecko under 'Screen Recording' \
                     and RESTART the app."
                );
                return Err(PlatformError::PermissionDenied(
                    "Screen Recording permission required. Please enable in System Settings > Privacy & Security > Screen Recording, then restart the app.".into(),
                ));
            }
        }

        trace!("Creating Process Tap for PID {}", pid);
        trace!("STEP 1: Creating ring buffer...");

        // Create ring buffer for audio data (2 seconds at 48kHz stereo)
        let buffer_size = 48000 * 2 * 2; // 2 seconds * 48kHz * 2 channels
        let ring_buffer = Arc::new(AudioRingBuffer::new(buffer_size));
        trace!("STEP 1: Ring buffer created OK");

        // Create the tap using CATapDescription Objective-C class
        // This is the official way to create tap descriptions on macOS 14.4+
        // Manual CFDictionary creation doesn't work - must use the class
        trace!("STEP 2: Creating CATapDescription...");
        let tap_description = TapDescription::with_processes(&[pid as i32])
            .ok_or_else(|| {
                PlatformError::Internal(
                    "Failed to create CATapDescription - is CATapDescription class available?".into(),
                )
            })?;
        trace!("STEP 2: CATapDescription created OK, UUID: {}", tap_description.uuid());

        // CRITICAL: Mute the original audio so it only goes through Gecko
        // Without this, audio plays BOTH through the tap AND directly to speakers,
        // making per-app volume control impossible (audio leaks around our processing)
        tap_description.set_mute(true);
        trace!("STEP 2b: Mute enabled - audio will only play through Gecko");

        // Create the tap using the CATapDescription
        // CATapDescription is an NSObject, not a CFDictionary, but CoreAudio accepts it
        // as a CFTypeRef - this is the official macOS 14.4+ API
        trace!("STEP 3: Calling AudioHardwareCreateProcessTap...");
        let tap_id = unsafe {
            let mut tap_id: AudioHardwareTapID = 0;
            let description_ptr = tap_description.as_ptr();
            trace!("STEP 3: CATapDescription ptr: {:?}", description_ptr);
            let status = AudioHardwareCreateProcessTap(description_ptr, &mut tap_id);
            trace!("STEP 3: AudioHardwareCreateProcessTap returned status: {}", status);

            // Note: Don't release the description - TapDescription owns it via Retained

            if status != 0 {
                // Common errors:
                // -50 (paramErr): Invalid parameters or process not found
                // -54 (permErr): Permission denied (need NSAudioCaptureUsageDescription)
                // -10863: Process not producing audio
                // 2003329396 ("who4"): Not authorized - check Screen Recording permission
                warn!(
                    "AudioHardwareCreateProcessTap failed: OSStatus {} (0x{:08x})",
                    status, status as u32
                );

                // Decode common error codes for better debugging
                let error_hint = match status as u32 {
                    0x77686F34 => " (who4: not authorized - check Microphone & Screen Recording permissions)",
                    0x77686174 => " (what: unspecified error - often permission related)",
                    0x21707270 => " (!prp: bad property)",
                    0x776F6E3F => " (won?: property not found)",
                    0x216F626A => " (!obj: bad object - CATapDescription may be invalid)",
                    0xFFFFFFCE => " (-50: paramErr - invalid parameters or process not playing audio)",
                    _ => "",
                };

                // DIAGNOSTIC FALLBACK: Try Global Tap if we get auth-related errors
                // If this works, permissions are fine, but per-process targeting is broken.
                // 'who4' = 0x77686F34 (not authorized)
                // 'what' = 0x77686174 (unspecified, often permission-related)
                let is_auth_error = status as u32 == 0x77686F34 || status as u32 == 0x77686174;
                if is_auth_error {
                    trace!("DIAGNOSTIC: Attempting fallback to Global Tap to verify permissions...");
                    let global_desc = TapDescription::stereo_global_tap_excluding(&[])
                        .ok_or_else(|| PlatformError::Internal("Failed to create Global Tap description".into()))?;

                    let global_ptr = global_desc.as_ptr();
                    let mut global_tap_id: AudioHardwareTapID = 0;
                    let global_status = AudioHardwareCreateProcessTap(global_ptr, &mut global_tap_id);

                    if global_status == 0 {
                        // Global tap works - permissions are valid, issue is per-process targeting
                        trace!("DIAGNOSTIC: Global Tap SUCCESS! Permissions are valid. The issue is with per-process targeting (NSRunningApplication/CATapDescription).");
                        // Clean up the test tap - we do NOT want to use global tap
                        // (per-app EQ is the core product differentiator)
                        AudioHardwareDestroyProcessTap(global_tap_id);

                        // Still return an error - we need per-process tap, not global
                        return Err(PlatformError::Internal(format!(
                            "Per-process tap failed with {} but global tap works. \
                             This indicates the issue is with per-process targeting, not permissions. \
                             Check CATapDescription configuration or NSRunningApplication handling.",
                            error_hint
                        )));
                    } else {
                        trace!("DIAGNOSTIC: Global Tap also FAILED: OSStatus {} (0x{:08x})", global_status, global_status as u32);
                        return Err(PlatformError::Internal(format!(
                            "Failed to create process tap: OSStatus {}{} (PID {}). \
                             Both per-process and global taps failed - likely a permission issue. \
                             Check Screen Recording and Microphone permissions.",
                            status, error_hint, pid
                        )));
                    }
                } else {
                    return Err(PlatformError::Internal(format!(
                        "Failed to create process tap: OSStatus {}{} (PID {}). \
                         Ensure app has audio capture permission and process is producing audio.",
                        status, error_hint, pid
                    )));
                }
            } else {
                debug!("Created Process Tap with ID {} for PID {}", tap_id, pid);
            }
            tap_id
        };

        // CRITICAL: Read the tap UID from the tap using kAudioTapPropertyUID
        // According to SoundPusher, we should read the UID from the tap rather than
        // setting our own UUID on CATapDescription
        trace!("STEP 4: Reading tap UID from tap...");
        let tap_uid = unsafe { get_tap_uid(tap_id) }.ok_or_else(|| {
            unsafe { AudioHardwareDestroyProcessTap(tap_id) };
            PlatformError::Internal(
                "Failed to read tap UID from tap. The tap may not be properly configured.".into(),
            )
        })?;
        trace!("STEP 4: Tap UID from system: {}", tap_uid);

        // Query the tap's audio format (informational - may fail but not critical)
        let tap_format = unsafe { get_tap_stream_format(tap_id) };
        if let Some(ref format) = tap_format {
            trace!(
                "Tap format: {:.0}Hz, {} channels, {} bits/channel, {} bytes/frame",
                format.mSampleRate,
                format.mChannelsPerFrame,
                format.mBitsPerChannel,
                format.mBytesPerFrame
            );
        } else {
            // This may fail with 'who?' error but is not critical - continue anyway
            debug!("Could not query tap format - continuing with aggregate device creation");
        }

        // NOTE: SoundPusher approach - aggregate device does NOT include the output device!
        // The aggregate device ONLY contains the tap.

        // Create aggregate device that ONLY includes the tap (SoundPusher style)
        trace!("STEP 5: Creating aggregate device...");
        let aggregate_device_id = unsafe {
            trace!("STEP 5a: Creating aggregate device description...");
            let description = create_aggregate_device_description(
                &tap_uid,
                &format!("Gecko Tap (PID {})", pid),
            );
            trace!("STEP 5a: Aggregate device description created");

            if description.is_null() {
                // Clean up tap
                AudioHardwareDestroyProcessTap(tap_id);
                return Err(PlatformError::Internal(
                    "Failed to create aggregate device description".into(),
                ));
            }

            let mut device_id: AudioDeviceID = 0;
            trace!("STEP 5b: Calling AudioHardwareCreateAggregateDevice...");
            let status = AudioHardwareCreateAggregateDevice(description, &mut device_id);
            trace!("STEP 5b: AudioHardwareCreateAggregateDevice returned status: {}, device_id: {}", status, device_id);

            CFRelease(description as CFTypeRef);

            if status != 0 {
                // Clean up tap
                AudioHardwareDestroyProcessTap(tap_id);
                warn!(
                    "AudioHardwareCreateAggregateDevice failed: OSStatus {}",
                    status
                );
                return Err(PlatformError::Internal(format!(
                    "Failed to create aggregate device: OSStatus {}",
                    status
                )));
            }

            trace!("STEP 5c: Created aggregate device with ID {}", device_id);

            // Log stream info to diagnose if input streams are configured
            super::process_tap_ffi::log_device_stream_info(
                device_id,
                &format!("Gecko Tap (PID {})", pid),
            );

            device_id
        };

        debug!("ProcessTapCapture created for PID {}", pid);
        Ok(Self {
            tap_id,
            aggregate_device_id,
            io_proc_handle: IOProcHandle::new(std::ptr::null_mut()),
            target_pid: pid,
            sample_rate: 48000,
            channels: 2,
            active: Arc::new(AtomicBool::new(false)),
            ring_buffer,
            io_proc_context: None,
            _tap_description: Some(tap_description), // Keep alive - CoreAudio may reference
            tap_created: true,
            aggregate_created: true,
            callback_count: Arc::new(AtomicUsize::new(0)),
            samples_received: Arc::new(AtomicUsize::new(0)),
            zero_buffers_count: Arc::new(AtomicUsize::new(0)),
            null_data_count: Arc::new(AtomicUsize::new(0)),
            zero_size_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Get the tap identifier.
    pub fn tap_id(&self) -> u32 {
        self.tap_id
    }

    /// Get the target process ID.
    pub fn target_pid(&self) -> u32 {
        self.target_pid
    }

    /// Get the sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Get the number of channels.
    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// Check if the tap is active.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Start capturing audio using IO proc callback.
    ///
    /// This is the same approach AudioCap uses - create an IO proc on the
    /// aggregate device and receive audio in the input buffer.
    pub fn start(&mut self) -> Result<(), PlatformError> {
        if self.active.load(Ordering::Acquire) {
            return Ok(()); // Already running
        }

        debug!("Starting Process Tap capture for PID {} using IO proc", self.target_pid);

        // Create context for IO proc callback
        // Using Box::leak to create a 'static lifetime pointer - this is intentional
        // The context will be cleaned up when stop() is called
        let context = Box::new(IOProcContext {
            ring_buffer: Arc::clone(&self.ring_buffer),
            callback_count: Arc::clone(&self.callback_count),
            samples_received: Arc::clone(&self.samples_received),
            zero_buffers_count: Arc::clone(&self.zero_buffers_count),
            null_data_count: Arc::clone(&self.null_data_count),
            zero_size_count: Arc::clone(&self.zero_size_count),
        });
        let context_ptr = Box::into_raw(context);

        // Create IO proc for the aggregate device
        let io_proc_id = unsafe {
            let mut proc_id: AudioDeviceIOProcID = std::ptr::null_mut();
            let status = AudioDeviceCreateIOProcID(
                self.aggregate_device_id,
                audio_io_proc,
                context_ptr as *mut std::ffi::c_void,
                &mut proc_id,
            );

            if status != 0 {
                // Clean up context on failure
                let _ = Box::from_raw(context_ptr);
                warn!("AudioDeviceCreateIOProcID failed: OSStatus {}", status);
                return Err(PlatformError::Internal(format!(
                    "Failed to create IO proc: OSStatus {}",
                    status
                )));
            }

            debug!(
                "Created IO proc for aggregate device {} (PID {})",
                self.aggregate_device_id, self.target_pid
            );
            proc_id
        };

        // Start the device
        let status = unsafe { AudioDeviceStart(self.aggregate_device_id, io_proc_id) };
        if status != 0 {
            // Clean up on failure
            unsafe {
                AudioDeviceDestroyIOProcID(self.aggregate_device_id, io_proc_id);
                let _ = Box::from_raw(context_ptr);
            }
            warn!("AudioDeviceStart failed: OSStatus {}", status);
            return Err(PlatformError::Internal(format!(
                "Failed to start audio device: OSStatus {}",
                status
            )));
        }

        debug!(
            "Started audio capture for PID {} on aggregate device {}",
            self.target_pid, self.aggregate_device_id
        );

        // Store handles
        self.io_proc_handle = IOProcHandle::new(io_proc_id);
        // Store context so it can be cleaned up later
        self.io_proc_context = Some(unsafe { Box::from_raw(context_ptr) });
        self.active.store(true, Ordering::Release);

        Ok(())
    }

    /// Stop capturing audio.
    pub fn stop(&mut self) -> Result<(), PlatformError> {
        if !self.active.load(Ordering::Acquire) {
            return Ok(()); // Already stopped
        }

        debug!("Stopping Process Tap capture for PID {}", self.target_pid);

        // Stop IO proc if active
        if !self.io_proc_handle.is_null() {
            unsafe {
                let proc_id = self.io_proc_handle.get();
                let status = AudioDeviceStop(self.aggregate_device_id, proc_id);
                if status != 0 {
                    warn!("AudioDeviceStop failed: OSStatus {}", status);
                }

                let status = AudioDeviceDestroyIOProcID(self.aggregate_device_id, proc_id);
                if status != 0 {
                    warn!("AudioDeviceDestroyIOProcID failed: OSStatus {}", status);
                } else {
                    debug!("Destroyed IO proc for PID {}", self.target_pid);
                }
            }
            self.io_proc_handle.clear();
        }

        // Drop the context (releases the Arc reference to ring buffer)
        self.io_proc_context = None;

        self.active.store(false, Ordering::Release);
        debug!("Stopped Process Tap capture for PID {}", self.target_pid);
        Ok(())
    }

    /// Read audio samples from the tap.
    ///
    /// Returns the number of samples actually read (may be less than buffer size).
    /// Audio is interleaved stereo float samples.
    ///
    /// # Arguments
    ///
    /// * `buffer` - Buffer to fill with interleaved float samples
    pub fn read_samples(&self, buffer: &mut [f32]) -> usize {
        if !self.active.load(Ordering::Acquire) {
            buffer.fill(0.0);
            return 0;
        }

        self.ring_buffer.read(buffer)
    }

    /// Get number of samples available to read
    pub fn available_samples(&self) -> usize {
        self.ring_buffer.available()
    }

    /// Get a clone of the ring buffer Arc
    ///
    /// This allows sharing the ring buffer with other components (like the audio mixer)
    /// for reading audio data. The caller can read directly from the ring buffer
    /// without going through ProcessTapCapture.
    pub fn ring_buffer(&self) -> Arc<AudioRingBuffer> {
        Arc::clone(&self.ring_buffer)
    }

    /// Get the number of times the IO proc callback has been invoked
    ///
    /// Used for debugging to verify audio is flowing from the tap.
    pub fn callback_count(&self) -> usize {
        self.callback_count.load(Ordering::Relaxed)
    }

    /// Get the total number of samples received
    ///
    /// Used for debugging to verify audio is flowing from the tap.
    pub fn samples_received(&self) -> usize {
        self.samples_received.load(Ordering::Relaxed)
    }

    /// Get debug stats as a formatted string
    pub fn debug_stats(&self) -> String {
        format!(
            "PID {}: callbacks={}, samples={}, ring_available={}, zero_buffers={}, null_data={}, zero_size={}",
            self.target_pid,
            self.callback_count(),
            self.samples_received(),
            self.ring_buffer.available(),
            self.zero_buffers_count.load(Ordering::Relaxed),
            self.null_data_count.load(Ordering::Relaxed),
            self.zero_size_count.load(Ordering::Relaxed),
        )
    }
}

impl Drop for ProcessTapCapture {
    fn drop(&mut self) {
        // Stop if running (this cleans up IO proc)
        if self.active.load(Ordering::Acquire) {
            let _ = self.stop();
        }

        // Destroy aggregate device first (before destroying tap)
        // Rust pattern: Clean up in reverse order of creation
        if self.aggregate_created && self.aggregate_device_id != 0 {
            unsafe {
                let status = AudioHardwareDestroyAggregateDevice(self.aggregate_device_id);
                if status != 0 {
                    warn!(
                        "AudioHardwareDestroyAggregateDevice failed: OSStatus {}",
                        status
                    );
                } else {
                    debug!(
                        "Destroyed aggregate device {} for PID {}",
                        self.aggregate_device_id, self.target_pid
                    );
                }
            }
        }

        // Destroy the tap
        if self.tap_created && self.tap_id != 0 {
            unsafe {
                let status = AudioHardwareDestroyProcessTap(self.tap_id);
                if status != 0 {
                    warn!(
                        "AudioHardwareDestroyProcessTap failed: OSStatus {}",
                        status
                    );
                } else {
                    debug!(
                        "Destroyed Process Tap {} for PID {}",
                        self.tap_id, self.target_pid
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macos_version_parsing() {
        let (major, minor, patch) = macos_version();
        println!("Detected macOS version: {}.{}.{}", major, minor, patch);

        #[cfg(target_os = "macos")]
        {
            assert!(major >= 10, "Major version should be at least 10");
        }
    }

    #[test]
    fn test_process_tap_availability() {
        let available = is_process_tap_available();
        let version = macos_version();

        println!(
            "macOS {}.{}.{}: Process Tap available = {}",
            version.0, version.1, version.2, available
        );

        // Verify the logic is consistent
        if version.0 > 14 || (version.0 == 14 && version.1 >= 4) {
            assert!(available, "Should be available on macOS 14.4+");
        } else {
            assert!(!available, "Should not be available on macOS < 14.4");
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_process_tap_creation_on_old_macos() {
        // On < 14.4, creation should fail gracefully
        if !is_process_tap_available() {
            let result = ProcessTapCapture::new(1);
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_ring_buffer() {
        let buffer = AudioRingBuffer::new(1024);

        // Initially empty
        assert_eq!(buffer.available(), 0);

        // Read from empty buffer returns 0
        let mut out = [0.0f32; 10];
        let read = buffer.read(&mut out);
        assert_eq!(read, 0);
    }
}
