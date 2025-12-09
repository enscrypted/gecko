# Linux Implementation Strategy: PipeWire & Graph

## 1. The PipeWire Environment

Gecko targets modern Linux distributions utilizing **PipeWire** (Fedora, Ubuntu 22.10+, Arch).

### 1.1 Dependencies

- **Crate**: `pipewire` (Rust bindings)
- **System Libs**: `libpipewire-0.3`, `libspa-0.2`

## 2. Virtual Sink Creation

Gecko creates virtual devices **programmatically at runtime**, requiring no permanent system configuration changes.

### 2.1 The Code Logic

```rust
let core = context.connect(None)?;
let proxy = core.create_object("adapter", &properties! {
    "factory.name" => "support.null-audio-sink",
    "node.name" => "Gecko-Virtual-Input",
    "media.class" => "Audio/Sink",
    "audio.channels" => "2",
    "audio.rate" => "48000",
    "object.linger" => "false"  // Device vanishes when Gecko closes
});
```

This creates a device that other apps can select as their output.

### 2.2 Visibility

Once loaded, this sink immediately appears in the system's volume control (GNOME/KDE), allowing the user (or Gecko programmatically) to route applications to it.

## 3. The "Patchbay" Logic (App Capture)

To capture a specific app (e.g., Firefox), Gecko acts as a **Link Manager**.

### 3.1 Discovery

1. Monitor the `Global` registry
2. Filter for `Node` objects where `application.name == "Firefox"`
3. Retrieve the `Port` objects associated with that Node (Output Ports)

### 3.2 Linking

```rust
core.create_object("link-factory", &properties! {
    "link.output.port" => output_port_id.to_string(),
    "link.input.port" => gecko_input_port_id.to_string(),
    "link.passive" => "true",
});
```

- `link.passive = true`: Don't force the source app to wake up if Gecko is the only consumer

## 4. Audio Transport

- Gecko uses `cpal` with the `jack` or `pipewire` host trait for actual audio data streaming
- The graph manipulation (Linking) is handled by the `pipewire` crate
- The data callback is handled by `cpal`

## 5. Registry Monitoring

Gecko registers a listener to detect:
- When new applications start (new Nodes appear)
- When applications close (Nodes removed)
- When devices are hot-plugged

This enables features like **"Auto-capture new instances of Firefox"**.

## 6. Permissions

PipeWire runs in user-space. No special permissions needed for:
- Creating virtual sinks
- Linking nodes
- Capturing application audio

## 7. Implementation Checklist

- [x] `pipewire-rs` crate integration
- [ ] Virtual sink creation via `null-audio-sink`
- [ ] Registry listener for node/port discovery
- [ ] Link manager for per-app routing
- [ ] Auto-reconnect logic for persistent routing rules
- [ ] Desktop integration (system tray indicator)

## 8. Testing on Linux

```bash
# Check PipeWire is running
systemctl --user status pipewire

# List all nodes
pw-cli list-objects Node

# List all links
pw-cli list-objects Link

# Monitor events
pw-mon
```

## 9. Why Linux is the Best Platform for Gecko

| Feature | Linux (PipeWire) | Windows (WASAPI) | macOS (CoreAudio) |
|---------|------------------|------------------|-------------------|
| Virtual Devices | Runtime creation | Kernel driver | HAL plugin |
| Per-App Capture | Native (linking) | API (Win10 20348+) | Not available |
| Permissions | User-space | User-space | Root for driver |
| Complexity | Low | Medium | High |
