# Session: Firefox YouTube Scrub Bug Investigation

**Date**: 2025-12-08
**Status**: In Progress - Fix Attempted But Not Working

## Task Summary

Investigating and attempting to fix a recurring bug where **Firefox drops as a recognized app when the user scrubs (drags the slider) on a YouTube video**. The app loses audio routing through Gecko's per-app EQ system.

This bug has been investigated across multiple sessions. Previous fixes did NOT resolve the issue.

## Problem Description

### Symptom
1. Start Gecko, Firefox audio works through per-app EQ
2. Play a YouTube video in Firefox
3. Scrub (drag the slider) on the YouTube video timeline
4. Firefox disappears from Gecko's audio stream list
5. Firefox audio no longer goes through Gecko's EQ

### Root Cause Analysis (from logs)

When scrubbing a YouTube video, Firefox's PipeWire behavior is:

```
Node removed: id=118 (Firefox's old stream node)
Raw node detected: Firefox (id=135...) (New node created)
New stream node for existing app 'Firefox' (node=135) - queuing for relink
Node removed: id=135 (But new node is removed ~7ms later!)
No app nodes found for 'Firefox' (Relinking fails because node is gone)
```

**The actual root cause**: Firefox creates a **transient stream node** during video scrubbing that exists for only ~7ms before being destroyed. The relink logic correctly queues the app for relinking, but by the time the `process_pending_app_capture_links` function runs (on the next 100ms tick), the node is already gone.

The app stays in `pending_app_capture_links` indefinitely, spamming "No app nodes found" every 100ms until the user plays audio again.

## Key Decisions

- **Decision**: Added retry-based timeout mechanism to prevent infinite retry loops
  - Rationale: Transient nodes that appear and immediately disappear should not cause the system to retry forever

- **Decision**: Set MAX_RELINK_RETRIES = 50 (5 seconds at 100ms intervals)
  - Rationale: 5 seconds is enough time to determine if an app actually stopped playing vs just a transient node

## Files Modified

| File | Change |
|------|--------|
| `crates/gecko_platform/src/linux/thread.rs` | Added retry timeout mechanism for transient stream nodes |

### Specific Changes Made

1. **Added new field to `LocalState` struct** (line ~271-274):
```rust
/// Retry count for apps in pending_app_capture_links
/// If an app has no nodes for too many iterations, remove it from the list
/// This handles transient stream nodes (like Firefox during video scrubbing)
pending_app_link_retries: HashMap<String, u32>,
```

2. **Added constant** (line ~384-387):
```rust
/// Maximum retry attempts for relink before giving up (50 * 100ms = 5 seconds)
/// This handles transient stream nodes that appear and immediately disappear
/// (like Firefox during video scrubbing)
const MAX_RELINK_RETRIES: u32 = 50;
```

3. **Modified `process_pending_app_capture_links` function** (lines ~432-453):
```rust
if app_nodes.is_empty() {
    // Increment retry counter and check if we should give up
    // This handles transient nodes (like Firefox during video scrubbing)
    let retry_count = local.pending_app_link_retries.entry(app_name.clone()).or_insert(0);
    *retry_count += 1;

    if *retry_count >= MAX_RELINK_RETRIES {
        tracing::info!(
            "Giving up on relink for '{}' after {} retries (app likely stopped playing)",
            app_name,
            retry_count
        );
        gave_up.push(app_name.clone());
    } else if *retry_count % 10 == 0 {
        // Only log every 10 retries to avoid spam
        tracing::debug!("No app nodes found for '{}' (retry {}/{})", app_name, retry_count, MAX_RELINK_RETRIES);
    }
    continue;
}

// Found app nodes - reset retry counter
local.pending_app_link_retries.remove(app_name);
```

4. **Added cleanup code** (lines ~544-549):
```rust
// Remove apps that gave up (exceeded MAX_RELINK_RETRIES) from pending list and retry counters
// This prevents infinite retry loops for transient stream nodes (like Firefox during video scrubbing)
for app in &gave_up {
    local.pending_app_capture_links.retain(|a| a != app);
    local.pending_app_link_retries.remove(app);
}
```

## Current State

- Code compiles and passes clippy
- **User reports the fix did NOT work** - same behavior occurs
- The retry timeout mechanism stops the log spam but does NOT preserve Firefox's audio routing

## Why the Fix Didn't Work

The retry timeout fix addresses a **symptom** (infinite retry spam) but not the **actual problem**:

**The actual problem is that when Firefox scrubs, its OLD stream node is removed before a NEW stable node replaces it.** The transient node that appears for ~7ms is NOT the node Firefox will use for playback - it's a temporary node created during the scrub action.

The fundamental issue is:
1. Firefox's old node (id=118) gets removed
2. A transient node (id=135) appears for ~7ms then disappears
3. Firefox creates a NEW stable node (id=136+) for actual playback
4. But Gecko has already given up or is confused about the state

## Hypotheses for Next Steps

### Hypothesis 1: Node ID Mismatch
The per-app sink (`Gecko-Firefox`) may be linked to the OLD node ID. When the old node is removed, the links are destroyed. The new stable node that Firefox creates doesn't get linked because Gecko thinks Firefox is already "linked".

**Investigation needed**: Check if `app_captures` or the link tracking correctly handles node replacement.

### Hypothesis 2: Capture Stream State
The capture stream (`AppCaptureState`) may still be referencing the old node's ring buffer or streams. When the old node is removed, the capture becomes stale.

**Investigation needed**: Check what happens to `app_captures` when a node is removed vs added.

### Hypothesis 3: Link Lifecycle
PipeWire may automatically destroy links when nodes are removed. When Firefox's node is removed, our links are destroyed. We may need to:
- Detect link destruction events
- Re-queue the app for relinking when links are destroyed

**Investigation needed**: Check if we handle `ObjectType::Link` removal events and whether we should re-queue apps when their links are destroyed.

### Hypothesis 4: The "Correct" Node Detection
We may be detecting the transient node and trying to link to IT, then failing. Instead, we should perhaps:
- Ignore nodes that exist for less than X ms
- Only link to nodes that are "stable" (exist for at least 100ms)

## Key Files to Investigate

1. **`crates/gecko_platform/src/linux/thread.rs`** - Main PipeWire thread
   - `process_pending_app_capture_links` function (lines ~389-553)
   - Node detection in registry listener (lines ~1480-1600)
   - Node removal handling (search for "Node removed")
   - Link removal handling (search for "Link removed")

2. **Check for link removal handling**:
   - Search for `ObjectType::Link` in the global_remove handler
   - See if we re-queue apps when their links are destroyed

## How to Continue

1. Start a new conversation
2. Say: "Continue from session 2025-12-08_firefox-youtube-scrub-bug"
3. Read this document and the thread.rs file
4. Investigate Hypothesis 3 first (link lifecycle) - it seems most promising
5. Add logging to track:
   - When links are created/destroyed
   - What node IDs Firefox uses before/after scrubbing
   - Whether Gecko detects the new stable node

## Commands to Test

```bash
# Build and run
pnpm tauri dev

# Watch logs in terminal, filter for Firefox:
# Look for: Node added/removed, Link created/destroyed, relink attempts

# To reproduce:
1. Start Gecko
2. Start Firefox, play YouTube video
3. Wait for Firefox to appear in Gecko stream list
4. Scrub the YouTube video timeline
5. Observe Firefox disappearing from stream list
```

## Previous Session Context

This is a continuation of debugging efforts from previous sessions. Key context:
- Previous fix attempt: Queue existing apps for relinking when new stream node detected
- That fix was added at lines ~1536-1560 in thread.rs
- The "New stream node for existing app" log line confirms that fix is running
- But the node disappears before relinking can complete

## Technical Details for Next Agent

### PipeWire Node Lifecycle During YouTube Scrub

```
T+0ms:    Node 118 (Firefox playback) exists, linked to Gecko-Firefox sink
T+0ms:    User scrubs YouTube video
T+1ms:    Firefox destroys node 118
T+2ms:    Node removed event fires, Gecko handles it
T+3ms:    Firefox creates transient node 135
T+5ms:    Gecko detects node 135, queues Firefox for relink
T+10ms:   Firefox destroys transient node 135
T+11ms:   Node removed event fires
T+100ms:  process_pending_app_capture_links runs
T+100ms:  No nodes found for Firefox (node 135 is gone)
T+???ms:  Firefox creates NEW stable node 136
T+???ms:  Gecko may or may not detect this depending on timing
```

The key insight is: **Firefox creates MULTIPLE nodes during a scrub operation**. We need to handle this gracefully.
