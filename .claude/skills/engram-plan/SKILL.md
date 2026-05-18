---
name: engram-plan
description: Use this skill when the user asks to "create a plan", "open a plan issue", "write a plan for X", "use engram plan", "start a new feature", or is about to begin work that will be tracked as an engram-plan GitHub issue. Provides guidance on writing effective engram plan issues that produce useful learnings later.
version: 0.1.0
---

# engram-plan skill

An **engram plan** is a GitHub issue labeled `engram-plan`. It represents exactly one unit of work — one PR from start to merge. After the PR merges, `engram learn <N>` (or `engram land <N>`) synthesizes cross-cutting learnings from the issue and PR into `.engram/memory/`.

## Quick reference

```
engram plan "Add --dry-run flag to compact command" \
  --body "..."

# implement, open PR with "closes #N" in body, merge

engram land <N>
```

## Title conventions

The title becomes the GitHub issue title **and** is fed to Claude as context during `engram learn`. Write it as a concise verb phrase describing the change:

- Good: `"Add --dry-run flag to compact command"`
- Good: `"Fix branch-not-found error in cmd_land when PR already merged"`
- Bad: `"Dry run"` (too vague — Claude can't derive `read_when` conditions from this)
- Bad: `"Investigate the compact issue"` (describes the work, not the change)

The title is used by `synthesize_learnings` in `src/claude.rs` as part of the synthesis prompt. A precise title produces more specific `read_when` routing conditions in the memory files.

## Body conventions

Pass `--body` with three sections:

```
**Why:** One sentence on the motivation. What breaks today, or what capability is missing.

**What:** Acceptance criteria as a short checklist. What must be true when this is done.

**Scope:** Which source modules will change. Example: "touches src/compact.rs and src/main.rs:cmd_compact".
```

The scope note helps Claude during synthesis — it links the learning to specific source locations and avoids broad, under-routed memory files.

## Scope discipline — one plan, one PR

`engram learn <N>` finds the merged PR that closed issue `N` via GitHub's `CLOSED_EVENT` graph. It expects **exactly one** such PR. If the work splits into two PRs, create two plan issues — one closes each PR.

The PR body must contain `closes #N` (or `fixes #N`, `resolves #N`). Without that marker, GitHub does not generate the close event and `engram learn` will report "no linked PR found."

## Branch naming

`engram land <N>` tries three branch name patterns in order:
1. `fix/issue-N`
2. `feat/issue-N`
3. `issue-N`

Name your working branch to match one of these so `land` can delete it automatically after close. Any other name requires manual branch cleanup.

## When NOT to create an engram plan

Skip the plan if the work is:
- A single-file typo/rename with no cross-cutting insight worth capturing
- A documentation-only change
- A revert of a recent commit

Plans generate learnings. If the work won't teach Claude anything new about the codebase, a plain branch + PR is sufficient.

## Full workflow

```
# 1. Create the plan issue
engram plan "Verb-phrase title" --body "Why / What / Scope"

# 2. Create a branch
git checkout -b feat/issue-<N>

# 3. Implement; open PR with "closes #N" in the PR body

# 4. After PR merges:
engram land <N>
# land = engram learn <N> + close issue + delete local branch
```

If you want to inspect the synthesized memory files before merging the learning PR, use `engram learn <N>` instead of `land`, review the branch, then run `engram land <N>` to close and clean up.
