# AI Command Library

**Last Updated**: December 2025

This directory contains reusable command workflows for AI agents working on this repository. Commands standardize common tasks and ensure consistency across sessions and tools.

---

## ⚠️ Quick Reference: TRIGGER-CHECKLIST.md

**Start here**: [TRIGGER-CHECKLIST.md](TRIGGER-CHECKLIST.md) provides a quick-scan checklist of mandatory triggers. Use it to verify you're not missing automatic behaviors.

---

## Purpose

AI agents (Claude Code, Cursor, Windsurf, Copilot, etc.) can reference these commands to:
- Execute standardized workflows consistently
- Trigger behaviors automatically based on context
- Maintain documentation and code quality
- Enable session continuity across conversations

---

## How Agents Use Commands

### Automatic Triggers (MANDATORY)

Agents MUST recognize when a command's trigger conditions are met and run it **without being asked**. Key triggers:

| Trigger | Command | Why |
|---------|---------|-----|
| User says "commit" or "stage" | `pre-commit-check` | Catch violations before commit |
| KB miss → code search → solution | `document-solution` | Update KB for next time |
| User corrects agent | `log-mistake` | Build correction patterns |
| Modified KB files | `check-kb-index` | Keep index current |

**Do NOT wait for explicit requests** - detect triggers proactively.

### Manual Invocation
Users can invoke commands directly:
- "run [command-name]"
- "execute [command-name]"
- Natural language: "check if there are missing tests" → `check-test-coverage`

---

## Command Categories

Commands have context requirements that determine how much to load before execution:

| Category | Context Required | Commands |
|----------|------------------|----------|
| **META** | None (skip AGENT.md) | `list-commands`, `cleanup-sessions` |
| **LOW-CONTEXT** | AGENT.md only | `pre-commit-check`, `check-test-coverage`, `check-kb-index`, `log-mistake`, `session-end-checklist` |
| **FULL-CONTEXT** | AGENT.md + relevant KB | `document-solution`, `save-session`, `check-agent-drift` |

> **Note**: The index table below is a summary. Each command file contains complete details including triggers, prerequisites, and examples.

---

## Command Index

| Command | Category | Triggers | Description |
|---------|----------|----------|-------------|
| [list-commands](list-commands.md) | META | User asks what commands exist | Lists all available commands with descriptions |
| [pre-commit-check](pre-commit-check.md) | LOW | Before any git commit | Validates staged files against AGENT.md standards |
| [session-end-checklist](session-end-checklist.md) | LOW | User ending session (done/thanks/bye) | Catches missed automation - SAFETY NET |
| [save-session](save-session.md) | FULL | Long session, pausing work | Creates handoff documentation for session continuity |
| [document-solution](document-solution.md) | FULL | Complex problem solved | Creates KB doc from solution pattern |
| [check-test-coverage](check-test-coverage.md) | LOW | After implementing feature/fix | Identifies missing test coverage |
| [check-kb-index](check-kb-index.md) | LOW | After KB file changes | Updates knowledge base README.md index |
| [log-mistake](log-mistake.md) | LOW | User corrects agent error | Logs correction pattern for future reference |
| [check-agent-drift](check-agent-drift.md) | FULL | Periodic / on request | Verifies AGENT.md matches codebase reality |
| [cleanup-sessions](cleanup-sessions.md) | META | Manual / maintenance | Deletes session files older than 30 days |

---

## Creating New Commands

### When to Create a Command

Create a new command when:
- Task is repeated across multiple sessions
- Task has clear steps that benefit from standardization
- Task involves updating shared resources (KB, AGENT.md)
- Task is complex enough that agents might do it inconsistently

### When NOT to Create a Command

Skip creating a command when:
- One-off task that won't recur
- Task is already covered by existing command
- Task is too vague to have clear steps
- Task requires human judgment at every step

### Command Design Principles

1. **Single responsibility**: One command = one job
2. **Clear triggers**: Agent should know unambiguously when to run
3. **Idempotent**: Running twice shouldn't break anything
4. **Observable output**: User should see what happened
5. **Fail gracefully**: If something's wrong, explain don't crash

### Standard Command File Format

Every command file must follow this structure:

```markdown
# Command: [kebab-case-name]

## Description
One paragraph explaining what this command does and why it exists.

## Triggers

### Automatic
Conditions when agent should run this without being asked:
- [condition 1]
- [condition 2]

### Manual
- User says: "run [command-name]" or "[command-name]"
- User says: [natural language variations]

## Prerequisites
What must be true before running (if any).

## Steps
1. [Action]
2. [Action]
3. [Action]

## Output
- What gets created/modified
- Where it goes
- What to tell the user

## Example
[Show a concrete example of input → output if helpful]
```

### Naming Conventions

- Use kebab-case: `check-test-coverage.md` not `checkTestCoverage.md`
- Start with verb: `check-`, `create-`, `update-`, `log-`, `save-`
- Be specific: `check-test-coverage` not `check-tests`

### After Creating a Command

1. Add entry to the index table in this README
2. Test with at least one real scenario
3. Consider if AGENT.md needs to reference it

---

## Integration with AGENT.md

The command library is referenced in AGENT.md under "Self-Maintaining Behaviors". Agents should:
1. Read AGENT.md first (always)
2. Recognize command triggers during work
3. Load and follow command files when triggered
4. Report command execution to user

---

## Maintenance

This command library is self-maintaining:
- `list-commands` regenerates the index dynamically
- `check-agent-drift` identifies stale commands
- New commands follow the standard format for consistency
