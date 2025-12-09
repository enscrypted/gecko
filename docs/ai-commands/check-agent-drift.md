# Command: check-agent-drift

## Description
Verifies that AGENT.md accurately reflects current codebase state. Identifies drift between documented conventions and actual code patterns.

## Triggers

### Automatic
- None (resource-intensive, run on request)

### Manual
- User says: "run check-agent-drift"
- User says: "verify agent.md"
- User says: "is our documentation current?"

## Prerequisites
- AGENT.md exists
- Codebase is stable (not mid-refactor)

## Steps

1. Parse AGENT.md sections:
   - Development Commands
   - File/Folder Patterns
   - Coding Standards
   - Things to Avoid

2. Verify Commands section:
   - Test that listed commands work (`cargo test`, `pnpm dev`, etc.)
   - Check for new scripts in package.json/Cargo.toml not documented

3. Verify File Patterns:
   - Check listed paths exist
   - Look for new patterns not documented

4. Verify Conventions:
   - Sample recent code changes
   - Check if they follow documented patterns
   - Look for new patterns emerging

5. Check constraints accuracy:
   - Verify version numbers
   - Check platform requirements

6. Scan for undocumented patterns:
   - Look at recent commits
   - Identify any new conventions

7. Generate drift report:
   - Stale paths/commands
   - Missing patterns
   - Outdated information
   - Sections verified OK

8. Offer to apply fixes

## Output
- Detailed drift report
- Specific line references
- Option to auto-apply fixes

## Example

```
Agent Documentation Drift Report
================================

Checking AGENT.md against codebase...

## Commands Section
✓ cargo test --workspace - Works
✓ pnpm tauri dev - Works
✗ cargo bench -p gecko_dsp - File missing (benches/engine_benchmark.rs)

## File Patterns
✓ crates/gecko_*/src/error.rs - All exist
✓ src/components/ui/ - Exists with expected files
? src/stores/ - Listed but empty (is this intentional?)

## Coding Standards
✓ CVA pattern found in all ui/ components
✓ thiserror used for error types
✗ New pattern: Some components using zustand (not documented)

## Things to Avoid
✓ No println! found in audio code
✓ No raw Tailwind colors in components

Summary:
- 2 issues found
- 1 potential new pattern to document

Would you like me to update AGENT.md with these findings?
```
