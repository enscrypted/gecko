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

| Platform | Status | Audio Backend |
|----------|--------|---------------|
| Linux | Fully Implemented | PipeWire |
| Windows | Planned | WASAPI Process Loopback |
| macOS | Planned | CoreAudio HAL Plugin |

## Requirements

### Linux

- PipeWire 0.3+ (default on Fedora, Ubuntu 22.10+, Arch)
- WirePlumber (session manager)
- Node.js 18+ and pnpm
- Rust 1.70+

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

## Usage

1. **Start Gecko**: Launch the application
2. **Route Audio**: In your system sound settings, change application output to "Gecko Audio"
3. **Adjust EQ**: Use the per-app sliders or apply a preset
4. **System Volume**: Use your volume keys - they control Gecko seamlessly

## Architecture

```
App Audio (Firefox, Spotify, etc.)
       |
       v
Gecko Virtual Sink ("Gecko Audio")
       |
       v
Per-App EQ Processing --> Per-App Volume --> Mixer
       |
       v
Master EQ --> Soft Limiter --> Speakers
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
```

### Project Structure

```
gecko/
├── crates/
│   ├── gecko_core/      # Audio engine, device management
│   ├── gecko_dsp/       # Signal processing (EQ, FFT, limiter)
│   └── gecko_platform/  # OS-specific backends (PipeWire)
├── src/                 # React frontend
├── src-tauri/           # Tauri application shell
└── docs/                # Architecture documentation
```

## Contributing

Contributions are welcome! Please read the architecture docs in `docs/` and follow the patterns in `AGENT.md`.

## License

GPL-3.0 - See [LICENSE](LICENSE) for details.

## Acknowledgments

Built with:
- [Tauri](https://tauri.app/) - Desktop application framework
- [PipeWire](https://pipewire.org/) - Modern Linux audio system
- [biquad](https://crates.io/crates/biquad) - Digital filter design
- [rtrb](https://crates.io/crates/rtrb) - Real-time ring buffer
