# Gecko Audio

A high-performance, cross-platform audio routing and processing application with **true per-application EQ**.

Gecko intercepts audio from specific applications, processes it through a 10-band parametric EQ (with individual settings per app), and routes it to your speakers - all with real-time performance.

## Features

- **Per-App EQ**: Apply different EQ settings to each application independently
- **10-Band Parametric EQ**: Full control from 31Hz to 16kHz
- **Real-Time Performance**: Zero-allocation audio processing with < 10ms latency
- **System Integration**: Volume keys control Gecko, system OSD shows feedback
- **Built-in Presets**: Rock, Pop, Jazz, Classical, Bass Boost, and more
- **Custom Presets**: Save and load your own EQ configurations
- **Spectrum Analyzer**: Real-time FFT visualization
- **Soft Limiter**: Prevents harsh digital clipping

## Platform Support

| Platform | Status | Audio Backend | Per-App EQ |
|----------|--------|---------------|------------|
| Linux | ✅ Implemented | PipeWire | ✅ All apps |
| macOS 14.4+ | ✅ Implemented | CoreAudio Process Tap | ✅ Most apps* |
| Windows | Planned | WASAPI Process Loopback | Planned |

*macOS limitations: Safari, FaceTime, Messages, and system sounds cannot be captured due to Apple's sandboxing. Other apps work via the Process Tap API.

## Requirements

### Linux

- PipeWire 0.3+ (default on Fedora, Ubuntu 22.10+, Arch)
- WirePlumber (session manager)
- Node.js 18+ and pnpm
- Rust 1.70+

### macOS

- **macOS 14.4+** (Sonoma 14.4 or later) - **required**
- Xcode Command Line Tools
- Node.js 18+ and pnpm
- Rust 1.70+

> **Important**: macOS 14.4+ is required for Gecko to work. The Process Tap API introduced in macOS 14.4 is the only supported method for per-app audio capture. Older macOS versions are not supported.

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/enscrypted/gecko.git
cd gecko

# Install frontend dependencies
pnpm install

# Build and run in development mode
pnpm tauri dev

# Build for production
pnpm tauri build
```

### Linux Dependencies

On Debian/Ubuntu:
```bash
sudo apt install libpipewire-0.3-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libsoup-3.0-dev libclang-dev
```

On Fedora:
```bash
sudo dnf install pipewire-devel gtk3-devel webkit2gtk4.1-devel \
  javascriptcoregtk4.1-devel libsoup3-devel clang-devel
```

### macOS Dependencies

```bash
# Install Xcode Command Line Tools
xcode-select --install

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Usage

### Linux

1. **Start Gecko**: Launch the application
2. **Route Audio**: In your system sound settings, change application output to "Gecko Audio"
3. **Adjust EQ**: Use the per-app sliders or apply a preset
4. **System Volume**: Use your volume keys - they control Gecko seamlessly

### macOS

1. **First Launch**: Grant permissions when prompted:
   - **Screen Recording**: Required for Process Tap API (captures app audio, not your screen)
   - **Microphone**: Required by macOS for audio capture (Gecko never records your voice)

2. **Start Gecko**: Launch the application

3. **Capture Apps**: Click the "Capture" toggle next to any app in the stream list
   - Captured apps route through Gecko's EQ
   - Non-captured apps play directly (no EQ)

4. **Adjust EQ**:
   - Click an app row to expand its individual EQ
   - Or use the Master EQ (affects all audio)

5. **Protected Apps**: Safari, FaceTime, Messages show as "Protected"
   - These cannot be captured due to Apple security
   - They still receive Master EQ when playing through system audio

### macOS Permissions Explained

Gecko requires two permissions that may seem unusual:

| Permission | Why It's Needed | What Gecko Does |
|------------|-----------------|-----------------|
| Screen Recording | Apple's Process Tap API is classified as "screen recording" because it captures other apps' output | Captures audio from apps (NOT your screen) |
| Microphone | macOS requires this for any audio capture, even from apps | Captures app audio (NOT your voice) |

**Privacy**: Gecko never records your screen, camera, or microphone input. It only processes audio from applications you explicitly select.

## Architecture

### Linux (PipeWire)

```
App Audio (Firefox, Spotify, etc.)
       │
       ▼
┌─────────────────────────────────┐
│ Gecko Audio (Virtual Sink)      │
│ - Apps route here via settings  │
└─────────────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│ Per-App EQ Processing           │
│ - Firefox EQ (separate)         │
│ - Spotify EQ (separate)         │
│ - Discord EQ (separate)         │
└─────────────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│ Master EQ → Soft Limiter        │
└─────────────────────────────────┘
       │
       ▼
    Speakers
```

### macOS (Process Tap)

```
┌─────────────────────────────────┐
│ App Audio (Firefox, Spotify)    │
│ - User clicks "Capture" toggle  │
└─────────────────────────────────┘
       │
       │ Process Tap API (macOS 14.4+)
       │ - Per-process audio capture
       │ - No driver installation needed
       │ - Audio muted from original path
       ▼
┌─────────────────────────────────┐
│ Per-App EQ Processing           │
│ - Each app: own IO proc callback│
│ - Lock-free ring buffer transfer│
└─────────────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│ Audio Mixer (combines all apps) │
└─────────────────────────────────┘
       │
       ▼
┌─────────────────────────────────┐
│ Master EQ → Soft Limiter        │
└─────────────────────────────────┘
       │
       ▼
    Speakers (via cpal)
```

Each application gets its own independent EQ instance, processed **before** mixing. This is true per-app EQ, not an approximation.

## Development

```bash
# Run all tests
cargo test --workspace

# Lint Rust code
cargo clippy --workspace

# Type-check TypeScript
pnpm build

# Run the app in development
pnpm tauri dev

# View debug logs (macOS)
tail -f ~/gecko-debug.log
```

### Project Structure

```
gecko/
├── crates/
│   ├── gecko_core/      # Audio engine, device management
│   ├── gecko_dsp/       # Signal processing (EQ, FFT, limiter)
│   └── gecko_platform/  # OS-specific backends
│       ├── linux/       # PipeWire implementation
│       └── macos/       # Process Tap implementation
├── src/                 # React frontend
├── src-tauri/           # Tauri application shell
└── docs/                # Architecture documentation
```

### macOS Implementation Details

The macOS backend uses Apple's Process Tap API (`AudioHardwareCreateProcessTap`) introduced in macOS 14.4:

| File | Purpose |
|------|---------|
| `macos/mod.rs` | CoreAudioBackend, trait implementation |
| `macos/coreaudio.rs` | Device enumeration, app discovery |
| `macos/process_tap.rs` | ProcessTapCapture with IO proc callbacks |
| `macos/process_tap_ffi.rs` | Raw FFI bindings to CoreAudio |
| `macos/tap_description.rs` | CATapDescription Objective-C bindings |
| `macos/audio_output.rs` | AudioMixer + cpal output stream |
| `macos/permissions.rs` | Screen Recording / Microphone permission handling |

## Troubleshooting

### macOS: "Permission Denied" when capturing

1. Open **System Settings** → **Privacy & Security** → **Screen Recording**
2. Enable Gecko in the list
3. If Gecko doesn't appear, try launching it once and grant permission when prompted
4. **Restart Gecko** after granting permission (required by macOS)

### macOS: App shows as "Protected"

Safari, FaceTime, Messages, and system sounds are protected by Apple's security sandbox. These apps cannot be captured. They will still receive Master EQ if you enable it globally.

### macOS: No apps appearing in stream list

1. Make sure apps are actually playing audio
2. Check that you have granted Screen Recording permission
3. Try clicking "Refresh" to rescan running applications

### Linux: No sound after routing

1. Check that PipeWire is running: `systemctl --user status pipewire`
2. Verify Gecko Audio sink exists: `pw-cli ls Node | grep Gecko`
3. Check application is routed to Gecko: Use `pavucontrol` or system settings

## Contributing

Contributions are welcome! Please read:
- `AGENT.md` - Coding conventions and patterns
- `docs/ai-knowledge/` - Architecture documentation

Key rules:
- Zero allocations in audio callbacks
- Use `gecko-*` Tailwind tokens for styling
- Add tests for new Rust code
- Run `cargo clippy --workspace` before committing

## License

GPL-3.0 - See [LICENSE](LICENSE) for details.

## Acknowledgments

Built with:
- [Tauri](https://tauri.app/) - Desktop application framework
- [PipeWire](https://pipewire.org/) - Modern Linux audio system
- [CoreAudio](https://developer.apple.com/documentation/coreaudio) - macOS audio framework
- [biquad](https://crates.io/crates/biquad) - Digital filter design
- [rtrb](https://crates.io/crates/rtrb) - Real-time ring buffer
- [cpal](https://crates.io/crates/cpal) - Cross-platform audio I/O

## References

- [Apple Process Tap Documentation](https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps)
- [AudioCap Sample Implementation](https://github.com/insidegui/AudioCap)
- [PipeWire Documentation](https://docs.pipewire.org/)
