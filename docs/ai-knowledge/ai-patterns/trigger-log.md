# AI Trigger Execution Log

**Purpose**: Track when AI agents execute mandatory triggers. Helps identify patterns and gaps in automation.

**How to use**: Agents should append entries when executing mandatory commands. Review periodically to assess automation health.

---

## Log Format

```markdown
## [YYYY-MM-DD] - [Agent] - [Command]

**Trigger**: [What triggered the command]
**Outcome**: [Success/Partial/Skipped]
**Notes**: [Any relevant context]

---
```

## Categories

- `pre-commit-check` - Ran before staging/committing
- `document-solution` - Created KB doc after solving problem
- `log-mistake` - Logged correction pattern
- `check-kb-index` - Updated KB index after file changes
- `save-session` - Created session handoff doc
- `session-end-checklist` - Ran end-of-session verification

---

## Log Entries

<!-- Entries are appended by AI agents when running mandatory triggers -->
<!-- This provides observability into automation compliance -->

*No entries yet. Entries will be added as triggers execute.*

---

## Monthly Summary Template

At the end of each month, an agent (or human) should summarize:

```markdown
## Month YYYY Summary

| Command | Executions | Notes |
|---------|------------|-------|
| pre-commit-check | X | |
| document-solution | X | |
| log-mistake | X | |
| check-kb-index | X | |
| save-session | X | |
| session-end-checklist | X | |

**Observations**:
- [Any patterns noticed]
- [Commands that should fire more often]
- [Suggested improvements]
```
