# Command: save-session

## Description
Creates a handoff document capturing current session context, decisions, and next steps. Enables seamless continuation in a new conversation.

## Triggers

### Automatic
- Conversation exceeds ~20 turns
- User says "let's pause here" or "save this for later"
- Context is getting low (compact warning)

### Manual
- User says: "run save-session"
- User says: "save progress" or "create handoff"

## Prerequisites
- Active work session with context to preserve

## Steps

1. Generate filename: `YYYY-MM-DD_topic-slug.md`

2. Gather session context:
   - What tasks were we working on?
   - What decisions were made?
   - What files were modified/created?
   - What's unfinished?
   - Any blockers?

3. Create handoff document at `docs/ai-knowledge/sessions/[filename]`:

```markdown
# Session: [Topic]

**Date**: YYYY-MM-DD
**Status**: [In Progress / Paused / Completed]

## Task Summary
[What we were doing]

## Key Decisions
- Decision: Rationale

## Files Modified
| File | Change |
|------|--------|
| path/to/file | Description |

## Current State
[Where things stand]

## Next Steps
1. [Action item]
2. [Action item]

## How to Continue
[Instructions for resuming this work]
```

4. Confirm to user with continuation instructions

## Output
- New file: `docs/ai-knowledge/sessions/YYYY-MM-DD_topic-slug.md`
- Confirmation with how to continue later

## Example

```
Session saved to: docs/ai-knowledge/sessions/2024-12-06_eq-implementation.md

To continue this work later:
1. Start a new conversation
2. Say: "Continue from session 2024-12-06_eq-implementation"
3. I'll read the handoff doc and pick up where we left off
```
