# Trigger Checklist (Quick Reference)

**Last Updated**: December 2025

Use this checklist to verify you're triggering commands correctly. These are MANDATORY behaviors.

---

## At Session Start

- [ ] Read `AGENT.md`
- [ ] Check `docs/ai-knowledge/README.md` for relevant docs
- [ ] Review `docs/ai-knowledge/ai-patterns/mistake-log.md` for patterns to avoid
- [ ] Check for active session files in `docs/ai-knowledge/sessions/`

---

## During Session - Mandatory Triggers

### 🔴 MUST Trigger: `pre-commit-check`

When user:
- Says "commit", "git commit"
- Says "stage", "git add", "add these files"
- Says "prepare for commit"
- Asks to create a PR
- Any variation of committing code

**Action**: Run `pre-commit-check` BEFORE the git operation, not after.

---

### 🔴 MUST Trigger: `log-mistake`

When user says ANY of:
- "that's wrong", "that's not right", "that's incorrect"
- "no, it should be...", "actually..."
- "you forgot to...", "you missed..."
- "we already discussed...", "I told you earlier..."
- Provides a fix or correction to your output
- Points out convention/pattern you violated
- Explains why your approach won't work

**Action**: Run `log-mistake` immediately. Do NOT wait for "please log this".

---

### 🔴 MUST Trigger: `document-solution`

When ALL of these are true:
1. You checked KB for relevant docs
2. No relevant doc existed
3. You searched code/investigated to solve
4. You successfully solved the problem

Also when:
- Bug fix required reading 3+ files
- Solution used non-obvious pattern
- 5+ back-and-forth exchanges to resolve
- User says "document this"

**Action**: Run `document-solution` → then `check-kb-index`.

---

### 🔴 MUST Trigger: `check-kb-index`

When you:
- Created any file in `docs/ai-knowledge/`
- Modified any file in `docs/ai-knowledge/`
- Ran `document-solution`

**Action**: Run `check-kb-index` to update the index.

---

### 🔴 MUST Trigger: `save-session`

When:
- Session reaches 20+ turns
- User says "let's pause", "continue later", "stop for now"
- User indicates switching to different task
- End of workday/session context

**Action**: Run `save-session` to create handoff doc.

---

### 🔴 MUST Trigger: `session-end-checklist`

When user says ANY of:
- "done", "done for now", "that's all", "that's it"
- "thanks", "thank you", "bye", "talk later"
- "ending session", "wrapping up", "stopping here"
- Any indication session is ending

**Action**: Run `session-end-checklist` to catch any missed automation.

This is the SAFETY NET - it verifies all other triggers fired correctly.

---

## Self-Check Questions

After completing any non-trivial task, ask yourself:

1. **Did I check KB first?** If not, I should have.
2. **Did KB have what I needed?** If no → need to document solution.
3. **Was I corrected by user?** If yes → need to log mistake.
4. **Did I modify KB files?** If yes → need to update index.
5. **Is user committing code?** If yes → need pre-commit check.
6. **Is session getting long?** If yes → consider save-session.
7. **Is user ending session?** If yes → run session-end-checklist.

---

## Command Quick Links

| Command | File |
|---------|------|
| `pre-commit-check` | [pre-commit-check.md](pre-commit-check.md) |
| `session-end-checklist` | [session-end-checklist.md](session-end-checklist.md) |
| `log-mistake` | [log-mistake.md](log-mistake.md) |
| `document-solution` | [document-solution.md](document-solution.md) |
| `check-kb-index` | [check-kb-index.md](check-kb-index.md) |
| `save-session` | [save-session.md](save-session.md) |
| `cleanup-sessions` | [cleanup-sessions.md](cleanup-sessions.md) |
| `check-test-coverage` | [check-test-coverage.md](check-test-coverage.md) |
| `list-commands` | [list-commands.md](list-commands.md) |
| `check-agent-drift` | [check-agent-drift.md](check-agent-drift.md) |
