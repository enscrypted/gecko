# CLAUDE.md

**STOP. YOU MUST READ THIS BEFORE DOING ANYTHING.**

This is NOT optional. Before responding to ANY user request, you MUST:

1. **READ AGENT.md** (repository root) - Contains ALL project conventions, patterns, and constraints. This is MANDATORY.

2. **READ docs/ai-knowledge/README.md** - Knowledge base index. Load relevant docs based on task.

3. **READ docs/ai-commands/README.md** - Available agent commands. Execute automatically when triggers match.

## Why This Matters

This is a real-time audio application. Violating conventions (like allocating in audio callbacks) causes **audible glitches**. The patterns in AGENT.md exist for critical technical reasons.

## Non-Negotiable Rules

- **Rust audio code**: ZERO allocations in `process()` functions
- **Frontend**: Use `gecko-*` Tailwind tokens, NOT raw colors
- **Testing**: All new Rust code needs tests
- **Comments**: Explain Rust patterns (user is learning)

## Quick Command Reference

```bash
cargo test --workspace    # Run all tests (110+ tests)
pnpm tauri dev           # Run full app
cargo clippy --workspace # Lint before commit
```

## Do NOT Skip Reading AGENT.md

The 2 minutes spent reading AGENT.md will save hours of back-and-forth corrections. The user has explicitly requested that agents follow these conventions strictly.

**NOW GO READ AGENT.md BEFORE PROCEEDING.**
