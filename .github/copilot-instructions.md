# GitHub Copilot Instructions

## FIRST: Verify Context is Loaded

If you have NOT yet read `AGENT.md` this session, read it NOW before proceeding.
If you have NOT yet scanned `docs/ai-knowledge/README.md` for relevant topics, do so NOW.

These files contain critical project conventions. Proceeding without them leads to incorrect implementations.

## DURING SESSION

- Load KB docs when task keywords match the README index
- On EVERY user message, check triggers: commit? correction? session end? → run matching command
- When context appears lost (unable to recall project conventions), re-read AGENT.md

---

## Quick Reference

| Topic | Document |
|-------|----------|
| All project instructions | AGENT.md |
| Knowledge base index | docs/ai-knowledge/README.md |
| Agent commands | docs/ai-commands/README.md |
| Architecture docs | docs/ai-knowledge/architecture/ |
| Feature docs | docs/ai-knowledge/features/ |
| Real-time safety rules | docs/ai-knowledge/architecture/realtime-rules.md |

## Critical Constraints

- **Real-time audio** - ZERO allocations in audio callbacks
- **NO microphone input** - Gecko captures APPLICATION audio, not voice
- **Per-app EQ** - Each app has independent EQ processing BEFORE mixing
- **Frontend tokens** - Use `gecko-*` Tailwind tokens, not raw colors

## Key Conventions

### Rust
- Use `thiserror` for error types
- Zero allocations in audio callbacks
- Tests inline with `#[cfg(test)]` modules
- Hardware tests marked `#[ignore]`

### Frontend
- CVA for component variants
- `gecko-*` color tokens only
- `forwardRef` for base components
- `useCallback` for handlers

## Mandatory Auto-Triggers (Do NOT Skip)

| Trigger | Action |
|---------|--------|
| User says "commit", "stage", "git add" | Run `pre-commit-check` BEFORE the operation |
| KB lookup failed → solved via code search | Run `document-solution` to update KB |
| User corrects you ("that's wrong", "actually...", etc.) | Run `log-mistake` immediately |
| Modified any KB file | Run `check-kb-index` |
| User says "done", "thanks", "bye", ending session | Run `session-end-checklist` |

See `docs/ai-commands/TRIGGER-CHECKLIST.md` for full details.

Always consult AGENT.md before making suggestions or generating code.
