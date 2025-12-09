# Command: list-commands

## Description
Meta-command that lists all available commands in the AI command library with descriptions and triggers.

## Triggers

### Automatic
- None (manual only)

### Manual
- User says: "run list-commands"
- User says: "what commands are available?"
- User says: "show me the command library"

## Prerequisites
None.

## Steps
1. Read all .md files in docs/ai-commands/ (excluding README.md)
2. Extract command name, description, and triggers from each
3. Format as readable list
4. Present to user

## Output
Formatted list of all commands with descriptions and triggers.

## Example

```
Available Commands:

1. pre-commit-check
   Validates staged files against AGENT.md before commits
   Trigger: Before any git commit

2. save-session
   Creates handoff document for session continuity
   Trigger: Long session (20+ turns) or "save progress"

3. document-solution
   Creates KB doc from complex problem solution
   Trigger: 3+ files modified, 5+ exchanges

...
```
