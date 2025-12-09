# Command: pre-commit-check

## Description
Validates staged files against AGENT.md standards before any git commit. Catches common issues like real-time safety violations, missing tests, and style inconsistencies.

## Triggers

### Automatic
- Before ANY git commit operation
- When user says "commit" or asks to commit changes

### Manual
- User says: "run pre-commit-check"
- User says: "validate my changes"

## Prerequisites
- Changes staged for commit
- AGENT.md readable at repository root

## Steps

1. Get staged files: `git diff --cached --name-only`

2. For each staged Rust file, check:
   - No `println!` in audio-related code (use tracing)
   - No `unwrap()` without context (use `expect()` or `?`)
   - No allocations in functions named `process*`
   - Tests exist for new public functions
   - Doc comments on public items

3. For each staged TypeScript/TSX file, check:
   - Uses `gecko-*` color tokens (no raw colors)
   - Components use `forwardRef` if in `ui/` folder
   - `useCallback` for handlers passed to children
   - No inline styles

4. For all files:
   - No secrets or credentials
   - No TODO without context
   - File follows existing patterns in codebase

5. Report findings with file:line references

6. If violations found, ask user to fix or proceed anyway

## Output
- Console report of all violations
- User prompted to fix or proceed

## Example

```
Pre-commit Check Results
========================

src-tauri/src/stream.rs:45
  WARNING: println! in audio code - use tracing instead

crates/gecko_dsp/src/eq.rs:120
  OK: No real-time safety issues detected

src/components/NewComponent.tsx:15
  WARNING: Using raw color "bg-gray-900" instead of "bg-gecko-bg-primary"

Summary: 2 warnings, 0 errors
Proceed with commit? (y/n)
```
