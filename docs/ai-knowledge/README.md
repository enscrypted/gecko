# AI Knowledge Base

## Purpose

This folder contains documented knowledge to reduce token usage and improve agent accuracy. Consult the index below and load ONLY documents relevant to your current task.

## How Agents Should Use This

1. Read this README first
2. Identify which documents match your task
3. Load only those documents
4. If you discover undocumented critical patterns during development, update AGENT.md

## Index

### Architecture

| Document | When to Read |
|----------|--------------|
| [audio-pipeline.md](architecture/audio-pipeline.md) | Working on audio engine, DSP, or thread model |
| [realtime-rules.md](architecture/realtime-rules.md) | Writing code that runs in audio callbacks |
| [platform-linux.md](architecture/platform-linux.md) | Working on PipeWire/Linux support (✅ FULLY IMPLEMENTED) |
| [platform-windows.md](architecture/platform-windows.md) | Working on WASAPI/Windows support (stub) |
| [platform-macos.md](architecture/platform-macos.md) | Working on CoreAudio/macOS support (✅ IMPLEMENTED - macOS 14.4+ Process Tap API) |

### Features

| Document | When to Read |
|----------|--------------|
| [frontend-patterns.md](features/frontend-patterns.md) | Building React components, styling, themes, state management |
| [eq-implementation.md](features/eq-implementation.md) | Working on the 10-band equalizer |

Note: `frontend-patterns.md` includes comprehensive theme system documentation (7 themes including accessibility options).

### Integrations

| Document | When to Read |
|----------|--------------|
| (empty - add as needed) | |

### API Documentation

| Document | When to Read |
|----------|--------------|
| (empty - add as needed) | |

### Bug Fixes

| Document | When to Read |
|----------|--------------|
| bugfixes/* | Only if debugging similar symptoms to filenames |

### Prompts

| Document | When to Read |
|----------|--------------|
| prompts/* | When needing reusable prompts for code review, summarization, etc. |

### AI Patterns

| Document | When to Read |
|----------|--------------|
| [mistake-log.md](ai-patterns/mistake-log.md) | Reference when making similar mistakes repeatedly |

### Session Handoffs

| Document | When to Read |
|----------|--------------|
| sessions/* | When continuing paused work (user will specify which file) |

## Document Structure

Each knowledge document should include:

- **Last Updated**: Date of last modification
- **Context**: When this document is relevant
- **Content**: The actual knowledge/documentation

## Related Project Files

These files live outside the KB but are important references:

| File | Purpose |
|------|---------|
| `docs/VISION.md` | **Complete vision document** - PDF spec + UI vision, discrepancies marked |
| `DEVELOPMENT.md` | Developer guide: prerequisites, build commands, debugging |
| `AGENT.md` | AI agent instructions and coding conventions |
| `CLAUDE.md` | Claude Code specific entry point (references AGENT.md) |

### Vision Document (IMPORTANT)

Before implementing any major feature, read `docs/VISION.md`. It contains:
- Full architecture from the original PDF specification
- Per-app audio capture vision (core differentiating feature)
- UI mockup for stream list with expandable EQ
- Implementation status for each feature (✅/⚠️/❌)
- Platform-specific requirements (PipeWire, WASAPI, CoreAudio)

## Adding New Documents

Use kebab-case filenames. Place documents in the appropriate category folder:

- `architecture/` - System design, data flows, infrastructure patterns
- `features/` - Feature-specific implementation details
- `integrations/` - Third-party service integrations
- `api/` - API documentation and contracts
- `bugfixes/` - Root cause analyses worth preserving
- `prompts/` - Reusable AI prompts (code review, ticket summarizers, etc.)
- `ai-patterns/` - Mistake log and common corrections
- `sessions/` - Session handoff documents for work continuity
