# AI Command Library

**Last Updated**: December 2024

This directory contains reusable command workflows for AI agents working on this repository. Commands standardize common tasks and ensure consistency across sessions and tools.

---

## Purpose

AI agents (Claude Code, Cursor, Windsurf, Copilot, Google Antigravity) can reference these commands to:
- Execute standardized workflows consistently
- Trigger behaviors automatically based on context
- Maintain documentation and code quality
- Enable session continuity across conversations

---

## How Agents Use Commands

### Automatic Triggers
Agents should recognize when a command's trigger conditions are met and run it without being asked.

### Manual Invocation
Users can invoke commands directly:
- "run [command-name]"
- Natural language that matches the command's purpose

---

## Command Index

| Command | Triggers | Description |
|---------|----------|-------------|
| [list-commands](list-commands.md) | User asks what commands exist | Lists all available commands |
| [pre-commit-check](pre-commit-check.md) | Before any git commit | Validates staged files against AGENT.md |
| [save-session](save-session.md) | Long session, pausing work | Creates handoff doc for continuity |
| [document-solution](document-solution.md) | Complex problem solved | Creates KB doc from solution |
| [check-test-coverage](check-test-coverage.md) | After implementing feature/fix | Identifies missing test coverage |
| [check-kb-index](check-kb-index.md) | After KB file changes | Updates knowledge base index |
| [log-mistake](log-mistake.md) | User corrects agent error | Logs correction for future reference |
| [check-agent-drift](check-agent-drift.md) | Periodic / on request | Verifies AGENT.md matches codebase |

---

## Creating New Commands

Create a command when a task is repeated across sessions and benefits from standardization.

### Standard Format

```markdown
# Command: [name]

## Description
[What this command does]

## Triggers

### Automatic
[Conditions that should trigger this command automatically]

### Manual
[How users can invoke this command]

## Prerequisites
[What must be true before running]

## Steps
[Numbered steps to execute]

## Output
[What the command produces]

## Example
[Example usage/output]
```
