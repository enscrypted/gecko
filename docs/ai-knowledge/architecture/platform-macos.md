# macOS Platform Implementation (CoreAudio HAL)

**Last Updated**: December 2024
**Context**: Read when working on macOS audio support, HAL plugins, or virtual devices

## Overview

macOS uses CoreAudio with HAL (Hardware Abstraction Layer) plugins for virtual devices. This is the **most complex** platform - requires a separate driver bundle.

## Architecture Challenge

### The Problem
- Virtual devices require an **AudioServerPlugIn** (HAL plugin)
- Plugin runs inside `coreaudiod` (system daemon) - separate process
- Plugin and Gecko app **cannot share memory directly**
- Need Inter-Process Communication (IPC)

### The Solution
**Shared Memory** ring buffer for data transfer between:
1. HAL Plugin (writes audio from OS)
2. Gecko App (reads audio for processing)

```
┌─────────────────────┐         ┌─────────────────────┐
│   HAL Plugin        │         │   Gecko App         │
│   (in coreaudiod)   │         │                     │
│                     │  shm    │                     │
│   IOProc ──────────▶│◀───────▶│ ──────▶ DSP        │
│                     │         │                     │
└─────────────────────┘         └─────────────────────┘
        Shared Memory Ring Buffer
```

## HAL Plugin Structure

### Bundle Layout
```
GeckoAudioDevice.driver/
├── Contents/
│   ├── Info.plist
│   ├── MacOS/
│   │   └── GeckoAudioDevice    # Binary
│   └── Resources/
```

### Location
```
/Library/Audio/Plug-Ins/HAL/GeckoAudioDevice.driver
```

### Properties
- Bundle ID: `com.gecko.driver.AudioDevice`
- Device Name: "Gecko Sink"
- Ownership: `root:wheel` (required by macOS)

## Shared Memory Protocol

### Header Structure
```c
struct GeckoShmHeader {
    atomic_uint32_t write_head;   // Updated by driver
    atomic_uint32_t read_head;    // Updated by app
    uint32_t buffer_size;         // In samples
    uint32_t channels;            // 2 for stereo
    uint32_t sample_rate;         // 48000 typically
};
// Followed by raw float PCM data
```

### Synchronization
- **Lock-Free**: No mutexes across process boundary (could deadlock coreaudiod)
- **Atomic Operations**: C++11/Rust atomics for head pointers
- **SPSC Pattern**: Single producer (driver), single consumer (app)

## Gecko App Side

### Connection

```rust
// Open shared memory
let fd = shm_open("/GeckoAudioShm", O_RDONLY, 0)?;

// Map into address space
let ptr = mmap(
    null_mut(),
    size,
    PROT_READ,
    MAP_SHARED,
    fd,
    0,
)?;

// Wrap in safe Rust struct
let audio_source = SharedMemorySource::new(ptr);
```

### Reading Audio

```rust
impl Iterator for SharedMemorySource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let write = self.header.write_head.load(Ordering::Acquire);
        if self.read_pos == write {
            return None;  // No new data
        }

        let sample = unsafe { *self.data.add(self.read_pos) };
        self.read_pos = (self.read_pos + 1) % self.buffer_size;
        self.header.read_head.store(self.read_pos, Ordering::Release);

        Some(sample)
    }
}
```

## Installation

### Requirements
- **Elevated Privileges**: Plugin installation requires root
- **Daemon Restart**: coreaudiod must be restarted

### Install Script
```bash
# Copy bundle
sudo cp -R GeckoAudioDevice.driver /Library/Audio/Plug-Ins/HAL/

# Set ownership (REQUIRED - macOS enforces this)
sudo chown -R root:wheel /Library/Audio/Plug-Ins/HAL/GeckoAudioDevice.driver

# Restart audio daemon
sudo launchctl kickstart -k system/com.apple.audio.coreaudiod
```

### Uninstall Script
```bash
sudo rm -rf /Library/Audio/Plug-Ins/HAL/GeckoAudioDevice.driver
sudo launchctl kickstart -k system/com.apple.audio.coreaudiod
```

## Platform Capabilities

```rust
pub fn supports_virtual_devices() -> bool {
    false  // Requires HAL plugin installation
}

pub fn supports_per_app_capture() -> bool {
    false  // No native macOS support
}
```

## Implementation Strategy

### Recommended Approach

1. **Fork existing driver**: BlackHole or libASPL (well-tested)
2. **Keep driver in C/C++**: Match Apple SDK examples
3. **Rust wrapper for IPC**: Safe interface to shared memory
4. **Tauri installer**: Handle privileged installation

### Don't
- Write HAL plugin from scratch (complex COM-style interfaces)
- Try to avoid the driver (there's no way around it on macOS)
- Skip the coreaudiod restart (device won't appear)

## Error Handling

```rust
pub enum PlatformError {
    #[error("HAL plugin not installed")]
    PluginNotInstalled,

    #[error("Failed to open shared memory: {0}")]
    ShmOpenFailed(String),

    #[error("Failed to map shared memory: {0}")]
    MmapFailed(String),

    #[error("Audio daemon restart required")]
    DaemonRestartRequired,
}
```

## Related Files

- `crates/gecko_platform/src/macos/` - CoreAudio implementation
- `crates/gecko_platform/src/lib.rs` - Platform trait and detection
- `docs/03_MacOS_Strategy.md` - Original strategy document
