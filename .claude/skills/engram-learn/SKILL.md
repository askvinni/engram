---
name: engram-learn
description: Use this skill when the user asks to "run engram learn", "synthesize learnings", "run engram land", "extract memory from issue", "what did we learn from #N", or is performing the post-PR memory workflow. Provides guidance on the learn/land workflow, what gets written to memory, and how to review synthesized learnings.
version: 0.2.0
---

# engram-learn skill

`engram plan learn <N>` synthesizes cross-cutting learnings from a closed issue and its merged PR into `.engram/memory/`. `engram plan land <N>` does the same plus closes the issue and deletes the local branch.

## Prerequisites

Before running, verify:

1. **Issue is CLOSED:**
   ```
   gh issue view <N> --json state --jq .state
   # must be "CLOSED"
   ```

2. **A MERGED PR exists that closed the issue** — the PR body must contain `closes #N` (or `fixes #N`, `resolves #N`):
   ```
   gh pr list --search "closes #<N>" --state merged
   ```

If either check fails, `engram plan learn` will exit with an error message. Fix the state before proceeding.

## What `engram plan learn` does

1. Fetches issue `#N` title and body
2. Finds the merged PR via GitHub's `CLOSED_EVENT` GraphQL query (`src/github.rs:find_linked_pr`)
3. Fetches the PR diff, capped at 8 000 bytes (`src/github.rs:get_pr_diff`)
4. Fetches issue comments and extracts the first comment containing `<!-- engram:conversation -->` (`src/github.rs:get_issue_comments`); the conversation text is included in the synthesis prompt under `## Planning Conversation` (also capped at 8 000 chars)
5. Reads all existing `.engram/memory/` files for deduplication context (`src/memory.rs:read_all`)
6. Loads `.engram/prompt-hooks/*.md` (excluding `README.md`) to inject repo-specific synthesis rules
7. Invokes `claude -p` **from `std::env::temp_dir()`** — not from the repo root — to prevent Claude Code from loading the repo's CLAUDE.md as agent context
8. Strips markdown code fences from the response, parses a JSON array of `LearningItem`s
9. For each item: writes to `.engram/memory/<category>/<slug>.md`, accumulating `source_issues` if the file already exists
10. Rebuilds `.engram/memory/index.md`
11. Updates the `<!-- engram:start --> ... <!-- engram:end -->` section in CLAUDE.md
12. Creates branch `engram/learn-<N>`, commits, pushes, opens a PR labeled `engram-learned`

## `engram plan land` vs `engram plan learn`

| | `engram plan learn <N>` | `engram plan land <N>` |
|---|---|---|
| Synthesizes learnings | yes | yes |
| Closes the GitHub issue | no | yes |
| Deletes the local branch | no | yes |

Use **`land`** immediately after a PR merges — single command, everything cleaned up.

Use **`learn`** alone when you want to inspect the created memory files on the branch before they land, or when the issue was already closed by GitHub automation and you only need the synthesis step.

## Reviewing synthesized learnings

After `engram plan learn` creates the branch, check each new file in `.engram/memory/`:

**`read_when` conditions** — should be concrete task phrases an agent will recognise at decision time:
- Good: `"calling claude -p programmatically from engram"`
- Bad: `"working with Claude"`

**Body** — should explain WHY, not WHAT. If the content could be found by reading the relevant source file for 5 minutes, the file should not exist. Source pointers (`see src/claude.rs:synthesize_learnings`) are better than copied code.

**Category** — verify it's the lowest applicable level:
- `patterns/` — approaches worth repeating across multiple future features
- `tripwires/` — things to avoid; past failures or surprises; always has a `tripwires:` YAML block
- `architecture/` — structural decisions affecting multiple modules; explains WHY a design choice exists
- `testing/` — test strategies reusable across the codebase

For the full extraction quality bar, see `references/memory-quality.md` in this skill.

## Prompt hooks

To influence how Claude categorises or phrases learnings for this repo, add `.md` files to `.engram/prompt-hooks/`. Each file's content is appended to the synthesis prompt.

Do not put a README or other documentation file there unless it is named exactly `README.md` — that name is the only one filtered out automatically (`src/claude.rs:load_prompt_hooks`). Any other `.md` file in that directory will be treated as a rule.

## After the learning PR merges

No further action is needed. The next time any agent starts a session, CLAUDE.md's `@.engram/memory/index.md` reference causes the updated index to load automatically. Future agents will see the new `read_when` entries.

Run `engram compact` periodically (once ≥10 memory files have accumulated) to prune stale or overlapping learnings.
