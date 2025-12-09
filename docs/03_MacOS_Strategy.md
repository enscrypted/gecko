# macOS Implementation Strategy: HAL Plug-In & IPC

## 1. The Virtual Device Driver (HAL)

macOS requires an **AudioServerPlugIn** to create virtual audio devices.

### 1.1 Architecture

- **Type**: User-Space Driver (running in `coreaudiod`)
- **Base**: C++ implementation based on AudioServerPlugIn template or libASPL
- **Bundle ID**: `com.gecko.driver.AudioDevice`
- **Location**: `/Library/Audio/Plug-Ins/HAL/GeckoAudioDevice.driver`

### 1.2 Functionality

The driver exposes a device named **"Gecko Sink"**.

**I/O Procedure**: When `coreaudiod` calls `IOOperation`, the driver:
1. Calculates the current cycle time
2. Writes the incoming audio buffer to a **Shared Memory Ring Buffer**
3. Updates the atomic write-head index

## 2. Inter-Process Communication (IPC)

Because the Driver and the App run in separate processes, **Shared Memory (SHM)** is the transport.

### 2.1 Memory Layout

```c
struct GeckoShmHeader {
    atomic_uint32_t write_head;
    atomic_uint32_t read_head;
    uint32_t buffer_size;
    uint32_t channels;
    uint32_t sample_rate;
};
// Followed by raw float data
```

### 2.2 Synchronization

- **Lock-Free**: No mutexes are used across the process boundary (risk of deadlocking `coreaudiod`)
- **Atomic Operations**: Standard C++11 / Rust `std::sync::atomic` are used

## 3. The Gecko Application (Backend)

### 3.1 Connection Logic

1. Gecko App starts
2. It calls `shm_open("/GeckoAudioShm", O_RDONLY)`
3. It `mmap`s the region
4. It wraps this memory in a Rust `AudioSource` struct implementing the `Iterator` trait
5. This iterator feeds the DSP pipeline

## 4. Deployment & Installation

### Privileges
Installation requires **root**.

### Lifecycle
- **Install**: Copy bundle → `chown root:wheel` → Kickstart `coreaudiod`
- **Uninstall**: Remove bundle → Kickstart `coreaudiod`

### Tauri Integration
The installer script (pkg) must handle these steps. The App itself cannot install the driver at runtime without prompting for an admin password via `osascript`.

## 5. Alternative: Use Existing Drivers

For users who prefer not to install a custom driver, Gecko supports:

- **BlackHole** (free, open-source)
- **Loopback** by Rogue Amoeba (commercial)

These appear as standard audio devices that Gecko can use via CPAL.

## 6. Limitations

- **No Per-Application Capture**: macOS doesn't expose per-app audio APIs
- **User Must Route Manually**: Users select "Gecko Sink" as output in app preferences or System Preferences

## 7. Implementation Checklist

- [ ] Fork BlackHole or libASPL for HAL plugin base
- [ ] Implement shared memory ring buffer in driver
- [ ] Implement Rust wrapper for `shm_open`/`mmap`
- [ ] Create macOS installer package (.pkg)
- [ ] Test on macOS 12+ (Monterey+)
- [ ] Document driver installation for users
