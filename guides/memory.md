# engram memory guide

`.engram/memory/` is the repository's persistent knowledge base for AI agents. It accumulates cross-cutting learnings from closed issues via `engram plan learn`.

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

Every memory file has typed, OKF v0.2-compatible YAML frontmatter (`src/memory/mod.rs:Frontmatter`):

```yaml
---
type: tripwire                       # pattern | tripwire | architecture | testing
title: "Human-readable title"
read_when:
  - "calling claude -p programmatically from engram"
  - "debugging unexpected Claude behavior"
tripwires:              # can be [] for patterns/architecture/testing files
  - action: "Invoking claude -p with current_dir set to a repo directory"
    warning: "Always use current_dir(std::env::temp_dir()) — repo CLAUDE.md turns Claude into an agent"
generated:
  by: "engram/0.1.0"
  at: "2026-05-18"
sources:
  - id: "26"
    resource: "https://github.com/<owner>/<repo>/issues/26"
  - id: "31"
    resource: "https://github.com/<owner>/<repo>/issues/31"
---
```

`sources` lets you trace a memory file back to its originating issue(s) and diff. If a learning seems wrong, check the linked issue for context. `title`, `read_when`, and `tripwires` are engram extension keys the OKF spec explicitly permits; `type`, `generated`, and `sources` map to the OKF v0.2 schema.

Files written by `engram capture --session` also carry `origin: session`. It marks an unreviewed, mid-session observation rather than a `learn`-synthesized learning, so weigh it accordingly during compaction. The marker is dropped if `learn` later rewrites the same file.

Files written before this schema existed used `last_updated`/`source_issues` instead of `generated`/`sources`. The reader still accepts that legacy shape; run `engram memory migrate` to rewrite everything into the current shape (idempotent — safe to run repeatedly).

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

Do not run compact immediately after `engram plan learn` — wait until several learn cycles have accumulated so compact has enough material to reason about relationships between files.

## Reviewing a learn or land PR

`engram plan learn`, `plan land` and `objective land` open a PR of new memory files. Expect to keep roughly a third or less; keeping everything is a red flag. For each file:

1. **Already in CLAUDE.md/AGENTS.md?** Those load every session, so a file that restates them (or follows directly from them) is pure duplication → delete.
2. **Issue/node number in the title or slug, or "differs from" the issue/spec/old tool?** The code is the source of truth, not the closed issue → delete, unless the divergence is itself a recurring trap (then rewrite it as the trap, without the history).
3. **One-off process advice** tied to a single event, PR or migration → delete.
4. **Describes the diff** — function names, signatures, struct shapes, single-feature details → delete.
5. **Verify every claim against the current code**, not against the issue; fix or delete what is wrong.
6. **Several files sharing one root insight** (common when one change yields many files) → merge into the strongest.

What survives records a failure, an external constraint, or a non-obvious WHY that applies beyond the feature. After deleting files, run `engram memory migrate` (idempotent) to rebuild `index.md` and the CLAUDE.md section, or remove the deleted rows from `index.md` by hand.

## Recording observations mid-session

When you notice something worth remembering during a session, record it right away instead of waiting for `plan land`:

```
engram capture --session --category tripwires --title "Short title" \
  --read-when "when this applies" --body "What you observed and why it matters"
```

No issue or PR is needed. Use `--slug` if the derived filename already exists. Session files are consolidated later by `engram compact`.

## Manual maintenance

You can edit memory files directly without going through `engram learn`:

1. Edit the file and update `generated.at` to today's date
2. If the learning no longer applies (e.g., the source was refactored), delete the file
3. Run `engram memory migrate` to rebuild `index.md` (or let the next `engram learn` do it)

Do not leave stale learnings in place — an agent that reads an outdated tripwire warning will mistrust the whole memory system.

## CLAUDE.md integration

CLAUDE.md contains an auto-managed section:

```
<!-- engram:start -->
@.engram/memory/index.md
<!-- engram:end -->
```

This section holds **only** the `@path` reference to `index.md` itself — never inline memory content directly, and never `@`-reference individual topic files from here. If any memory file's body contains the string `<!-- engram:end -->` literally, the boundary detection in `src/memory/mod.rs:write_claude_md_section` will break and truncate the section.

`index.md`'s own entries link to topic files with plain `./category/slug.md` markdown links, not `@` imports — an `@`-prefixed link there would eager-load every memory file into every session, silently defeating `read_when` routing. Only the one `@.engram/memory/index.md` line in CLAUDE.md is eager-loaded.

Do not manually edit text between the markers; `engram learn` and `engram compact` manage it automatically.
