# engram

Engram is a CLI that turns GitHub Issues into a structured learning loop for AI-assisted development. Each unit of work starts as a plan issue, ships as a PR, and lands as categorized memory that future Claude sessions can read — so the AI gets smarter about your codebase over time instead of starting blank every session.

## How it works

```
engram plan "Add rate limiting"   →  GitHub issue #42 (engram-plan label)
git checkout -b feat/issue-42
# ... implement, open PR with "closes #42" in the body, merge ...
engram land 42                    →  memory files written, issue closed, branch deleted
```

After `land`, the next Claude session in this repo reads `.engram/memory/` and already knows the patterns, gotchas, and architectural decisions from that work.

## Prerequisites

- [`gh`](https://cli.github.com/) authenticated with `gh auth login`
- [Claude Code](https://claude.ai/code) installed and authenticated

## Installation

Requires Rust (stable):

```
cargo install --git https://github.com/askvinni/engram
```

Then initialize engram in each project you want to track:

```
cd your-project
engram init
```

`engram init` creates `.engram/` config and memory directories, ensures the required GitHub labels exist, and installs Claude Code skills into `.claude/skills/`. After upgrading engram, re-run `engram init` to update the installed skills. `engram doctor` will flag stale skills with `✗ claude skills current`.

---

## Commands

```
engram init                          Initialize in this repo; install Claude skills
engram plan <title> [--body <body>]  Create a GitHub issue as a plan
engram learn <issue> [--all]         Synthesize learnings from a closed issue+PR
engram land <issue>                  learn + close issue + delete local branch
engram list                          List open engram-plan issues
engram status                        Show linked issue/PR for the current branch
engram compact                       Prune and merge stale memory files
engram doctor                        Verify all dependencies and configuration
```

---

## Workflow

### 1. Create a plan

```
engram plan "Add rate limiting to the API"
```

This opens a GitHub issue tagged `engram-plan`. The issue body should cover seven sections that determine the quality of the memory files produced when the work lands (see [Plan body format](#plan-body-format) below). If you're using Claude Code, the `/engram-plan` skill walks you through writing a thorough body.

### 2. Implement and ship

Create a branch and open a PR. The PR body **must** include `closes #N` (or `fixes #N` / `resolves #N`) so GitHub links the PR to the issue — `engram learn` finds the PR via that close event.

Recommended branch naming so `engram land` can clean up automatically:
- `feat/issue-42`
- `fix/issue-42`
- `issue-42`

### 3. Land the work

After the PR merges:

```
engram land 42
```

`land` does three things:
1. Calls Claude to synthesize cross-cutting learnings from the issue and PR diff into `.engram/memory/`
2. Closes the issue (if not already auto-closed by GitHub)
3. Deletes the local branch

It opens a PR tagged `engram-learned` with the new memory files. Review and merge that PR to commit the learnings to the repo.

If you want to inspect the memory files before closing the issue, use `engram learn 42` first, then `engram land 42` after reviewing.

### 4. Batch-process unlearned issues

```
engram learn --all
```

Processes all closed `engram-plan` issues that haven't been learned yet in a single branch and PR. Useful after a burst of shipping.

### 5. Compact periodically

```
engram compact
```

Over time, memory accumulates files that describe *how the code works* rather than *what not to do* — these are better read directly from the source. `compact` calls Claude to audit every memory file and delete or merge ones that don't pass the quality bar. Run it after every few `learn` cycles.

---

## Claude Code skills

`engram init` installs three skills into `.claude/skills/`. These are invocable as slash commands in Claude Code:

### `/engram-plan <title>`

Guides you through writing a plan body before opening the issue. Claude reads the relevant source files first, drafts all seven sections, presents the draft for your review, and only creates the issue after you confirm.

```
/engram-plan Add rate limiting to the API
```

### `/engram-learn`

Explains the learn/land workflow, what gets written to memory, and how to review the synthesized files.

### `/engram-memory`

Helps you navigate, evaluate, and manually maintain `.engram/memory/`. Use it when deciding whether a file is worth keeping, or when running `engram compact`.

---

## Plan body format

The plan body has seven sections. All are required — missing sections produce a warning but don't block issue creation. The `/engram-plan` skill writes these for you.

| Section | What goes here |
|---------|---------------|
| **Why** | One falsifiable sentence: what is broken or missing today |
| **Background** | 2–4 sentences for a cold reader: which files, what the UX looks like before the change |
| **Approach** | 3–6 bullets on technical strategy: which module, what data flows where, key tradeoff |
| **Acceptance criteria** | Checkable outcomes: each item verifiable by running a command or reading a diff |
| **Scope** | Which files change; which adjacent files explicitly do *not* change |
| **Edge cases and risks** | What can go wrong; what inputs need special handling; what's explicitly out of scope |
| **Key files** | 3–6 `src/foo.rs:FunctionName` entry points for a cold reader to orient |

The Approach section is what `engram learn` draws on most heavily when synthesizing pattern and architecture memory files. Vague bullets produce vague memory.

---

## Memory

Learnings are written to `.engram/memory/<category>/<slug>.md` and indexed in `.engram/memory/index.md`. The index is injected into `CLAUDE.md` so Claude Code loads it automatically. Individual topic files are routed via `read_when` conditions — agents self-select which files to load rather than reading everything.

**Categories:**

| Category | What belongs here |
|----------|------------------|
| `patterns` | Recurring solutions and idioms discovered during implementation |
| `tripwires` | Gotchas, wrong approaches, bugs that bit you — things not to repeat |
| `architecture` | Non-obvious structural decisions and the reasons behind them |
| `testing` | Test strategies, fixtures, and constraints specific to this codebase |

Memory files have a high quality bar — `engram compact` enforces it. A file earns its place by recording a discovered failure, capturing non-obvious external behaviour, or explaining *why* an architectural decision was made. Files that just describe how the code works get pruned.

---

## Prompt hooks

`.engram/prompt-hooks/` contains Markdown files injected into the Claude prompt during `engram learn`. Use them to customize how learnings are classified for your repo:

```markdown
<!-- .engram/prompt-hooks/classify.md -->
Always classify Rust lifetime errors as tripwires.
Prefer architecture entries for any change to the public API surface.
```

Files are loaded in alphabetical order. Only `.md` files are included. Commit this directory so the rules are shared across the team.

---

## File layout

```
.engram/
  config.toml              Repo configuration
  memory/
    index.md               Auto-generated routing index (read by Claude)
    patterns/              Recurring implementation patterns
    tripwires/             Gotchas and wrong approaches
    architecture/          Structural decisions and rationale
    testing/               Test strategies and constraints
  prompt-hooks/            Per-repo rules injected into engram learn prompts

.claude/
  skills/
    engram-plan/           /engram-plan slash command
    engram-learn/          /engram-learn slash command
    engram-memory/         /engram-memory slash command

CLAUDE.md                  Auto-updated with link to memory index
```
