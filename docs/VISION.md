# Gecko Audio App - Complete Vision Document

**Source**: `Gecko Audio App Architecture Research.pdf` (Original Vision Document)
**Last Updated**: December 2024
**Purpose**: Comprehensive reference for implementation, marking what's done vs. pending

---

## Executive Summary

Gecko is a **per-application audio equalizer** that captures audio from individual applications (not microphone!), applies DSP processing, and outputs to speakers. The key differentiator is **per-app control** - each audio stream gets its own EQ settings.

---

## Architecture Overview

### Hybrid-Native Core (from PDF Section 2)

```
┌─────────────────────────────────────────────────────────────────┐
│                         UI Shell                                 │
│  Tauri + React + TypeScript + Tailwind                          │
│  - Per-app stream list with expandable EQ controls              │
│  - Master EQ at top                                             │
│  - Real-time visualizations (levels, FFT)                       │
└─────────────────────────────────────────────────────────────────┘
                              │ Tauri IPC (invoke/listen)
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                        gecko_core                                │
│  AudioEngine - coordinates all audio operations                 │
│  - Command/Event channels for UI communication                  │
│  - Spawns Audio Thread for real-time processing                 │
└─────────────────────────────────────────────────────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          ▼                   ▼                   ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│  gecko_platform │  │    gecko_dsp    │  │   gecko_common  │
│  (OS backends)  │  │  (Processing)   │  │    (Shared)     │
└─────────────────┘  └─────────────────┘  └─────────────────┘
```

**Status**: ✅ Crate structure exists | ⚠️ Integration incomplete

---

## Platform Backends (from PDF Section 5-7)

### Linux: PipeWire (PDF Section 6)

**Vision**:
```
App (Firefox) ──▶ Gecko Virtual Sink ──▶ Gecko DSP ──▶ Real Speakers
                        │
                        └── Monitor port for capture
```

| Feature | PDF Vision | Status |
|---------|------------|--------|
| Virtual sink creation | `null-audio-sink` factory | ✅ Implemented |
| Registry monitoring | Track nodes/ports/links | ✅ Implemented |
| Link creation | Connect app → virtual sink | ✅ Implemented |
| Audio capture from monitor | Stream API with callback | ✅ Implemented |
| DSP in audio path | Process captured audio | ✅ Implemented |
| Output to speakers | Playback stream | ✅ Implemented |
| Per-app routing | Auto-detect streams routing to Gecko | ✅ Implemented |
| Per-app EQ | Individual EQ per app before mixing | ✅ Implemented |
| Per-app volume | Individual volume control per app | ✅ Implemented |

**Status**: ✅ Fully functional on Linux with PipeWire!

### Windows: WASAPI Process Loopback (PDF Section 5)

**Vision**:
```rust
// PDF specifies Windows 10 2004+ Process Loopback API
AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
    TargetProcessId: pid,
    ProcessLoopbackMode: INCLUDE_TARGET_PROCESS_TREE,
}
```

| Feature | PDF Vision | Status |
|---------|------------|--------|
| Process loopback capture | Capture specific PID | ❌ NOT IMPLEMENTED |
| IAudioClient3 activation | Low-latency activation | ❌ NOT IMPLEMENTED |
| Output to speakers | WASAPI render stream | ❌ NOT IMPLEMENTED |

**Current**: Only CPAL output-only mode (no capture).

### macOS: CoreAudio HAL Plugin (PDF Section 7)

**Vision**:
```
┌─────────────────┐    Shared Memory    ┌─────────────────┐
│  HAL Plugin     │◀──────────────────▶│  Gecko Main App │
│ (AudioServer)   │    Ring Buffer      │  (User Space)   │
└─────────────────┘                     └─────────────────┘
```

| Feature | PDF Vision | Status |
|---------|------------|--------|
| HAL plugin (`AudioServerPlugIn`) | System extension | ❌ NOT IMPLEMENTED |
| Shared memory IPC | `shm_open` + ring buffer | ❌ NOT IMPLEMENTED |
| Virtual device routing | Aggregate device | ❌ NOT IMPLEMENTED |
| Codesigning/notarization | Apple requirements | ❌ NOT IMPLEMENTED |

**Current**: Only CPAL output-only mode.

---

## DSP Pipeline (from PDF Section 3)

### 10-Band Parametric EQ

**Frequencies** (from PDF):
| Band | Frequency | Type |
|------|-----------|------|
| 1 | 31 Hz | Low Shelf |
| 2 | 62 Hz | Peaking |
| 3 | 125 Hz | Peaking |
| 4 | 250 Hz | Peaking |
| 5 | 500 Hz | Peaking |
| 6 | 1 kHz | Peaking |
| 7 | 2 kHz | Peaking |
| 8 | 4 kHz | Peaking |
| 9 | 8 kHz | Peaking |
| 10 | 16 kHz | High Shelf |

**Status**: ✅ Implemented in `gecko_dsp/src/eq.rs`

### Processing Chain

```rust
// PDF vision for audio callback
fn audio_callback(input: &[f32], output: &mut [f32]) {
    // 1. Copy input to processing buffer (or process in-place)
    // 2. Apply EQ cascade (10 biquad filters)
    // 3. Apply master volume
    // 4. Soft clip if needed
    // 5. Write to output
    // 6. Send copy to analysis thread (FFT)
}
```

| Feature | PDF Vision | Status |
|---------|------------|--------|
| BiQuad filters | `biquad` crate | ✅ Implemented |
| Coefficient updates | Lock-free atomic | ✅ Implemented |
| In-place processing | `process_interleaved()` | ✅ Implemented |
| Master volume | Linear gain | ✅ Implemented in audio path |
| Per-app volume | Individual app volume (0-200%) | ✅ Implemented |
| Per-app bypass | Skip EQ per app | ✅ Implemented |
| Soft clipping | Prevent hard distortion | ✅ Implemented (tanh-based limiter) |
| FFT analysis | Send to UI | ✅ Implemented (32-bin spectrum analyzer) |

---

## Per-Application Audio (CORE FEATURE)

### PDF Vision (Section 5-7)

Each platform provides per-process audio capture:
- **Linux**: PipeWire links specific app nodes to Gecko
- **Windows**: Process Loopback API targets specific PIDs
- **macOS**: HAL plugin can intercept specific app audio

### UI Vision (from user description)

```
┌─────────────────────────────────────────────────────────────┐
│  Gecko Audio                                          [─][□][×]│
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ▼ Master (All Apps Combined)                        [ON]  │
│    ┌─────────────────────────────────────────────────────┐  │
│    │ [31Hz][62Hz][125Hz]...[16kHz]  Sliders             │  │
│    │  -24     0    +6              +12dB                │  │
│    └─────────────────────────────────────────────────────┘  │
│                                                             │
│  ▶ Firefox (pid: 12345)                              [ON]  │
│    Click to expand EQ controls...                          │
│                                                             │
│  ▶ Spotify (pid: 67890)                              [ON]  │
│    Click to expand EQ controls...                          │
│                                                             │
│  ▶ Discord (pid: 11111)                             [OFF]  │
│    Click to expand EQ controls...                          │
│                                                             │
│  ──────────────────────────────────────────────────────────│
│  L ████████████░░░░░░░░░░░░░░░░░░░░░░░  R                  │
│  Level Meters                                              │
│                                                             │
│  Output: [Speakers (sof-hda-dsp) ▼]                        │
└─────────────────────────────────────────────────────────────┘
```

| Feature | Vision | Status |
|---------|--------|--------|
| List audio streams | Show Firefox, Spotify, etc. | ✅ Implemented (StreamList component) |
| Per-stream EQ | Each app has own EQ | ✅ Implemented (expandable EQ per app) |
| Master EQ | Combined output | ✅ Implemented |
| Expandable items | Click to show/hide sliders | ✅ Implemented (accordion pattern) |
| Stream metadata | App name | ✅ Implemented |
| Per-stream bypass | Enable/disable per app | ✅ Implemented |
| Per-stream volume | 0-200% volume per app | ✅ Implemented |
| Settings persistence | Save EQ/volume across restart | ✅ Implemented |

---

## Thread Model (from PDF Section 4)

### Audio Thread (Real-Time)
```rust
// HARD RULES - VIOLATING CAUSES AUDIBLE GLITCHES
// - NO heap allocations
// - NO syscalls (file, network, mutex)
// - NO unbounded loops
// - Constant or O(n) time only
```

| Rule | Status |
|------|--------|
| Zero allocations in callback | ✅ EQ design follows this |
| Lock-free communication | ✅ crossbeam channels |
| Pre-allocated buffers | ✅ Vec allocated at setup |

### UI Thread
- React components
- Tauri IPC handlers
- Non-blocking operations

**Status**: ✅ Implemented

### Analysis Thread (Optional)
- FFT computation for visualizations
- Level metering
- Not time-critical

**Status**: ✅ Implemented - FFT spectrum analyzer with 32 logarithmic bins

---

## Communication (from PDF Section 4.3)

### Commands (UI → Audio)

```rust
pub enum Command {
    Start,
    Stop,
    SetBandGain { band: usize, gain_db: f32 },
    SetMasterVolume(f32),
    SetBypass(bool),
    // Per-app commands (NOT IMPLEMENTED)
    SetAppBandGain { pid: u32, band: usize, gain_db: f32 },
    SetAppBypass { pid: u32, bypassed: bool },
    RouteApp { pid: u32, to_gecko: bool },
}
```

**Status**: ✅ All commands implemented including per-app

### Events (Audio → UI)

```rust
pub enum Event {
    Started,
    Stopped,
    LevelUpdate { left: f32, right: f32 },
    Error { message: String },
    StreamDiscovered { name: String },
    StreamRemoved { name: String },
}
```

**Status**: ✅ Events implemented, per-app discovery via polling

---

## Implementation Roadmap

### Phase 1: Audio Routing ✅ COMPLETE
1. ✅ Implement PipeWire capture stream (from virtual sink monitor)
2. ✅ Implement PipeWire playback stream (to speakers)
3. ✅ Wire DSP into audio path
4. ✅ Get level meters working

### Phase 2: Per-App Support ✅ COMPLETE
1. ✅ Discover running audio apps via PipeWire registry
2. ✅ Create UI component for stream list (StreamList, AudioStreamItem)
3. ✅ Per-app EQ state management (with persistence)
4. ✅ Per-app commands/events
5. ✅ Per-app volume control
6. ✅ Per-app bypass

### Phase 3: Cross-Platform (PENDING)
1. ❌ Windows: WASAPI process loopback
2. ❌ macOS: HAL plugin (significant effort)

### Phase 4: Polish ✅ COMPLETE (Linux)
1. ✅ FFT visualization (32-bin spectrum analyzer with toggle)
2. ✅ Presets (save/load EQ configs)
3. ✅ System tray integration (minimize to tray, click to restore)
4. ✅ Auto-start option (tauri-plugin-autostart)
5. ✅ Theme system with multiple colorways
6. ✅ Settings UI with all options
7. ✅ Soft clipping (tanh-based limiter)

---

## Implementation Status Summary

### Platform Support

| Platform | Status | Notes |
|----------|--------|-------|
| Linux (PipeWire) | ✅ Fully functional | Per-app EQ, volume, bypass all working |
| Windows (WASAPI) | ❌ Not implemented | Needs Process Loopback API |
| macOS (CoreAudio) | ❌ Not implemented | Needs HAL plugin |

### What Works Today (Linux)
- ✅ Virtual sink appears in sound settings
- ✅ Audio flows through Gecko DSP pipeline
- ✅ Per-app EQ with 10-band parametric
- ✅ Per-app volume control (0-200%)
- ✅ Per-app bypass toggle
- ✅ Level meters animate with audio
- ✅ FFT spectrum analyzer (32-bin, toggleable with L/R meters)
- ✅ Soft clipping (tanh-based limiter) - toggle in settings
- ✅ System tray integration (minimize to tray, click to restore)
- ✅ Auto-start on login option
- ✅ Settings persist across app restarts
- ✅ Theme system with 7 themes (including accessibility)
- ✅ Stable stop/start cycles

### What's Still Missing
- ❌ Windows/macOS platform support

---

## File Reference

### Critical Implementation Files

| File | Purpose | Status |
|------|---------|--------|
| `gecko_platform/src/linux/thread.rs` | PipeWire thread with streaming | ✅ Complete |
| `gecko_platform/src/linux/audio_stream.rs` | Stream types and state | ✅ Complete |
| `gecko_core/src/engine.rs` | Audio engine coordination | ✅ Complete |
| `gecko_dsp/src/eq.rs` | 10-band parametric EQ | ✅ Complete |
| `gecko_dsp/src/fft.rs` | FFT spectrum analyzer (32-bin) | ✅ Complete |
| `gecko_dsp/src/soft_clip.rs` | Tanh-based soft clipper/limiter | ✅ Complete |
| `src/components/Equalizer.tsx` | EQ slider UI | ✅ Complete |
| `src/components/StreamList.tsx` | Per-app stream list | ✅ Complete |
| `src/components/AudioStreamItem.tsx` | Individual app row with EQ | ✅ Complete |
| `src/components/SpectrumAnalyzer.tsx` | FFT visualization (toggleable) | ✅ Complete |
| `src/contexts/SettingsContext.tsx` | Settings persistence | ✅ Complete |
| `src/components/Settings.tsx` | Settings modal with themes | ✅ Complete |

---

## Next Steps

1. **Windows/macOS support** - Platform backends for cross-platform

**Linux development is feature-complete!** All Phase 1-4 features are implemented.

---

*Last updated: December 2024 (Phase 4 complete)*
