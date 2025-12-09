# Command: check-kb-index

## Description
Verifies and updates the knowledge base README.md index after documentation changes. Ensures index stays in sync with actual files.

## Triggers

### Automatic
- After creating any file in docs/ai-knowledge/
- After modifying any file in docs/ai-knowledge/
- After running document-solution command

### Manual
- User says: "run check-kb-index"
- User says: "update kb index"

## Prerequisites
- docs/ai-knowledge/README.md exists

## Steps

1. Scan all .md files in docs/ai-knowledge/ (excluding README.md)

2. Extract from each file:
   - Category (from folder)
   - Filename
   - Title (first # heading)
   - Context line (from **Context**: if present)

3. Load current README.md index

4. Compare and identify:
   - New files not in index
   - Removed files still in index
   - Files with changed titles/context

5. Update README.md index tables by category

6. Report changes made

## Output
- Updated docs/ai-knowledge/README.md
- Console report of changes

## Example

```
KB Index Update
===============

Scanning docs/ai-knowledge/...

Found 8 documents:
- architecture/audio-pipeline.md
- architecture/realtime-rules.md
- architecture/platform-linux.md
- architecture/platform-windows.md
- architecture/platform-macos.md
- features/frontend-patterns.md
- features/eq-implementation.md
- ai-patterns/mistake-log.md

Changes to index:
+ Added: architecture/platform-linux.md
+ Added: architecture/platform-windows.md
+ Added: architecture/platform-macos.md
~ Updated: features/frontend-patterns.md (title changed)

Index updated successfully.
```
