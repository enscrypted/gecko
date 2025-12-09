# GitHub Copilot Instructions

**IMPORTANT**: Before responding to any request, read the following files:

1. **AGENT.md** (repository root) - Contains all project conventions, patterns, and constraints
2. **docs/ai-knowledge/README.md** - Index of domain-specific documentation
3. **docs/ai-commands/README.md** - Available agent commands

## Quick Reference

| Topic | Document |
|-------|----------|
| All project instructions | AGENT.md |
| Knowledge base index | docs/ai-knowledge/README.md |
| Agent commands | docs/ai-commands/README.md |
| Architecture docs | docs/ai-knowledge/architecture/ |
| Feature docs | docs/ai-knowledge/features/ |
| Real-time safety rules | docs/ai-knowledge/architecture/realtime-rules.md |

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

Always consult AGENT.md before making suggestions or generating code.
