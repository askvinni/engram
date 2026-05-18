# engram

A Rust CLI for plan-based agentic development. Manages GitHub Issues as units of work, learns from closed issue+PR pairs using Claude, and surfaces that knowledge as categorized memory in the repo.

## Installation

Requires Rust (stable). Install directly from the repository:

```
cargo install --git https://github.com/askvinni/engram
```

Then initialize engram in each project you want to use it with:

```
cd your-project
engram init
```

`engram init` sets up `.engram/` config and memory directories, ensures the required GitHub labels exist, and installs Claude Code skills into `.claude/skills/`. The skills teach Claude how to write good plans, run the learn workflow, and work with memory files — so they need to be in each project's directory, not just in the engram source repo.

After upgrading engram, re-run `engram init` in each project to update the installed skills to the new version. `engram doctor` will flag a `✗ claude skills current` failure if they're out of date.

## Prerequisites

- [`gh`](https://cli.github.com/) — authenticated with `gh auth login`
- [Claude Code](https://claude.ai/code) — installed and authenticated

## Commands

```
engram init                         # Initialize in this repo; install Claude skills
engram plan <title> [--body <...>]  # Create a GitHub issue as a plan
engram learn <issue-number>         # Synthesize learnings from a closed issue+PR
engram land <issue-number>          # learn + close issue + delete local branch
engram list                         # List open engram-plan issues
engram status                       # Show linked issue/PR for the current branch
engram compact                      # Prune and merge stale memory files
engram doctor                       # Verify all dependencies and configuration
```

## Workflow

1. `engram plan "do X"` — creates a GitHub issue tagged `engram-plan`
2. Implement the work, open a PR with `closes #N` in the body
3. Merge the PR
4. `engram land N` — synthesizes learnings into `.engram/memory/`, updates `CLAUDE.md`, opens a PR tagged `engram-learned`, closes the issue, and deletes the local branch

Run `engram compact` periodically to prune memory files that no longer meet the quality bar.

## Storage

- `.engram/config.toml` — repo config
- `.engram/memory/<category>/<slug>.md` — categorized learnings (patterns, tripwires, architecture, testing)
- `.claude/skills/` — Claude Code skills installed by `engram init`
- `CLAUDE.md` — auto-updated with memory index between `<!-- engram:start -->` / `<!-- engram:end -->`
