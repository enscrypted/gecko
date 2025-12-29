# macOS Implementation Strategy: Process Tap API

**Status**: ✅ IMPLEMENTED (December 2024)
**Requirement**: macOS 14.4+ (Sonoma 14.4 or later)

## Overview

macOS uses Apple's **Process Tap API** introduced in macOS 14.4 for per-application audio capture. This native API enables per-app EQ without requiring driver installation or kernel extensions.

> **Note**: This document supersedes the original HAL plugin strategy. The Process Tap API provides a simpler, more reliable solution without the complexity of custom audio drivers.

## 1. The Process Tap API

### 1.1 Architecture

- **Type**: Native CoreAudio API (macOS 14.4+)
- **No Installation Required**: Works out of the box
- **Per-Process Capture**: Each app can be tapped individually
- **Permissions**: Requires Screen Recording permission

### 1.2 How It Works

```
┌─────────────────┐
│ Application     │  (e.g., Firefox, Spotify)
│ Audio Output    │
└────────┬────────┘
         │ Process Tap API
         │ AudioHardwareCreateProcessTap()
         ▼
┌─────────────────┐
│ CATapDescription│  Objective-C class describing tap target
│ - Target PIDs   │  - Uses AudioObjectID (not raw PID)
│ - Mute option   │  - Can mute original audio path
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Aggregate Device│  Virtual device combining tap
│ - Input from tap│  - Allows IO proc registration
│ - Output capable│
└────────┬────────┘
         │ IO Proc Callback (real-time)
         ▼
┌─────────────────┐
│ Gecko DSP       │  EQ, Volume, Soft Clipper
│ Processing      │
└────────┬────────┘
         │
         ▼
    Speakers (via cpal)
```

### 1.3 Key Functionality

The Process Tap API provides:
1. **Per-process audio capture** - Target specific PIDs
2. **Mute capability** - Route audio exclusively through Gecko
3. **No driver needed** - Native macOS API
4. **Low latency** - Direct hardware path

## 2. Implementation Details

### 2.1 Creating a Process Tap

```rust
// 1. Convert PID to AudioObjectID
let object_id = translate_pid_to_audio_object_id(pid)?;

// 2. Create CATapDescription (Objective-C)
let tap_description = TapDescription::with_processes(&[pid])?;
tap_description.set_mute(true);  // Audio only through Gecko

// 3. Create the tap
let mut tap_id: AudioHardwareTapID = 0;
AudioHardwareCreateProcessTap(tap_description.as_ptr(), &mut tap_id);

// 4. Get tap UID for aggregate device
let tap_uid = get_tap_uid(tap_id)?;

// 5. Create aggregate device
let description = create_aggregate_device_description(&tap_uid, name);
AudioHardwareCreateAggregateDevice(description, &mut device_id);

// 6. Register IO proc for audio data
AudioDeviceCreateIOProcID(device_id, Some(audio_io_proc), context, &mut io_proc_id);
AudioDeviceStart(device_id, io_proc_id);
```

### 2.2 Ring Buffer Data Transfer

Audio flows from IO proc to output via lock-free ring buffer:

```rust
// IO Proc callback (real-time thread)
extern "C" fn audio_io_proc(..., in_input_data: *const AudioBufferList, ...) -> i32 {
    // CRITICAL: No allocations, no blocking!
    let samples = get_samples_from_buffer_list(in_input_data);
    context.ring_buffer.push_slice(samples);  // Lock-free
    0
}

// cpal output callback (audio thread)
fn output_callback(data: &mut [f32], ring_buffer: &AudioRingBuffer) {
    ring_buffer.pop_slice(data);  // Lock-free
    // Apply DSP processing...
}
```

### 2.3 Critical Discovery: AudioObjectID vs PID

The `initStereoMixdownOfProcesses:` method requires **AudioObjectIDs**, not raw PIDs:

```rust
// WRONG: Using raw PID
let tap = TapDescription::with_processes(&[pid])?;  // Fails!

// CORRECT: Convert PID to AudioObjectID first
let object_id = translate_pid_to_audio_object_id(pid)?;
// Then pass AudioObjectID (wrapped in NSNumber) to the method
```

Use `kAudioHardwarePropertyTranslatePIDToProcessObject` for conversion.

## 3. Permissions

### 3.1 Required Permissions

| Permission | Purpose | User Action |
|------------|---------|-------------|
| Screen Recording | Process Tap API is classified as screen capture | System Settings → Privacy → Screen Recording |
| Microphone | macOS requires this for audio capture APIs | Grant when prompted |

### 3.2 Info.plist Keys

```xml
<key>NSScreenCaptureUsageDescription</key>
<string>Gecko needs Screen Recording permission to capture application audio.</string>

<key>NSMicrophoneUsageDescription</key>
<string>Gecko needs Microphone permission to process application audio.</string>
```

### 3.3 Permission Flow

1. App checks `CGPreflightScreenCaptureAccess()`
2. If not granted, calls `CGRequestScreenCaptureAccess()` → Opens System Settings
3. User enables Gecko in Screen Recording list
4. **User MUST restart app** (macOS requirement)

## 4. Limitations

### 4.1 Protected Apps (Cannot Be Captured)

Due to Apple's security sandbox, these apps cannot be tapped:
- **Safari** - WebKit sandboxing
- **FaceTime** - Privacy protection
- **Messages** - Privacy protection
- **System Sounds** - System audio

**Workaround**: These apps receive Master EQ when audio passes through system output.

### 4.2 macOS Version Requirement

- **macOS 14.4+**: Full per-app capture via Process Tap
- **macOS < 14.4**: **NOT SUPPORTED** - Gecko requires 14.4+

## 5. Comparison with Original HAL Plugin Approach

| Aspect | HAL Plugin (Original Plan) | Process Tap (Implemented) |
|--------|----------------------------|---------------------------|
| Installation | Required driver installation | None needed |
| Permissions | Admin password | Screen Recording only |
| Complexity | High (C++ plugin, IPC, shared memory) | Low (native Rust FFI) |
| Per-app capture | No (required manual routing) | Yes (native API) |
| macOS version | 10.15+ | 14.4+ only |
| Maintenance | Must update for macOS changes | Apple-supported API |

The Process Tap API provides a significantly better solution despite the higher macOS version requirement.

## 6. Implementation Files

| File | Purpose |
|------|---------|
| `crates/gecko_platform/src/macos/mod.rs` | CoreAudioBackend |
| `crates/gecko_platform/src/macos/process_tap.rs` | ProcessTapCapture |
| `crates/gecko_platform/src/macos/process_tap_ffi.rs` | Raw FFI bindings |
| `crates/gecko_platform/src/macos/tap_description.rs` | CATapDescription wrapper |
| `crates/gecko_platform/src/macos/audio_output.rs` | AudioMixer + cpal output |
| `crates/gecko_platform/src/macos/coreaudio.rs` | Device/app enumeration |
| `crates/gecko_platform/src/macos/permissions.rs` | Permission handling |

## 7. References

- [Apple Process Tap Documentation](https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps)
- [AudioCap Sample Implementation](https://github.com/insidegui/AudioCap)
- [SoundPusher (aggregate device approach)](https://github.com/q-p/SoundPusher)
- [CoreAudio HAL Tap APIs](https://developer.apple.com/documentation/coreaudio/audio_hardware_tap_services)

## 8. Implementation Checklist

- [x] Process Tap API integration
- [x] CATapDescription Objective-C bindings
- [x] AudioObjectID conversion (PID → AudioObjectID)
- [x] Aggregate device creation
- [x] IO proc callback registration
- [x] Lock-free ring buffer data transfer
- [x] Per-app EQ processing
- [x] Per-app volume control
- [x] Master EQ
- [x] Soft clipper
- [x] Screen Recording permission handling
- [x] Protected app detection and UI
- [x] App discovery with async caching
- [x] Settings persistence
