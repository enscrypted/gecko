# Command: document-solution

## Description
Creates a knowledge base document from a complex problem solution. Captures pattern, gotchas, and key files for future reference.

## Triggers

### Automatic
- Bug fix required understanding 3+ files
- Solution had 5+ back-and-forth exchanges
- Non-obvious pattern was discovered

### Manual
- User says: "run document-solution"
- User says: "document this" or "add to knowledge base"

## Prerequisites
- Just completed solving a non-trivial problem
- Solution is generalizable (not one-off)

## Steps

1. Analyze the solution:
   - Root problem?
   - Files involved?
   - Non-obvious part?
   - Gotchas encountered?

2. Determine category: `architecture/`, `features/`, `integrations/`, `api/`, `bugfixes/`

3. Generate kebab-case filename

4. Create document following KB format:

```markdown
# [Topic Title]

**Last Updated**: [Month Year]
**Context**: Read when [trigger conditions]

## Overview
[Problem space and why it matters]

## The Problem
[What goes wrong / what's confusing]

## The Solution
[Pattern that works, key files, code examples]

## Gotchas
[What to watch out for]

## Related Files
[List of relevant files with line references]
```

5. Save to `docs/ai-knowledge/[category]/[filename].md`

6. Run check-kb-index to update index

## Output
- New KB document
- Updated index
- Confirmation

## Example

After fixing a complex BiQuad filter test:

```
Created: docs/ai-knowledge/bugfixes/biquad-transient-response.md

# BiQuad Filter Transient Response

**Last Updated**: December 2024
**Context**: Read when writing tests for audio filters

## The Problem
Tests expected filters to pass through audio unchanged at 0dB gain,
but BiQuad filters have transient response...

[etc.]
```
