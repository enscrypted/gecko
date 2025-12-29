# Command: cleanup-sessions

**Category**: META (no project context needed)

## Description

Deletes old session handoff documents to prevent accumulation. Sessions older than 30 days are typically no longer relevant for context continuity.

## Triggers

### Manual
- User says: "run cleanup-sessions"
- User says: "clean up old sessions"
- User says: "delete old session files"

### Suggested
- When `docs/ai-knowledge/sessions/` has 10+ files
- During periodic maintenance
- When KB health check flags stale sessions

## Prerequisites

- None

## Steps

1. **List session files**
   ```bash
   ls -la docs/ai-knowledge/sessions/*.md | grep -v README.md
   ```

2. **Identify old sessions** (30+ days)
   - Session files use format: `YYYY-MM-DD-description.md` or `YYYY-MM-DD_description.md`
   - Parse date from filename
   - Calculate age from today's date

3. **Scan for unlogged patterns** (before deletion)
   - Quick scan each old session for:
     - Corrections mentioned but not in mistake-log
     - Solutions found but not documented
   - If found, ask user: "Session X has undocumented patterns. Log them first?"

4. **Delete old sessions**
   ```bash
   # Delete sessions older than 30 days
   find docs/ai-knowledge/sessions -name "*.md" ! -name "README.md" -mtime +30 -delete
   ```

5. **Report results**
   ```text
   Session cleanup complete:
   - Scanned: X sessions
   - Deleted: Y sessions (older than 30 days)
   - Kept: Z sessions (recent)

   Oldest remaining session: YYYY-MM-DD
   ```

## Options

| Option | Default | Description |
|--------|---------|-------------|
| `--days N` | 30 | Delete sessions older than N days |
| `--dry-run` | false | Preview deletions without executing |
| `--force` | false | Skip pattern scan, delete immediately |

## Example

User: "clean up old sessions"

Agent:
```text
Scanning docs/ai-knowledge/sessions/...

Found 8 session files:
- 2025-11-15-pipewire-routing.md (44 days old) → DELETE
- 2025-11-20-macos-debugging.md (39 days old) → DELETE
- 2025-12-07-audio-routing-debug.md (22 days old) → KEEP
- 2025-12-08-ui-polish-and-bugfixes.md (21 days old) → KEEP
- 2025-12-09-macos-implementation-plan.md (20 days old) → KEEP
- 2025-12-26-process-tap-api-debugging.md (3 days old) → KEEP

Deleting 2 sessions older than 30 days...

Session cleanup complete:
- Scanned: 6 sessions
- Deleted: 2 sessions
- Kept: 4 sessions

Oldest remaining session: 2025-12-07
```

## Notes

- Always preserves `sessions/README.md`
- Sessions are low-value after 30 days (context is stale)
- If a session contains valuable patterns, document them via `document-solution` before cleanup
- Run periodically or when session count grows large
