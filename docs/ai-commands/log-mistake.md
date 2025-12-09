# Command: log-mistake

## Description
Logs correction patterns when user corrects an agent error. Builds reference of common mistakes. If pattern appears 3+ times, suggests adding to AGENT.md "Things to Avoid".

## Triggers

### Automatic
- User explicitly corrects agent's output
- User says "that's wrong" or "no, it should be..."
- User points out missed convention

### Manual
- User says: "run log-mistake"
- User says: "log this correction"

## Prerequisites
- A correction has just occurred
- Mistake is a pattern (not just a typo)

## Steps

1. Identify the mistake pattern:
   - What did agent do wrong?
   - What type of error? (convention, logic, performance, testing, etc.)

2. Identify the correction:
   - What should have been done?
   - What rule was violated?

3. Categorize: `convention`, `logic`, `performance`, `testing`, `style`, etc.

4. Append to `docs/ai-knowledge/ai-patterns/mistake-log.md`:

```markdown
### YYYY-MM-DD - [Category]

**Mistake**: [What was done wrong]

**Correction**: [What should have been done]

**Prevention**: [How to catch next time]

**Count**: 1
```

5. Check for repeated patterns:
   - Search for similar entries
   - If 3+ similar mistakes, alert user

6. If pattern is recurring (3+), suggest AGENT.md update

## Output
- Appended entry to mistake-log.md
- Alert if pattern is recurring
- Suggestion to update AGENT.md if warranted

## Example

User: "No, you shouldn't use println! in the audio callback, use tracing"

```
Logged to mistake-log.md:

### 2024-12-06 - Convention

**Mistake**: Used println! for debugging in audio callback code

**Correction**: Use tracing macros (debug!, info!, etc.) which can be
filtered and don't cause syscalls when disabled

**Prevention**: Check if code is in audio path before adding any I/O

**Count**: 1

---

Note: This is a new pattern. Will alert if it occurs 2 more times.
```
