# macOS Platform Implementation (Process Tap API)

**Last Updated**: December 2024
**Context**: Read when working on macOS audio support, Process Tap integration, or CoreAudio
**Status**: ✅ FULLY IMPLEMENTED (macOS 14.4+ only)

## ⚠️ CRITICAL: No Microphone Input

**Gecko captures APPLICATION AUDIO, NOT microphone input!**

```
CORRECT:  App Audio (Firefox, Spotify) → Process Tap → Gecko DSP → Speakers
WRONG:    Microphone → Gecko DSP → Speakers  ← CAUSES FEEDBACK LOOP!
```

Never use `host.default_input_device()` - that grabs the microphone.

## Overview

macOS 14.4+ uses Apple's **Process Tap API** (`AudioHardwareCreateProcessTap`) for per-app audio capture. This is a native API that requires no driver installation - the first platform-native per-app capture on macOS.

**Requirements**: macOS 14.4+ (Sonoma 14.4 or later) - **MANDATORY**

Older macOS versions are NOT supported. The Process Tap API is the only supported capture method.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      Main Thread                                  │
│  CoreAudioBackend                                                │
│  ├── process_taps: HashMap<u32, ProcessTapCapture>              │
│  ├── pid_to_name: HashMap<u32, String>                          │
│  ├── processing_state: Arc<AudioProcessingState>                │
│  └── app_eq_gains: HashMap<String, [f32; 10]>                   │
└─────────────────────────────────────────────────────────────────┘
                    │ Tauri commands
                    ▼
┌─────────────────────────────────────────────────────────────────┐
│                 Process Tap Captures                              │
│  Per-App (each app has its own):                                 │
│  ├── ProcessTapCapture                                           │
│  │   ├── tap_id: AudioHardwareTapID                             │
│  │   ├── aggregate_device_id: AudioDeviceID                     │
│  │   ├── io_proc_handle: AudioDeviceIOProcID                    │
│  │   └── ring_buffer: Arc<AudioRingBuffer>                      │
│  │                                                               │
│  │   IO Proc Callback (real-time thread):                       │
│  │   - Reads audio from aggregate device                        │
│  │   - Writes to lock-free ring buffer                          │
│  │   - ZERO allocations (real-time safe)                        │
│  └─────────────────────────────────────────────────────────────│
└─────────────────────────────────────────────────────────────────┘
                    │ rtrb ring buffers
                    ▼
┌─────────────────────────────────────────────────────────────────┐
│                   Audio Output Thread                             │
│  AudioOutputStream (cpal)                                        │
│  ├── AudioMixer - combines all Process Tap sources              │
│  ├── Per-app EQ processing                                       │
│  ├── Master EQ + Soft Clipper                                    │
│  └── Output to speakers                                          │
└─────────────────────────────────────────────────────────────────┘
```

## Audio Flow

```
┌───────────────┐     ┌───────────────────────────────────────────┐
│   Firefox     │────▶│  Process Tap (created per-app)            │
│   (PID 1234)  │     │  ├── AudioHardwareCreateProcessTap        │
│               │     │  ├── CATapDescription (Objective-C)       │
└───────────────┘     │  └── tap_id = unique identifier           │
                      └───────────────────────────────────────────┘
                                        │
                                        ▼
                      ┌───────────────────────────────────────────┐
                      │  Aggregate Device                          │
                      │  - Combines tap with output capability     │
                      │  - IO Proc registered here                 │
                      │  - kAudioAggregateDeviceTapList property   │
                      └───────────────────────────────────────────┘
                                        │
                                        │ IO Proc Callback (real-time)
                                        ▼
                      ┌───────────────────────────────────────────┐
                      │  AudioRingBuffer (lock-free SPSC)         │
                      │  - 2 seconds capacity (48kHz stereo)      │
                      │  - Producer: IO Proc callback             │
                      │  - Consumer: cpal output callback         │
                      └───────────────────────────────────────────┘
                                        │
                                        ▼
                      ┌───────────────────────────────────────────┐
                      │  AudioMixer + DSP                          │
                      │  ├── Mix all ring buffers                  │
                      │  ├── Per-app EQ (before mix)               │
                      │  ├── Master EQ                             │
                      │  └── Soft Clipper                          │
                      └───────────────────────────────────────────┘
                                        │
                                        ▼
                                    Speakers
```

## Implementation Files

| File | Purpose |
|------|---------|
| `macos/mod.rs` | CoreAudioBackend struct, PlatformBackend trait implementation |
| `macos/coreaudio.rs` | Device enumeration, app discovery, volume control |
| `macos/process_tap.rs` | ProcessTapCapture - creates/manages per-app taps |
| `macos/process_tap_ffi.rs` | Raw FFI bindings to CoreAudio Process Tap API |
| `macos/tap_description.rs` | CATapDescription Objective-C class wrapper |
| `macos/audio_output.rs` | AudioMixer, AudioOutputStream (cpal-based) |
| `macos/permissions.rs` | Screen Recording and Microphone permission handling |

## Dependencies

```toml
# Cargo.toml
[target.'cfg(target_os = "macos")'.dependencies]
coreaudio-rs = "0.13"
coreaudio-sys = "0.2"
cpal = "0.16"
objc2 = "0.6"
objc2-foundation = "0.3"
core-foundation = "0.10"
rtrb = "0.3"  # Lock-free ring buffer
```

System requirements:
```bash
# Xcode Command Line Tools
xcode-select --install
```

## Process Tap API Flow

### 1. Create CATapDescription

CATapDescription is an Objective-C class that describes what to tap:

```rust
// Convert PID to AudioObjectID (CRITICAL step!)
let object_id = translate_pid_to_audio_object_id(pid)?;

// Create tap description targeting specific process
let tap_description = TapDescription::with_processes(&[pid])?;

// Enable mute - audio ONLY plays through Gecko (no bypass)
tap_description.set_mute(true);
```

**Key Discovery**: `initStereoMixdownOfProcesses:` requires AudioObjectIDs, NOT PIDs!
Use `kAudioHardwarePropertyTranslatePIDToProcessObject` to convert.

### 2. Create Process Tap

```rust
let mut tap_id: AudioHardwareTapID = 0;
let status = AudioHardwareCreateProcessTap(
    tap_description.as_ptr(),  // CATapDescription as CFTypeRef
    &mut tap_id
);
// tap_id is now valid
```

### 3. Create Aggregate Device

The tap cannot be read directly - must create an aggregate device:

```rust
// Get tap UID from the tap (NOT from CATapDescription!)
let tap_uid = get_tap_uid(tap_id)?;

// Create aggregate device description
let description = create_aggregate_device_description(&tap_uid, name);

let mut device_id: AudioDeviceID = 0;
AudioHardwareCreateAggregateDevice(description, &mut device_id);
```

### 4. Register IO Proc and Start

```rust
// Register callback for audio data
let mut io_proc_id: AudioDeviceIOProcID = std::ptr::null_mut();
AudioDeviceCreateIOProcID(
    aggregate_device_id,
    Some(audio_io_proc),      // Callback function
    context_ptr,               // User data
    &mut io_proc_id
);

// Start receiving audio
AudioDeviceStart(aggregate_device_id, io_proc_id);
```

### 5. IO Proc Callback (Real-Time Thread)

```rust
extern "C" fn audio_io_proc(
    _in_device: AudioDeviceID,
    _in_now: *const AudioTimeStamp,
    in_input_data: *const AudioBufferList,  // Audio data here!
    // ...
    in_client_data: *mut c_void,            // Our context
) -> i32 {
    // CRITICAL: No allocations, no blocking, no syscalls!

    let context = unsafe { &*(in_client_data as *const IOProcContext) };

    // Get audio buffer
    let buffer_list = unsafe { &*in_input_data };
    let buffer = &buffer_list.mBuffers[0];
    let samples = unsafe {
        std::slice::from_raw_parts(
            buffer.mData as *const f32,
            buffer.mDataByteSize as usize / 4
        )
    };

    // Write to ring buffer (lock-free)
    context.ring_buffer.push_slice(samples);

    0  // noErr
}
```

## DSP Integration

Audio processing happens in the cpal output callback:

```rust
// In cpal data callback
fn audio_callback(data: &mut [f32], mixer: &AudioMixer, state: &AudioProcessingState) {
    // 1. Read from all Process Tap ring buffers
    let mut mixed = vec![0.0f32; data.len()];
    for source in mixer.sources.read().iter() {
        let mut app_buffer = vec![0.0f32; data.len()];
        source.ring_buffer.pop_slice(&mut app_buffer);

        // 2. Apply per-app EQ (BEFORE mixing)
        if let Some(eq) = state.get_app_eq(&source.app_name) {
            eq.process_interleaved(&mut app_buffer);
        }

        // 3. Apply per-app volume
        let volume = state.get_app_volume(&source.app_name);
        for s in app_buffer.iter_mut() {
            *s *= volume;
        }

        // 4. Mix into output
        for (out, inp) in mixed.iter_mut().zip(app_buffer.iter()) {
            *out += inp;
        }
    }

    // 5. Master EQ
    state.master_eq.process_interleaved(&mut mixed);

    // 6. Soft clipper
    state.soft_clipper.process(&mut mixed);

    // 7. Copy to output
    data.copy_from_slice(&mixed);
}
```

## Permissions

### Required Permissions

| Permission | TCC Key | Why Required |
|------------|---------|--------------|
| Screen Recording | `kTCCServiceScreenCapture` | Process Tap API is classified as screen capture |
| Microphone | `kTCCServiceMicrophone` | macOS requires this for any audio capture |

### Permission Check/Request

```rust
// Check Screen Recording permission
pub fn has_screen_recording_permission() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

// Request Screen Recording permission (opens System Settings)
pub fn request_screen_recording_permission() -> bool {
    unsafe { CGRequestScreenCaptureAccess() }
    // NOTE: User must RESTART app after granting!
}
```

### Info.plist Entitlements

```xml
<key>NSScreenCaptureUsageDescription</key>
<string>Gecko needs Screen Recording permission to capture application audio for EQ processing.</string>

<key>NSMicrophoneUsageDescription</key>
<string>Gecko needs Microphone permission to process application audio (your voice is never recorded).</string>
```

## Protected Apps (Cannot Be Captured)

Due to Apple's security sandboxing, these apps CANNOT be tapped:

- **Safari** - WebKit process sandboxing
- **FaceTime** - Privacy protection
- **Messages** - Privacy protection
- **System Sounds** - System audio
- Apps with `com.apple.security.device.audio-input` entitlement

These apps show as "Protected" in the UI. They still receive Master EQ when audio passes through the system.

## Thread Safety Model

### Main Thread
- Manages `CoreAudioBackend` state
- Creates/destroys Process Taps via Tauri commands
- Updates EQ settings atomically

### IO Proc Threads (Per-App)
- One real-time thread per Process Tap
- Reads from aggregate device
- Writes to ring buffer (lock-free)
- **MUST NOT allocate, block, or make syscalls**

### cpal Output Thread
- Reads from all ring buffers
- Applies DSP processing
- Outputs to speakers

### Shared State (Lock-Free)

```rust
pub struct AudioProcessingState {
    // Atomics for real-time safety
    eq_band_gains: [AtomicU32; 10],    // Stored as f32 bits
    master_volume_bits: AtomicU32,
    bypassed: AtomicBool,
    running: AtomicBool,

    // Per-app state (RwLock - only UI thread writes)
    app_eq_gains: RwLock<HashMap<String, [f32; 10]>>,
    app_volumes: RwLock<HashMap<String, f32>>,
}
```

## App Discovery

GUI apps are discovered via AppleScript (cached, async refresh):

```rust
// Runs in background thread every 3 seconds
fn refresh_gui_apps_cache() {
    let script = r#"
        tell application "System Events"
            get {unix id, name} of (every application process whose visible is true)
        end tell
    "#;
    // Parse results and update cache
}
```

Audio-active apps are detected via `kAudioHardwarePropertyProcessObjectList`.

## Feature Flag

macOS support is automatically enabled on macOS targets:

```toml
# gecko_platform/Cargo.toml
[target.'cfg(target_os = "macos")'.dependencies]
# macOS-specific dependencies here
```

## Current Status

### Implemented ✅
- Process Tap API integration (macOS 14.4+)
- CATapDescription Objective-C bindings
- Per-app audio capture via IO proc callbacks
- Lock-free ring buffer data transfer
- Aggregate device creation with tap
- **Per-app EQ** (independent EQ per application)
- **Per-app volume** (0-200% individual volume)
- **Per-app bypass** (skip EQ per app)
- **Master EQ** (10-band parametric)
- **Spectrum analyzer** (FFT visualization)
- **Soft clipper** (prevents digital clipping)
- **Level metering** (VU meters in UI)
- Screen Recording permission handling
- Microphone permission handling
- Protected app detection and UI indication
- App discovery with caching
- Settings persistence

### Not Supported
- macOS < 14.4 (Process Tap API required)
- Safari, FaceTime, Messages capture (Apple sandbox)
- HAL plugin approach (removed - Process Tap is simpler)

## Testing

```bash
# Run unit tests
cargo test -p gecko_platform

# Run all workspace tests
cargo test --workspace

# View debug log while running
tail -f ~/gecko-debug.log
```

## Debugging

```bash
# Check macOS version
sw_vers

# View Gecko debug log
tail -f ~/gecko-debug.log

# Check Screen Recording permission
# System Settings → Privacy & Security → Screen Recording

# List audio devices
# Use "Audio MIDI Setup" app

# Enable verbose logging
RUST_LOG=gecko_platform=trace pnpm tauri dev
```

### Debug Log Levels

| Level | Use Case |
|-------|----------|
| `error!` | Failures that prevent operation |
| `warn!` | Important issues (permissions, fallbacks) |
| `info!` | Startup/shutdown, key lifecycle events |
| `debug!` | Operational details (tap created, stream started) |
| `trace!` | Verbose details (STEP logs, cache updates) |

## Common Issues

### "Permission Denied" / "who4" Error
Screen Recording permission not granted.
**Solution**: System Settings → Privacy & Security → Screen Recording → Enable Gecko → **RESTART APP**

### No Apps Appearing in Stream List
1. Apps must be actively playing audio
2. Screen Recording permission required
3. Check debug log for errors

### Audio Glitches
Ring buffer underrun - cpal callback faster than IO proc.
**Solution**: Increase ring buffer size in `AudioRingBuffer::new()`

### Protected Apps
Safari, FaceTime, Messages cannot be captured due to Apple sandboxing.
**Workaround**: Use Master EQ for these apps.

## References

- [Apple Process Tap Documentation](https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps)
- [AudioCap Sample Implementation](https://github.com/insidegui/AudioCap)
- [SoundPusher (aggregate device approach)](https://github.com/q-p/SoundPusher)
- [CoreAudio HAL Tap APIs](https://developer.apple.com/documentation/coreaudio/audio_hardware_tap_services)

## Related Documentation

- [audio-pipeline.md](audio-pipeline.md) - Overall audio flow
- [realtime-rules.md](realtime-rules.md) - Rules for audio callbacks
- [eq-implementation.md](../features/eq-implementation.md) - EQ filter details
- [platform-linux.md](platform-linux.md) - Linux implementation (for comparison)
