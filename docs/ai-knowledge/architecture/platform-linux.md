# Linux Platform Implementation (PipeWire)

**Last Updated**: December 2024
**Context**: Read when working on Linux audio support, PipeWire integration, or virtual devices

## ⚠️ CRITICAL: No Microphone Input

**Gecko captures APPLICATION AUDIO, NOT microphone input!**

```
CORRECT:  App Audio (Firefox, Spotify) → Virtual Sink → Gecko DSP → Speakers
WRONG:    Microphone → Gecko DSP → Speakers  ← CAUSES FEEDBACK LOOP!
```

Never use `host.default_input_device()` - that grabs the microphone.

## Overview

Linux uses PipeWire for audio routing. This is the **most flexible** platform for Gecko - virtual devices can be created at runtime without kernel modules.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                   Main Thread                            │
│  PipeWireBackend                                        │
│  ├── state: Arc<RwLock<PipeWireState>>                 │
│  ├── command_tx: Sender<PwCommand>                     │
│  └── response_rx: Receiver<PwResponse>                 │
└─────────────────────────────────────────────────────────┘
                    │ crossbeam-channel
                    ▼
┌─────────────────────────────────────────────────────────┐
│                  PipeWire Thread                         │
│  MainLoop::run()                                        │
│  ├── Registry listener (nodes/ports/links)             │
│  ├── Command handler (create/destroy objects)          │
│  ├── Capture stream (reads from virtual sink monitor)  │
│  ├── Playback stream (outputs to speakers)             │
│  └── Local state (Rc<RefCell<...>>)                    │
└─────────────────────────────────────────────────────────┘
```

## Audio Flow

```
┌─────────────┐     ┌─────────────────┐     ┌────────────────┐     ┌─────────────┐
│   Firefox   │────▶│   Gecko Audio   │────▶│ Gecko Capture  │────▶│   Gecko     │
│   (App)     │link │  (Virtual Sink) │     │   (Stream)     │     │  Playback   │
│  [Out Port] │     │ [In] [Monitor]  │     │  [In Ports]    │     │ [Out Ports] │
└─────────────┘     └─────────────────┘     └────────────────┘     └─────────────┘
                                                    │                      │
                                                    ▼                      ▼
                                              ┌──────────┐          ┌─────────────┐
                                              │   DSP    │          │  Speakers   │
                                              │   (EQ)   │ ─ring──▶ │  (Hardware) │
                                              └──────────┘  buffer  └─────────────┘
```

## Implementation Files

| File | Purpose |
|------|---------|
| `linux/mod.rs` | PipeWireBackend struct, trait implementation |
| `linux/thread.rs` | PipeWire thread with MainLoop, registry, streaming |
| `linux/message.rs` | PwCommand/PwResponse for thread communication |
| `linux/state.rs` | PwNodeInfo, PwPortInfo, PwLinkInfo types |
| `linux/filter.rs` | FilterState for real-time audio processing |
| `linux/audio_stream.rs` | AudioProcessingState, StreamConfig types |

## Dependencies

```toml
# Cargo.toml
[target.'cfg(target_os = "linux")'.dependencies]
pipewire = "0.8"
crossbeam-channel = "0.5"
rtrb = "0.3"  # Lock-free ring buffer
```

System packages required:
```bash
sudo apt install libpipewire-0.3-dev libspa-0.2-dev
```

## Virtual Sink Creation

Gecko creates virtual devices at runtime - no installation needed:

```rust
// In PipeWireBackend
let config = VirtualSinkConfig {
    name: "Gecko Audio".to_string(),
    channels: 2,
    sample_rate: 48000,
    persistent: false,
};

let sink_id = backend.create_virtual_sink(config)?;
```

### Properties Used
```rust
let props = properties! {
    "factory.name" => "support.null-audio-sink",
    "node.name" => "Gecko Audio",
    "media.class" => "Audio/Sink",
    "audio.channels" => "2",
    "audio.rate" => "48000",
    "object.linger" => "false",  // Disappears when Gecko closes
    "node.description" => "Gecko Audio",
};
```

### Result
- Device appears in system volume control (GNOME/KDE Settings)
- Users can route any app to "Gecko Audio"
- Gecko captures audio from this sink's monitor port

## Audio Streaming

### Capture Stream
Reads from virtual sink's monitor ports:
```rust
let capture_props = properties! {
    "media.type" => "Audio",
    "media.category" => "Capture",
    "node.name" => "Gecko Capture",
    "node.passive" => "true",       // Don't trigger auto-routing
    "node.autoconnect" => "false",  // Manual link management
};
```

### Playback Stream
Outputs processed audio to speakers:
```rust
let playback_props = properties! {
    "media.type" => "Audio",
    "media.category" => "Playback",
    "node.name" => "Gecko Playback",
    "target.object" => target_name,  // Target device by NAME (not ID)
    "node.dont-reconnect" => "true",
};
```

### Ring Buffer
Audio flows: Capture callback → Ring buffer → Playback callback
```rust
const RING_BUFFER_SIZE: usize = 48000 * 2;  // ~1 second stereo
let (producer, consumer) = rtrb::RingBuffer::new(RING_BUFFER_SIZE);
```

## DSP Integration

EQ processing happens in the capture callback:

```rust
// In capture callback
let current_eq_counter = user_data.audio_state.eq_update_counter();
if current_eq_counter != user_data.last_eq_update_counter {
    // Apply EQ band updates from shared state
    let gains = user_data.audio_state.get_all_eq_gains();
    for (band, gain_db) in gains.iter().enumerate() {
        user_data.equalizer.set_band_gain(band, *gain_db);
    }
}

// Process samples through EQ
if !user_data.audio_state.bypassed.load(Ordering::Relaxed) {
    user_data.equalizer.process_interleaved(samples);
}

// Apply master volume
let volume = user_data.audio_state.master_volume();
for sample in samples.iter_mut() {
    *sample *= volume;
}
```

## Device Hotplug (SwitchPlaybackTarget)

When the default audio device changes (e.g., USB headphones plugged in):

1. Engine detects default sink change
2. Calls `switch_playback_target(device_name)`
3. PipeWire thread disconnects old streams
4. Clears stale registry entries (CRITICAL - avoids "ghost" links)
5. Creates new streams targeting new device
6. Re-establishes capture links to virtual sink monitor

**Key insight**: Use device NAME not ID for targeting, as IDs change during hotplug.

## Thread Safety Model

PipeWire objects are NOT Send/Sync, so we use a dedicated thread:

```rust
// Main thread sends commands
command_tx.send(PwCommand::StartStreaming { ... })?;

// PipeWire thread processes them
let response = response_rx.recv_timeout(Duration::from_secs(5))?;
```

Shared state uses `Arc<RwLock<PipeWireState>>` with `try_write()` to avoid blocking.

### EQ Updates (Lock-Free)
```rust
// AudioProcessingState uses atomics for real-time safety
pub struct AudioProcessingState {
    eq_band_gains: [AtomicU32; 10],    // Stored as f32 bits
    eq_update_counter: AtomicU32,      // Increment on change
    master_volume_bits: AtomicU32,
    // ...
}
```

## Feature Flag

PipeWire support is behind a feature flag:

```toml
# gecko_platform/Cargo.toml
[features]
default = []
pipewire = ["dep:pipewire"]
```

## Current Status

### Implemented ✅
- PipeWire connection and registry monitoring
- Virtual sink creation/destruction
- Link creation/destruction
- Application discovery (nodes with `Stream/Output/Audio` class)
- Channel-matched linking (FL→FL, FR→FR)
- **Capture stream** from virtual sink monitor
- **Playback stream** to speakers (with device targeting)
- **DSP in audio path** (10-band EQ)
- **Level metering** (peak levels to UI)
- **Device hotplug** (seamless output switching)
- **Real-time EQ updates** (atomic counter pattern)
- **Per-app audio routing** (automatic default sink switching)
- **Per-app EQ** (independent EQ per application, processed before mixing)
- **Per-app volume** (0-200% individual volume control)
- **Per-app bypass** (skip EQ processing per app)
- **Settings persistence** (EQ, volume, bypass saved per-app by name)

### Fully Functional
The Linux PipeWire backend is fully implemented and production-ready.

## Testing

```bash
# Run unit tests (no PipeWire needed)
cargo test -p gecko_platform

# Run integration tests (requires PipeWire daemon)
cargo test -p gecko_platform --features pipewire -- --ignored

# Verify virtual sink manually
pw-cli ls Node | grep Gecko
```

## Debugging

```bash
# Monitor PipeWire events
pw-mon

# List all nodes
pw-cli ls Node

# List all links
pw-cli ls Link

# Check Gecko logs
RUST_LOG=gecko_platform=debug pnpm tauri dev
```

## Common Issues

### Stale Registry Data After Hotplug
When switching playback targets, old node/port/link entries can remain in local state.
**Solution**: Clear stale entries before creating new streams (see `SwitchPlaybackTarget` handler).

### Capture Links Not Created
WirePlumber may auto-connect capture to wrong source.
**Solution**: Use `node.autoconnect=false` and create links manually after stream appears.

### EQ Updates Not Applied
If counter mechanism fails, check that `AudioProcessingState` Arc is shared between command handler and callback.

## Related Documentation

- [audio-pipeline.md](audio-pipeline.md) - Overall audio flow
- [realtime-rules.md](realtime-rules.md) - Rules for audio callbacks
- [eq-implementation.md](../features/eq-implementation.md) - EQ filter details
- [mistake-log.md](../ai-patterns/mistake-log.md) - Past mistakes to avoid
