# CLAUDE.md

## FIRST: Verify Context is Loaded

If you have NOT yet read `AGENT.md` this session, read it NOW before proceeding.
If you have NOT yet scanned `docs/ai-knowledge/README.md` for relevant topics, do so NOW.

These files contain critical project conventions. Proceeding without them leads to incorrect implementations.

## DURING SESSION

- Load KB docs when task keywords match the README index
- On EVERY user message, check triggers: commit? correction? session end? → run matching command
- When context appears lost (unable to recall project conventions), re-read AGENT.md

---

## Quick Reference (Summary Only - Full Details in AGENT.md)

- **Stack**: Rust 2021, Tauri v2, React 18, TypeScript
- **Main branch**: `main`
- **Testing**: `cargo test --workspace`
- **Linting**: `cargo clippy --workspace`, `pnpm lint`

## Critical Constraints

- **Real-time audio** - ZERO allocations in audio callbacks
- **NO microphone input** - Gecko captures APPLICATION audio, not voice
- **Per-app EQ** - Each app has independent EQ processing BEFORE mixing
- **Frontend tokens** - Use `gecko-*` Tailwind tokens, not raw colors

## Knowledge Base Triggers

If your task involves ANY of these topics, you MUST load the corresponding doc:

| Keywords | Load This Document |
|----------|-------------------|
| audio, DSP, pipeline, buffer | `architecture/audio-pipeline.md` |
| callback, real-time, lock-free | `architecture/realtime-rules.md` |
| Linux, PipeWire, virtual sink | `architecture/platform-linux.md` |
| macOS, CoreAudio, Process Tap | `architecture/platform-macos.md` |
| Windows, WASAPI, loopback | `architecture/platform-windows.md` |
| frontend, React, component, CVA | `features/frontend-patterns.md` |
| EQ, filter, BiQuad, parametric | `features/eq-implementation.md` |

**When in doubt, check `docs/ai-knowledge/README.md` for the full index.**

## Mandatory Auto-Triggers (Do NOT Skip)

See `docs/ai-commands/TRIGGER-CHECKLIST.md` for quick reference. Key mandatory behaviors:

| Trigger | Action |
|---------|--------|
| User says "commit", "stage", "git add" | Run `pre-commit-check` BEFORE the operation |
| KB lookup failed → solved via code search | Run `document-solution` to update KB |
| User corrects you ("that's wrong", "actually...", etc.) | Run `log-mistake` immediately |
| Modified any KB file | Run `check-kb-index` |
| User says "done", "thanks", "bye", ending session | Run `session-end-checklist` |

**Do NOT wait for explicit requests - detect and trigger proactively.**
