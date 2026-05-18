---
name: engram-memory
description: Use this skill when the user asks to "check what engram knows", "read the memory", "run engram compact", "prune memory files", "is this memory file worth keeping", "review learnings", or is navigating, evaluating, or compacting the .engram/memory/ directory. Provides guidance on the memory layout, category semantics, compaction workflow, and manual maintenance.
version: 0.1.0
---

# engram-memory skill

`.engram/memory/` is the repository's persistent knowledge base for AI agents. It accumulates cross-cutting learnings from closed issues via `engram learn`.

## Layout

```
.engram/memory/
  index.md          ← routing table — READ THIS FIRST
  patterns/
    <slug>.md       ← approaches worth repeating
  tripwires/
    <slug>.md       ← things to avoid; past failures
  architecture/
    <slug>.md       ← structural decisions and WHY
  testing/
    <slug>.md       ← reusable test strategies
```

**Always read `index.md` first.** Load individual files only when their `read_when` condition matches the current task. Never load all files blindly — that wastes tokens and buries the relevant signal.

## Category semantics

| Category | Contains | Tone |
|----------|----------|------|
| `patterns/` | Approaches worth repeating across ≥2 future features | Prescriptive — "do this" |
| `tripwires/` | Things to avoid; past failures; external surprises | Cautionary — "avoid this" — always has a `tripwires:` YAML block |
| `architecture/` | Structural decisions affecting multiple modules; WHY a boundary exists | Explanatory — "this is why" |
| `testing/` | Test strategies reusable across the codebase | Prescriptive — "test this way" |

When a learning could fit two categories, prefer `tripwires/` if it records a past failure, and `patterns/` if it records a successful technique.

## Frontmatter fields

Every memory file has YAML frontmatter:

```yaml
---
title: "Human-readable title"
read_when:
  - "calling claude -p programmatically from engram"
  - "debugging unexpected Claude behavior"
tripwires:              # can be [] for patterns/architecture/testing files
  - action: "Invoking claude -p with current_dir set to a repo directory"
    warning: "Always use current_dir(std::env::temp_dir()) — repo CLAUDE.md turns Claude into an agent"
last_updated: "YYYY-MM-DD"
source_issues: [26, 31]  # GitHub issue numbers that produced this learning
---
```

`source_issues` lets you trace a memory file back to its originating PR and diff. If a learning seems wrong, check the source issue for context.

## Running `engram compact`

```
engram compact
```

Compact sends all memory files to Claude for audit. Claude returns `keep`, `delete`, or `merge_into` decisions. **The default verdict is DELETE** — a file must actively earn its keep by recording a failure, capturing a non-obvious external constraint, or explaining a WHY that is invisible in source.

After compact:
1. Reviews decisions and creates branch `engram/compact-<date>`
2. Commits all changes (deletions, merges, content updates)
3. Opens a PR labeled `engram-learned`

**Review the compact PR diff carefully before merging.** Compact can delete files that were genuinely valuable. Check each deletion: does the learning appear anywhere in the source code or CLAUDE.md? If not, is it possible future agents will still need it?

## When to run compact

Run compact when:
- The index has grown to ≥10 files and some entries look redundant
- Two or more files have overlapping `read_when` conditions
- Source code has been refactored and old learnings now describe behavior that no longer exists

Do not run compact immediately after `engram learn` — wait until several learn cycles have accumulated so compact has enough material to reason about relationships between files.

## Manual maintenance

You can edit memory files directly without going through `engram learn`:

1. Edit the file and update `last_updated` to today's date
2. If the learning no longer applies (e.g., the source was refactored), delete the file
3. Run `engram compact`-equivalent rebuild by editing `index.md` to remove the deleted entry, or just let the next `engram learn` call rebuild the index

Do not leave stale learnings in place — an agent that reads an outdated tripwire warning will mistrust the whole memory system.

## CLAUDE.md integration

CLAUDE.md contains an auto-managed section:

```
<!-- engram:start -->
@.engram/memory/index.md
<!-- engram:end -->
```

This section holds **only** the `@path` reference — never inline memory content directly. If any memory file's body contains the string `<!-- engram:end -->` literally, the boundary detection in `src/memory.rs:write_claude_md_section` will break and truncate the section.

Do not manually edit text between the markers; `engram learn` and `engram compact` manage it automatically.
