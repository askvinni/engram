# engram

A Rust CLI for plan-based agentic development. Manages GitHub Issues as units of work, learns from closed issue+PR pairs using Claude, and surfaces that knowledge as categorized memory in the repo.

## Prerequisites

- [`gh`](https://cli.github.com/) — authenticated with `gh auth login`
- [Claude Code](https://claude.ai/code) — installed and authenticated

## Commands

```
engram init                        # Initialize in this repo
engram plan <title> [--body <...>] # Create a GitHub issue as a plan
engram learn <issue-number>        # Synthesize learnings from a closed issue+PR
```

## Workflow

1. `engram plan "do X"` — creates a GitHub issue tagged `engram-plan`
2. Implement the work, open a PR with `closes #N` in the body
3. Merge the PR
4. `engram learn N` — synthesizes learnings into `.engram/memory/`, updates `CLAUDE.md`, opens a PR tagged `engram-learned`

## Storage

- `.engram/config.toml` — repo config
- `.engram/memory/<category>.md` — categorized learnings (patterns, tripwires, architecture, testing)
- `CLAUDE.md` — auto-updated with current memory between `<!-- engram:start -->` / `<!-- engram:end -->`
