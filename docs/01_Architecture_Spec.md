# Gecko Audio Architecture Specification

## 1. System Overview

Gecko is a high-performance, cross-platform audio routing and processing application. It intercepts audio from specific applications or devices, processes it via a Digital Signal Processing (DSP) pipeline, and routes it to virtual or physical outputs.

### 1.1 Architectural Philosophy

The system follows a **Core-Shell** architecture:

- **The Core (`gecko_core`)**: A headless Rust library responsible for all audio I/O, DSP, and OS interaction. Designed to be crash-resilient and thread-safe.
- **The Shell (`gecko_ui`)**: A Tauri-based graphical interface responsible for user interaction, configuration, and visualization.

This separation ensures that the audio engine can run with real-time priority, decoupled from the rendering loop of the webview.

## 2. Technology Stack

| Component | Choice | Justification |
|-----------|--------|---------------|
| Language | Rust (2021 Edition) | Memory safety in concurrent threads; Zero-cost abstractions for DSP |
| GUI Framework | Tauri v2 | Lightweight (<5MB); Direct Rust backend integration |
| Audio Transport | CPAL (v0.16+) | Rust-native; Safe abstractions for WASAPI/CoreAudio/ALSA |
| Windows Backend | windows-rs | Access to `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS` |
| macOS Backend | CoreAudio HAL | Access to AudioServerPlugIn via Shared Memory |
| Linux Backend | pipewire-rs | Native graph manipulation and virtual sink creation |
| DSP Engine | biquad crate | `no_std` compatible; Pre-calculated filter coefficients |

## 3. Data Flow Architecture

The audio pipeline is strict and unidirectional to prevent feedback loops and race conditions.

```
┌─────────────────────────────────────────────────────────────┐
│                        UI Thread                            │
│  (Tauri/Web) ──commands──▶ Engine ◀──events── (Tauri/Web)  │
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

### 3.1 Pipeline Steps

1. **Capture Source**: OS Audio Callback (WASAPI Loopback / HAL / PipeWire Monitor)
   - Data is written to a lock-free **Ring Buffer**

2. **Thread Boundary**:
   - Capture Thread writes to the buffer
   - Processing Thread reads from the buffer
   - Mechanism: `rtrb` (Real-Time Ring Buffer) crate

3. **DSP Processing**:
   - Input: Interleaved or de-interleaved float frames
   - Action: Apply 10-Band EQ, Volume, and Limiting
   - **Constraint**: Zero allocation. No syscalls.

4. **Output / Visualization**:
   - Path A (Audio): Processed samples written to Output Stream (CPAL)
   - Path B (Visual): Copy of samples sent to Analysis Thread (FFT) → Tauri Event → Frontend

## 4. Concurrency Model

Audio programming requires rigorous thread management:

- **Audio Thread**: High priority. No blocking. No Mutexes. Communication via `crossbeam-channel` (TryRecv) for control messages.
- **UI Thread**: Standard priority. Reacts to user input. Sends messages to Audio Thread.
- **Analysis Thread**: Low priority. Computes FFT/RMS for UI visualization. Decoupled from Audio Thread.

## 5. Error Handling Strategy

- **Device Disconnect**: CPAL streams return an error on device loss. The Core must catch this, pause the pipeline, and attempt re-initialization.
- **Buffer Underrun**: If the DSP takes too long, the output callback may starve. The system logs these "Xruns" and potentially degrades quality to maintain stream continuity.

## 6. Crate Structure

```
gecko/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── gecko_core/         # Audio engine, stream management
│   ├── gecko_dsp/          # Signal processing, EQ
│   └── gecko_platform/     # OS-specific backends
├── src-tauri/              # Tauri application
└── src/                    # Frontend (React)
```
