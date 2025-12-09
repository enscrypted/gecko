# Real-Time Audio Safety Rules

**Last Updated**: December 2024
**Context**: MUST READ when writing any code that executes in audio callbacks

## The Golden Rules

Code in the audio callback path MUST follow these rules. Violating them causes audio dropouts ("glitches").

### 1. NO Heap Allocations

```rust
// FORBIDDEN in audio callback
let v = Vec::new();           // Allocates
v.push(sample);               // May reallocate
let s = format!("{}", val);   // Allocates String
let b = Box::new(data);       // Heap allocation
String::from("text");         // Allocates

// ALLOWED
let mut buffer: [f32; 512] = [0.0; 512];  // Stack allocation
buffer[i] = sample;                        // No allocation
```

### 2. NO System Calls

```rust
// FORBIDDEN in audio callback
println!("debug");            // I/O syscall
std::fs::read("file");        // File I/O
std::thread::sleep(...);      // Sleep syscall
std::time::Instant::now();    // Clock syscall (on some systems)

// ALLOWED
// Just... don't do I/O in the audio thread
```

### 3. NO Blocking Operations

```rust
// FORBIDDEN in audio callback
let guard = mutex.lock();              // Can block
let data = channel.recv();             // Blocks until data
let result = channel.recv_timeout();   // Can still block

// ALLOWED
if let Ok(cmd) = channel.try_recv() {  // Never blocks
    // process command
}
// Atomics
let val = atomic.load(Ordering::SeqCst);
atomic.store(new_val, Ordering::SeqCst);
```

### 4. NO Unbounded Loops

```rust
// FORBIDDEN
while some_condition() {
    // Unknown iteration count
}

// ALLOWED
for i in 0..buffer_size {  // Bounded by known size
    process(buffer[i]);
}

for sample in buffer.iter_mut() {  // Bounded by slice length
    *sample = process(*sample);
}
```

### 5. O(n) Time Complexity

Where n = buffer size (typically 128-1024 samples)

```rust
// ALLOWED: O(n) - linear in buffer size
for sample in buffer.iter_mut() {
    for filter in &mut self.filters {  // Fixed count (10 bands)
        *sample = filter.run(*sample);
    }
}

// FORBIDDEN: O(n²) or worse
for i in 0..buffer.len() {
    for j in 0..buffer.len() {
        // Quadratic - will miss deadline
    }
}
```

## Safe Patterns

### Lock-Free Parameter Updates

```rust
// UI Thread: Send new value
command_sender.send(Command::SetGain(0.5)).ok();

// Audio Thread: Check for updates (non-blocking)
while let Ok(cmd) = command_receiver.try_recv() {
    match cmd {
        Command::SetGain(g) => self.gain = g,
        // ...
    }
}
```

### Atomic State

```rust
use std::sync::atomic::{AtomicBool, Ordering};

// Shared between threads
let bypassed = Arc::new(AtomicBool::new(false));

// UI Thread
bypassed.store(true, Ordering::SeqCst);

// Audio Thread
if bypassed.load(Ordering::SeqCst) {
    return input; // Bypass processing
}
```

### Pre-Allocated Buffers

```rust
pub struct Processor {
    // Pre-allocate during construction
    temp_buffer: Vec<f32>,
}

impl Processor {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            temp_buffer: vec![0.0; max_buffer_size],
        }
    }

    pub fn process(&mut self, buffer: &mut [f32]) {
        // Use pre-allocated buffer, no allocation here
        self.temp_buffer[..buffer.len()].copy_from_slice(buffer);
    }
}
```

## The AudioProcessor Trait

All DSP processors must implement this trait:

```rust
pub trait AudioProcessor: Send {
    /// Process audio buffer in-place
    /// MUST follow all real-time safety rules
    fn process(&mut self, buffer: &mut [f32], context: &ProcessContext);

    /// Reset internal state (delay lines, etc.)
    fn reset(&mut self);

    /// Human-readable name
    fn name(&self) -> &'static str;

    /// Whether enabled (can be bypassed)
    fn is_enabled(&self) -> bool { true }
}
```

## Deadline Calculation

At 48kHz sample rate with 512-sample buffer:
- Buffer duration: 512 / 48000 = **10.67ms**
- Your `process()` function MUST complete in **< 10ms** (leave margin)
- Target: **< 1% CPU** on a single core

## Testing Real-Time Safety

While Rust can't enforce these rules at compile time, you can:

1. **Code review**: Look for forbidden patterns
2. **Profiling**: Measure actual CPU usage
3. **Stress testing**: Run with small buffers (128 samples)
4. **Debug builds**: Use `#[cfg(debug_assertions)]` to add checks

```rust
#[cfg(debug_assertions)]
fn assert_realtime_safe() {
    // In debug builds, track allocations
    // and panic if allocation occurs in audio thread
}
```

## Related Files

- `crates/gecko_dsp/src/processor.rs` - AudioProcessor trait definition
- `crates/gecko_dsp/src/eq.rs` - Example real-time safe implementation
- `crates/gecko_core/src/stream.rs` - Audio callback implementation
