# Command: session-end-checklist

**Category**: LOW-CONTEXT (AGENT.md only)

## Description

Quick checklist to run before ending any AI session. Catches automation that should have triggered but didn't. This is the safety net for agent compliance.

## Triggers

### Automatic (MUST detect and run)

Trigger when user says ANY of:
- "done", "done for now", "that's all", "thanks"
- "ending session", "wrapping up", "stopping here"
- "let's commit", "commit everything", "push this"
- "bye", "talk later", "end of day"
- Any indication session is ending

### Manual
- User says: "run session-end-checklist"
- User says: "end session"
- User says: "wrap up"

## Steps

Run through this checklist and take action on any "Yes":

### 1. Complex Problem Check
```
Did we solve something that required:
- Reading 3+ files?
- 5+ back-and-forth exchanges?
- A non-obvious pattern?
- KB lookup that found nothing?

→ If YES: Run `document-solution`
```

### 2. Correction Check
```
Did the user correct me at any point?
- Said "that's wrong", "actually...", "no..."?
- Pointed out a mistake?
- Provided a fix?

→ If YES: Run `log-mistake` for each correction
```

### 3. KB Modification Check
```
Did I create or modify any file in docs/ai-knowledge/?

→ If YES: Run `check-kb-index`
```

### 4. Uncommitted Changes Check
```
Are there uncommitted changes that should be committed?

→ If YES: Run `pre-commit-check`, then offer to commit
```

### 5. Session Length Check
```
Was this a long session (20+ turns)?

→ If YES: Run `save-session`
```

## Output

Report to user:

```
Session End Checklist
=====================

[x] Complex problem documented: [filename or "None needed"]
[x] Corrections logged: [count or "None"]
[x] KB index updated: [Yes/No/Not needed]
[x] Pre-commit check: [Passed/Issues found/No changes]
[x] Session saved: [filename or "Not needed"]

Session complete. Ready for next session.
```

## Example

User: "That's all for today, thanks!"

Agent thinks:
- Did we solve complex problem? Yes - figured out PipeWire routing
- Was I corrected? Yes - once about using default_input_device
- KB files modified? No
- Uncommitted changes? Yes - 3 files
- Long session? Yes - 25 turns

Agent runs:
1. `document-solution` → creates pipewire-routing-pattern.md
2. `log-mistake` → appends to mistake-log.md
3. `pre-commit-check` → shows results
4. `save-session` → creates session handoff

Agent reports:
```
Session End Checklist
=====================

[x] Complex problem documented: architecture/pipewire-routing-pattern.md
[x] Corrections logged: 1 (default_input_device misuse)
[x] KB index updated: Yes (after document-solution)
[x] Pre-commit check: Passed - 3 files ready
[x] Session saved: sessions/2025-12-29-pipewire-work.md

Session complete. Ready for next session.
```
