# AGENT.md - AI Agent Instructions

Tool-specific configs (`.cursor/rules/`, `.windsurfrules`, `.github/copilot-instructions.md`, `.agent/rules/`, `CLAUDE.md`) reference this file.

---

## QUICK TRIGGERS (Memorize These)

**Session Start**: Check `docs/ai-knowledge/ai-patterns/mistake-log.md` for patterns to avoid.

**During Session**:
- User says "commit/stage" → run `pre-commit-check`
- You get corrected → run `log-mistake`
- KB lookup fails → after solving, run `document-solution`
- You modify KB files → run `check-kb-index`

**Session End** (user says "done/thanks/bye"): Run `session-end-checklist`

---

## Project Context

Gecko is a high-performance, cross-platform audio routing and processing application. It intercepts audio from specific applications or devices, processes it via a DSP pipeline (10-band parametric EQ), and routes it to virtual or physical outputs.

### ⚠️ CRITICAL: Audio Architecture (DO NOT USE MICROPHONE) ⚠️

**Gecko captures APPLICATION AUDIO, NOT microphone input!**

```
CORRECT:  App Audio (Firefox, Spotify) → Virtual Sink → Gecko DSP → Speakers
WRONG:    Microphone → Gecko DSP → Speakers  ← NEVER DO THIS!
```

Platform-specific capture methods:
- **Linux**: Create PipeWire virtual sink → Apps route to it → Capture from monitor port
- **Windows**: WASAPI Process Loopback API → Capture specific app audio
- **macOS 14.4+**: Process Tap API → Per-app audio capture via `AudioHardwareCreateProcessTap`

**Never use `host.default_input_device()` for audio capture** - that grabs the microphone and causes feedback loops. This is a **system audio processor**, not a voice application.

### ⚠️ CRITICAL: Per-App EQ is the CORE MVP FEATURE ⚠️

**TRUE per-application EQ is Gecko's key differentiator. This is NON-NEGOTIABLE.**

Each application MUST have its own independent EQ processing BEFORE audio is mixed:

```
CORRECT Architecture (TRUE Per-App EQ):
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  Firefox    │     │   Spotify   │     │   Discord   │
│   Audio     │     │    Audio    │     │    Audio    │
└──────┬──────┘     └──────┬──────┘     └──────┬──────┘
       │                   │                   │
       ▼                   ▼                   ▼
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│ Firefox EQ  │     │ Spotify EQ  │     │ Discord EQ  │
│ (separate)  │     │ (separate)  │     │ (separate)  │
└──────┬──────┘     └──────┬──────┘     └──────┬──────┘
       │                   │                   │
       └───────────────────┼───────────────────┘
                           ▼
                    ┌─────────────┐
                    │  Master EQ  │
                    │  (combined) │
                    └──────┬──────┘
                           ▼
                       Speakers

WRONG Architecture (Additive Approximation - DO NOT USE):
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  Firefox    │     │   Spotify   │     │   Discord   │
└──────┬──────┘     └──────┬──────┘     └──────┬──────┘
       │                   │                   │
       └───────────────────┼───────────────────┘
                           ▼
                  ┌─────────────────┐
                  │  Single Mixed   │  ← Audio already mixed!
                  │     Stream      │    Can't EQ individually!
                  └────────┬────────┘
                           ▼
                    ┌─────────────┐
                    │  Single EQ  │  ← "Additive offsets" is a LIE
                    └─────────────┘
```

**Platform Implementation Requirements**:
- **Linux (PipeWire)**: Create SEPARATE virtual sink per app, capture each independently
- **Windows (WASAPI)**: Process Loopback API already captures per-process
- **macOS 14.4+ (CoreAudio)**: Process Tap API captures per-app via `CATapDescription`

**DO NOT implement shortcuts or approximations.** If an agent suggests "additive EQ offsets"
or "combined gains", they are trying to avoid the real implementation. Reject this approach.

### Key Characteristics
- **Hybrid-Native Core Architecture**: Rust backend (gecko_core) handles all audio I/O and DSP; Tauri frontend is a thin visualization layer
- **Real-Time Audio Processing**: Zero-allocation audio callbacks, lock-free thread communication
- **Cross-Platform**: Linux (PipeWire), Windows (WASAPI), macOS (CoreAudio HAL)
- **Memory Safe**: Rust's ownership model prevents data races in concurrent audio threads
- **Educational Codebase**: Comments explain Rust-specific patterns for learning

### Tech Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| Language | Rust 2021 | Core audio engine, DSP, platform backends |
| GUI Framework | Tauri v2 | Lightweight cross-platform desktop app |
| Frontend | React 18 + TypeScript | UI components and state |
| Styling | Tailwind CSS + CVA | Design system with variants |
| Audio I/O | CPAL v0.16 | Cross-platform audio transport |
| DSP | biquad v0.4 | BiQuad filters (no_std, zero-allocation) |
| IPC | rtrb, crossbeam-channel | Lock-free audio thread communication |
| Windows | windows-rs | WASAPI Process Loopback |
| Linux | pipewire-rs | Graph manipulation, virtual sinks |
| macOS | coreaudio-rs, objc2 | Process Tap API (macOS 14.4+) |

### Development Commands

```bash
# Frontend development
pnpm dev              # Vite dev server
pnpm build            # TypeScript + Vite production build
pnpm test             # Vitest tests
pnpm lint             # ESLint

# Full application
pnpm tauri dev        # Run app with hot reload

# Rust backend
cargo test --workspace           # All 110+ tests across 4 crates
cargo test -p gecko_dsp          # Single crate tests
cargo test -- --ignored          # Hardware-dependent tests
cargo bench -p gecko_dsp         # DSP benchmarks
cargo build --release            # Optimized build

# Before commits
cargo clippy --workspace         # Lint Rust code
cargo fmt --all                  # Format Rust code
```

---

## Knowledge Base Instructions

### Location
Detailed knowledge lives in `/docs/ai-knowledge/`. Consult the index at `docs/ai-knowledge/README.md`.

### When to Load Knowledge Docs
Load ONLY documents relevant to your current task. Do not load the entire knowledge base.

| Task Type | Load These Docs |
|-----------|-----------------|
| DSP/Audio work | `architecture/audio-pipeline.md` |
| Platform-specific | `architecture/platform-*.md` |
| Frontend components | `features/frontend-patterns.md` |
| Real-time safety | `architecture/realtime-rules.md` |

---

## Coding Standards & Conventions

### Rust Backend Patterns

#### Error Handling
```rust
// Each crate defines its own error type using thiserror
#[derive(Error, Debug)]
pub enum DspError {
    #[error("Invalid band index: {0} (must be 0-9)")]
    InvalidBandIndex(usize),

    #[error("Invalid filter coefficients for frequency {frequency}Hz at sample rate {sample_rate}Hz")]
    InvalidCoefficients { frequency: f32, sample_rate: f32 },
}

// Result type alias for ergonomics
pub type DspResult<T> = Result<T, DspError>;
```

#### Module Documentation
```rust
//! Module-level doc comment (//!) at top of file
//! Describes the module's purpose and architecture

/// Item-level doc comment (///) for public functions/structs
/// Include # Examples section for complex APIs
pub fn public_function() {}
```

#### Real-Time Safety Rules (CRITICAL)
The `process()` function in audio callbacks MUST follow:
- **NO heap allocations** (`Vec::push`, `Box::new`, `String`, `format!`)
- **NO syscalls** (file I/O, network, `println!`)
- **NO blocking** (`Mutex::lock`, channels that block)
- **NO unbounded loops**
- **O(n) complexity** where n = buffer size

```rust
// GOOD: Use try_recv() which never blocks
while let Ok(cmd) = command_receiver.try_recv() {
    // process command
}

// BAD: recv() can block the audio thread
let cmd = command_receiver.recv(); // NEVER in audio callback
```

#### Testing Patterns
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_functionality() {
        // Tests that don't require hardware
    }

    #[test]
    #[ignore = "requires audio hardware"]
    fn test_with_hardware() {
        // Hardware-dependent tests, run with: cargo test -- --ignored
    }
}
```

#### Educational Comments
Add comments explaining Rust-specific patterns (the user is learning Rust):
```rust
// Rust pattern: `?` operator propagates errors up the call stack
// This is idiomatic Rust error handling - no exceptions, explicit Result types
let result = fallible_operation()?;

// Rust pattern: `core::array::from_fn` creates array by calling closure with each index
let filters = core::array::from_fn(|i| create_filter(i));
```

### Frontend Patterns

#### Component Structure with CVA
```tsx
// Base components in src/components/ui/ use CVA for variants
const buttonVariants = cva(
  // Base styles (array for readability)
  ["inline-flex items-center justify-center", "rounded font-medium"],
  {
    variants: {
      variant: {
        default: ["bg-gecko-bg-tertiary", "text-gecko-text-primary"],
        primary: ["bg-gecko-accent", "text-gecko-bg-primary"],
      },
      size: {
        sm: "h-8 px-3 text-xs",
        md: "h-9 px-4",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "md",
    },
  }
);

// Always use forwardRef for base components
export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, ...props }, ref) => (
    <button
      className={cn(buttonVariants({ variant, size, className }))}
      ref={ref}
      {...props}
    />
  )
);
Button.displayName = "Button";
```

#### Design Tokens (Tailwind)
All colors use the `gecko-*` namespace defined in `tailwind.config.js`:
```tsx
// GOOD: Use design tokens
<div className="bg-gecko-bg-primary text-gecko-text-primary border-gecko-border">

// BAD: Don't use raw colors
<div className="bg-gray-900 text-white border-gray-700">
```

#### Performance Patterns
```tsx
// Memoize components that receive stable props
const EqBand = memo(function EqBand({ band, onGainChange }) {
  // useCallback for handlers passed to children
  const handleChange = useCallback((e) => {
    onGainChange(band.index, parseFloat(e.target.value));
  }, [band.index, onGainChange]);

  return <Slider onChange={handleChange} />;
});

// Optimistic updates for responsiveness
const handleBandChange = useCallback(async (index, gainDb) => {
  // Update UI immediately
  setBands(prev => prev.map(b => b.index === index ? {...b, gain_db: gainDb} : b));

  // Then sync with backend
  await invoke("set_band_gain", { band: index, gainDb });
}, []);
```

### File/Folder Patterns

| Pattern | Location | Purpose |
|---------|----------|---------|
| `crates/gecko_*/` | Rust workspace crates | Modular backend components |
| `crates/*/src/error.rs` | Each crate | Crate-specific error types |
| `src/components/ui/` | Frontend | Reusable base components (CVA) |
| `src/components/*.tsx` | Frontend | Feature components |
| `src/lib/utils.ts` | Frontend | Shared utilities (cn helper) |
| `src-tauri/src/commands.rs` | Tauri | IPC command handlers |
| `docs/*.md` | Project docs | Architecture specifications |

---

## Infrastructure Constraints

### Audio Thread Constraints
- Buffer callback deadline: ~2.6ms at 48kHz stereo
- DSP CPU target: < 1% single core
- Latency target: < 10ms round-trip
- Memory target: < 50MB resident

### Platform-Specific Requirements
| Platform | Min Version | Notes |
|----------|-------------|-------|
| Linux | PipeWire 0.3+ | Most flexible, runtime virtual devices |
| Windows | 10 Build 20348+ | Per-app capture requires newer API |
| macOS | 14.4+ (Sonoma) | Process Tap API, requires Screen Recording permission |

---

## Things to Avoid

### Rust Backend
- **Using `default_input_device()` for audio capture** - this grabs the MICROPHONE and causes feedback. Gecko captures APPLICATION audio via platform backends (PipeWire/WASAPI/CoreAudio)
- `std::sync::Mutex` in audio callbacks (use atomics or lock-free channels)
- Allocations in `process()` functions (pre-allocate buffers)
- `println!` or logging in audio thread (use tracing with appropriate levels)
- Blocking channel operations in real-time code
- `unwrap()` in production code (use `expect()` with context or propagate errors)

### Frontend
- Raw Tailwind colors (use `gecko-*` tokens)
- Inline styles (use Tailwind classes)
- Anonymous functions in render (use `useCallback`)
- Missing `key` props in lists
- Direct DOM manipulation (use React state)

### General
- Creating documentation files unless explicitly requested
- Over-engineering - only make directly requested changes
- Adding features beyond what was asked
- Breaking existing tests
- Ignoring established patterns in the codebase
- Using `format!` or string concatenation in hot paths

---

## Testing

### Framework & Commands
- **Rust**: Built-in `cargo test` with per-crate organization
- **Frontend**: Vitest (not yet fully configured)

### Testing Requirements
- Write tests for new Rust code (inline `#[cfg(test)]` modules)
- Hardware-dependent tests use `#[ignore = "reason"]`
- Bug fixes should include regression tests
- Run `cargo test --workspace` before committing
- Never commit code that breaks existing tests

### Test Organization
```
crates/gecko_dsp/src/eq.rs      # 22 tests - EQ filters
crates/gecko_core/src/          # 25 tests - Engine, devices, config
crates/gecko_platform/src/      # 8 tests - Platform backends
src-tauri/src/lib.rs            # 3 tests - Tauri commands
```

---

## Code Comments

### When to Comment
- Complex algorithms that aren't self-explanatory
- Non-obvious business logic with specific requirements
- Workarounds with context on why they're needed
- **Rust-specific patterns** (for educational purposes - user is learning Rust)
- Real-time safety justifications

### Comment Style
```rust
// Rust pattern: explanation of the Rust-specific concept
// This helps the reader understand idiomatic Rust

// SAFETY: explanation of why unsafe block is sound
unsafe { ... }

// NOTE: important caveat or consideration

// TODO: future work with context
```

### When NOT to Comment
- Self-explanatory code
- Obvious method names
- Every method or class (avoid boilerplate)
- Obvious type conversions

---

## Git Workflow

- **Main branch**: `main`
- **Commit style**: Conventional commits (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`)
- **Pre-commit**: Run `cargo test --workspace && cargo clippy --workspace`

---

## Agent Self-Documentation Rules

### When to Update AGENT.md
If you discover a critical project-wide pattern or constraint NOT already documented, add it to the appropriate section.

### When to Create New Knowledge Docs
Create a new doc in `/docs/ai-knowledge/` when:
- You've done significant codebase analysis others would benefit from
- A feature/system is complex enough to warrant dedicated documentation
- You've resolved a non-obvious bug with root cause worth preserving

### When NOT to Document
- One-off debugging sessions
- Ticket-specific implementation details
- Obvious language/framework conventions

---

## Mandatory Auto-Triggers

These MUST fire automatically. Do not wait for user to ask:

| Trigger | Action | Why |
|---------|--------|-----|
| User says "commit", "stage", "git add" | `pre-commit-check` | Catch violations |
| KB lookup fails → solved via code | `document-solution` | Update KB |
| User corrects you | `log-mistake` | Build patterns |
| Modified KB files | `check-kb-index` | Keep index current |
| User ending session | `session-end-checklist` | Catch missed automation |

### Trigger Detection Examples

```
USER: "let's commit these changes"
→ Detected "commit" → run pre-commit-check BEFORE git operations

USER: "no that's wrong, you need to use gecko-* tokens"
→ Detected correction phrase → run log-mistake

USER: "thanks, that's all for today"
→ Detected session end → run session-end-checklist
```

**Correction phrases** (run `log-mistake` immediately):
"that's wrong", "actually...", "no, it should be...", "you forgot to...", "you missed..."

Do NOT wait for explicit requests - detect and trigger proactively.

---

## Command Library

Commands: `docs/ai-commands/`

| Command | When |
|---------|------|
| `pre-commit-check` | Before commit/staging |
| `session-end-checklist` | Session ending |
| `log-mistake` | User corrects you |
| `document-solution` | Complex problem solved or KB miss |
| `check-kb-index` | After KB file changes |
| `save-session` | Long session (20+ turns) |

*Tables are summaries. Source of truth is individual files in `docs/ai-commands/`.*

---

## Self-Documentation Rules

### Update KB when:
- KB lookup failed but you solved via code search → run `document-solution`
- Complex solution (3+ files, 5+ exchanges) → run `document-solution`
- After any KB change → run `check-kb-index`

### Format:
- Kebab-case filenames
- Include "Last Updated" date
- Include "Context" section

### Mistake Pattern Learning
When corrected, log the pattern to `docs/ai-knowledge/ai-patterns/mistake-log.md`. If a pattern appears 3+ times, add it to the "Things to Avoid" section above.

---

## Quick Reference

| Task | Command |
|------|---------|
| Run all tests | `cargo test --workspace` |
| Run single crate tests | `cargo test -p gecko_dsp` |
| Run app in dev mode | `pnpm tauri dev` |
| Build production | `pnpm tauri build` |
| Lint Rust | `cargo clippy --workspace` |
| Format Rust | `cargo fmt --all` |
| Check types (TS) | `pnpm build` |

---

## Architecture Quick Reference

```
┌─────────────────────────────────────────────────────────────┐
│                        UI Thread                            │
│  (Tauri/React) ──commands──▶ Engine ◀──events── (Tauri)    │
└─────────────────────────────────────────────────────────────┘
                             │ crossbeam-channel (try_recv)
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                      Audio Thread                           │
│   Capture ──rtrb──▶ DSP Chain ──rtrb──▶ Output             │
│     │                   │                  │                │
│     └───────────────────┴──────────────────┘                │
│              (Zero allocation in this path)                 │
└─────────────────────────────────────────────────────────────┘
```

### Crate Responsibilities
- **gecko_core**: Audio engine, device management, thread coordination
- **gecko_dsp**: Signal processing (EQ, future: compressor, limiter)
- **gecko_platform**: OS-specific audio routing (PipeWire, WASAPI, CoreAudio)
- **src-tauri**: Tauri commands, app state, IPC bridge

---

## REMEMBER (End of File)

Before responding, check:
1. **Am I being corrected?** → Run `log-mistake`
2. **Is user committing/staging?** → Run `pre-commit-check`
3. **Is user ending session?** → Run `session-end-checklist`
4. **Did I solve something complex without KB?** → Run `document-solution`

Detect and execute automatically.
