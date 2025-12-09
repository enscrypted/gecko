# Gecko Development Guide

## Quick Start

### Prerequisites

**Linux (Ubuntu/Debian)**:
```bash
# System dependencies
sudo apt update
sudo apt install -y \
    libpipewire-0.3-dev \
    libspa-0.2-dev \
    libasound2-dev \
    libgtk-3-dev \
    libwebkit2gtk-4.1-dev \
    libjavascriptcoregtk-4.1-dev \
    libsoup-3.0-dev \
    libclang-dev \
    build-essential

# Rust (if not installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# pnpm (if not installed)
curl -fsSL https://get.pnpm.io/install.sh | sh -
source ~/.bashrc
```

### Build and Run

```bash
# Install frontend dependencies
pnpm install

# Run in development mode (hot reload)
pnpm tauri dev

# Build production release
pnpm tauri build
```

## Project Structure

```
gecko/
├── src/                    # Frontend (React/TypeScript)
│   ├── components/         # React components
│   ├── hooks/             # Custom React hooks
│   └── App.tsx            # Main app component
├── src-tauri/             # Tauri app wrapper
│   └── src/               # Tauri commands/setup
├── crates/
│   ├── gecko_core/        # Audio engine
│   ├── gecko_dsp/         # DSP processing (EQ)
│   └── gecko_platform/    # Platform-specific backends
│       └── src/linux/     # PipeWire implementation
└── docs/
    └── ai-knowledge/      # Documentation for AI agents
```

## Running Tests

```bash
# All tests (quick, no hardware needed)
cargo test --workspace

# With verbose output
cargo test --workspace -- --nocapture

# PipeWire integration tests (requires running daemon)
cargo test -p gecko_platform --features pipewire -- --ignored

# Specific package
cargo test -p gecko_core
cargo test -p gecko_dsp
cargo test -p gecko_platform
```

## Debugging

### Enable Logging

```bash
# Basic logging
RUST_LOG=info pnpm tauri dev

# Detailed logging
RUST_LOG=debug pnpm tauri dev

# Specific modules
RUST_LOG=gecko_core=debug,gecko_platform=trace pnpm tauri dev
```

### PipeWire Debugging

```bash
# Monitor all PipeWire events (very verbose)
pw-mon

# List all audio nodes
pw-cli ls Node

# List all links
pw-cli ls Link

# Check if Gecko virtual sink exists
pw-cli ls Node | grep -i gecko

# Check PipeWire service status
systemctl --user status pipewire pipewire-pulse wireplumber
```

### Common Issues

#### "PipeWire connection failed"
```bash
# Ensure PipeWire is running
systemctl --user start pipewire pipewire-pulse wireplumber

# Check if PipeWire is the audio server
pactl info | grep "Server Name"  # Should show PipeWire
```

#### No audio devices found
```bash
# List ALSA devices (used by CPAL)
aplay -l
arecord -l

# List PipeWire devices
pw-cli ls Node | grep -E "(Sink|Source)"
```

#### Build errors
```bash
# Clean build
cargo clean
pnpm tauri dev

# Check for missing dependencies
pkg-config --libs pipewire-0.3
pkg-config --libs libspa-0.2
```

## Code Quality

```bash
# Format code
cargo fmt --all

# Lint
cargo clippy --workspace

# Format + Lint (run before commits)
cargo fmt --all && cargo clippy --workspace
```

## Current Functionality (v0.1)

### What Works
- UI with 10-band EQ sliders
- Start/Stop engine controls
- Volume control
- PipeWire virtual sink creation ("Gecko Audio")
- Application discovery in PipeWire graph

### What's In Progress
- Audio actually flowing through DSP
- Routing virtual sink audio to speakers
- Per-app EQ profiles

### What's NOT Implemented Yet
- Windows support (WASAPI)
- macOS support (CoreAudio)
- Preset management
- Audio visualization

## Architecture Notes

### Audio Flow (Linux)
```
App Audio (Firefox, Spotify)
       ↓ (user routes in system settings)
"Gecko Audio" Virtual Sink
       ↓ (PipeWire monitor)
DSP Processing (EQ, volume)
       ↓
Real Speakers
```

### Thread Model
- **Main Thread**: UI, Tauri commands
- **Audio Thread**: Manages audio streams, processes commands
- **PipeWire Thread**: Registry monitoring, graph manipulation

### Key Design Decisions

1. **No microphone input** - We capture APPLICATION audio, not mic
2. **Lock-free audio** - No mutexes in audio callbacks
3. **PipeWire native** - Use PipeWire API directly, not ALSA abstraction
4. **Feature flags** - Platform support via Cargo features

## Useful Commands

```bash
# Watch for file changes and rebuild
cargo watch -x "build -p gecko_core"

# Check dependencies for security issues
cargo audit

# See dependency tree
cargo tree -p gecko_platform

# Profile build times
cargo build --timings
```

## Getting Help

- Read `AGENT.md` for coding conventions
- Check `docs/ai-knowledge/` for detailed documentation
- See `docs/ai-knowledge/ai-patterns/mistake-log.md` for common pitfalls
