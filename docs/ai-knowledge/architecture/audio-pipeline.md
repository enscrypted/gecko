# Audio Pipeline Architecture

**Last Updated**: December 2024
**Context**: Read when working on audio engine, DSP, thread model, or understanding data flow

## ⚠️ CRITICAL: No Microphone Input ⚠️

**Gecko captures APPLICATION AUDIO, NOT microphone input!**

```
CORRECT:  App Audio (Firefox, Spotify) → Virtual Sink → Gecko DSP → Speakers
WRONG:    Microphone → Gecko DSP → Speakers  ← CAUSES FEEDBACK LOOP!
```

Never use `host.default_input_device()` - that grabs the microphone.

## Overview

Gecko uses a **Core-Shell** architecture where the audio engine (Core) operates independently from the UI (Shell). This separation ensures audio processing maintains real-time priority without UI interference.

## Data Flow

```
┌─────────────────────────────────────────────────────────────┐
│                        UI Thread                            │
│  (Tauri/React) ──commands──▶ Engine ◀──events── (Tauri)    │
└─────────────────────────────────────────────────────────────┘
                             │ crossbeam-channel
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                      Audio Thread                           │
│   Capture ──rtrb──▶ DSP Chain ──rtrb──▶ Output             │
│     │                   │                  │                │
│     └───────────────────┴──────────────────┘                │
│              (Zero allocation in this path)                 │
└─────────────────────────────────────────────────────────────┘
```

## Pipeline Steps

### 1. Capture Source (APPLICATION AUDIO - NOT MICROPHONE!)
- **Input**: Platform-specific application audio capture:
  - **Linux**: PipeWire virtual sink monitor port
  - **Windows**: WASAPI Process Loopback API
  - **macOS**: HAL Plugin via shared memory
- **Action**: Data written to lock-free Ring Buffer (rtrb)
- **Thread**: OS callback thread (high priority)
- **⚠️ WARNING**: Never use `default_input_device()` - that's the microphone!

### 2. Thread Boundary
- **Mechanism**: `rtrb` (Real-Time Ring Buffer) crate
- **Pattern**: Single Producer Single Consumer (SPSC)
- **Safety**: No locks, no allocations

### 3. DSP Processing (Per-App + Master)
- **Input**: Interleaved float frames `[L0, R0, L1, R1, ...]`
- **Per-App Processing** (if enabled):
  - Check per-app bypass flag (atomic)
  - Apply per-app 10-band EQ (independent gains per app)
  - Apply per-app volume (0-200%)
- **Master Processing**:
  - Apply master 10-Band EQ cascade
  - Apply master volume
- **Constraint**: ZERO allocation, NO syscalls

### 4. Output
- **Path A (Audio)**: Processed samples to Output Stream
- **Path B (Visual)**: Peak levels sent to UI via atomic state

## Thread Model

| Thread | Priority | Purpose | Communication |
|--------|----------|---------|---------------|
| UI Thread | Normal | User interaction, rendering | Tauri IPC |
| Audio Thread | Real-time | Capture, DSP, playback | crossbeam (try_recv) |
| Analysis Thread | Low | FFT, level metering | rtrb (one-way) |

## Key Data Structures

### Command Channel (UI → Audio)
```rust
pub enum Command {
    Start,
    Stop,
    SetBandGain { band: usize, gain_db: f32 },
    SetMasterVolume(f32),
    SetBypass(bool),
    // Per-app commands
    SetStreamBandGain { stream_id: String, band: usize, gain_db: f32 },
    SetStreamVolume { stream_id: String, volume: f32 },
    SetAppBypass { app_name: String, bypassed: bool },
}
```

### Event Channel (Audio → UI)
```rust
pub enum Event {
    Started,
    Stopped,
    LevelUpdate { left: f32, right: f32 },
    Error { message: String },
}
```

## Buffer Sizes

| Setting | Value | Latency |
|---------|-------|---------|
| Low Latency | 128 samples | ~2.7ms @ 48kHz |
| Default | 512 samples | ~10.7ms @ 48kHz |
| High Buffer | 1024 samples | ~21.3ms @ 48kHz |

## Error Handling

- **Device Disconnect**: CPAL returns error, Core pauses pipeline and attempts re-init
- **Buffer Underrun**: Log "Xrun", optionally bypass heavy effects
- **DSP Overflow**: Clamp output to [-1.0, 1.0], log warning

## Related Files

- `crates/gecko_core/src/engine.rs` - Main engine controller
- `crates/gecko_core/src/stream.rs` - Audio stream management
- `crates/gecko_core/src/message.rs` - Command/Event definitions
- `crates/gecko_dsp/src/processor.rs` - AudioProcessor trait
