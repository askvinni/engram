# engram

Engram is a CLI that turns GitHub Issues into a structured learning loop for AI-assisted development. Each unit of work starts as a plan issue, ships as a PR, and lands as categorized memory that future Claude sessions can read — so the AI gets smarter about your codebase over time instead of starting blank every session.

## How it works

```
engram plan new "Add rate limiting"   →  GitHub issue #42 (engram-plan label)
git checkout -b my-feature
# ... implement, open PR with "closes #42" in the body, merge ...
engram plan land 42                   →  memory files written, issue closed, branch deleted
```

After `land`, the next Claude session in this repo reads `.engram/memory/` and already knows the patterns, gotchas, and architectural decisions from that work.

---

## The Tao of Engram

### The real problem with AI-assisted development

Agentic coding tools have become genuinely useful. But there is a structural problem that no amount of model capability improvement will solve on its own: **AI starts every session blank.**

Every time you open a new Claude Code session, the agent has no idea what your codebase learned last week. It doesn't know that the GraphQL mutation API takes opaque node IDs, not integers. It doesn't know that calling `claude -p` from inside a repo directory loads `CLAUDE.md` as agent context and changes its behavior. It doesn't know that you tried the obvious approach to the auth middleware three months ago and it failed for a specific compliance reason.

Without a system that captures and routes that knowledge, you pay the same tax over and over: the agent makes the same class of mistake, you correct it, and the correction evaporates when the session ends. The more your codebase evolves, the worse this gets.

Engram's answer is a structured learning loop that accumulates institutional memory directly in your repository — memory that agents self-load at the right moment, without you doing anything.

### Plans are context transfer, not task tracking

Engram uses GitHub Issues as plan records. But the point of a plan issue is not to track tasks — it's to move context from your head into a form that can be synthesized later.

A plan that says `"fix the auth bug"` will produce no useful memory when it lands. A plan with a seven-section body — explaining *why the current behavior is wrong*, *which files are involved*, *what tradeoff was made in the approach*, *what was explicitly ruled out and why* — will produce memory files that change how the next agent session approaches similar work.

The quality of the plan determines the quality of the memory. This is the most important property of the system. The `/engram-plan` skill exists to enforce it: it reads the relevant source files before drafting anything, produces all seven sections grounded in what it found, and refuses to create the issue until the draft is reviewed. The friction is intentional.

### The planning lifecycle

```
Plan → Implement → Land → Remember
```

**Plan.** You write a plan issue. The body records why the change is needed, what the approach is, where the risk is, and what done looks like. This is the context that will be synthesized after the PR merges.

**Implement.** You work normally — branch, code, open a PR with `closes #N` in the body, iterate, merge. Engram is invisible during this phase. The only requirement is the close marker, which tells GitHub to link the PR to the issue so `land` can find it.

**Land.** After the PR merges, you run `engram plan land <N>`. This calls Claude with the full issue body and the merged PR diff, asks it to extract cross-cutting learnings, and writes them to `.engram/memory/` categorised by type. It opens a new PR with those memory files for you to review and merge.

**Remember.** The merged memory files are injected into `CLAUDE.md` via a routing index. The next time an agent session opens in this repo, it reads that index and self-loads the files whose `read_when` conditions match what it is currently doing. The agent begins the session knowing what the last ten PRs taught.

Each cycle deposits a small amount of specific, verified knowledge. Over months, this accumulates into a picture of the codebase that no individual context window could hold.

### Memory quality over memory quantity

Not everything that happens during implementation is worth remembering. A memory file that describes *how the code works* is worse than useless — it duplicates the source, adds maintenance burden, and dilutes the signal for agents routing their context. The code already documents how it works. Memory is for what the code cannot tell you.

A memory file earns its place by recording one of three things:

- **A discovered failure**: the approach you tried that didn't work, and specifically why it didn't work. Future agents won't repeat it.
- **A non-obvious external constraint**: the API that accepts only node IDs, the CLI that changes behavior based on working directory, the flag that silently ignores your input. Things that can only be known by being burned by them.
- **A structural decision and its reason**: not just *what* the architecture is, but *why* — the constraint that was being satisfied, the alternative that was rejected, the thing that would break if someone changed it.

`engram compact` enforces this bar. It audits every memory file, deletes those that merely describe the code, merges redundant files, and flags ones that are too generic to route accurately. Run it every few learning cycles. A small, high-signal memory directory is more valuable than a large, low-signal one.

### The routing model

Memory files are not loaded wholesale into every session. Each file has a `read_when` field — a list of conditions describing when the file is relevant. Agents read the routing index (a single lightweight table in `CLAUDE.md`) and self-select which files to load based on what they are currently doing.

This design scales. A codebase that has been under active development for a year may have dozens of memory files. An agent working on the auth layer should not have to read a tripwire about the build system's cache invalidation behaviour. `read_when` conditions are specific: *"adding a new gh CLI wrapper that must be idempotent"*, not *"working with GitHub"*.

Claude generates these conditions during synthesis. If a plan body's Approach section is vague — *"refactor the handler"* — the synthesized `read_when` will be vague too. Specific plans produce specific routing, which produces accurate memory loading.

### Objectives for multi-PR coordination

Some work cannot fit in a single PR without becoming unreviewable. A large migration, a multi-phase refactor, a new subsystem — these span weeks and multiple branches of work that may even run in parallel.

Objectives let you group related plans under a shared goal while preserving the per-plan learning granularity. Each node in an objective roadmap is its own plan issue with its own PR and its own memory output. The objective issue tracks which nodes are done, which are in progress, and which are blocked by unfinished dependencies.

When a plan that was created via an objective lands, engram automatically marks its node as done in the objective issue. When every node is done, the objective closes itself. There is no manual bookkeeping.

---

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
engram init                                Initialize in this repo; install Claude skills
engram doctor                              Verify all dependencies and configuration
engram compact                             Prune and merge stale memory files

engram plan new <title> [--body <body>]    Create a GitHub issue as a plan
engram plan learn <issue> [--all]          Synthesize learnings from a closed issue+PR
engram plan land <issue>                   learn + close issue + delete local branch
engram plan list                           List open engram-plan issues
engram plan status                         Show linked issue/PR for the current branch

engram objective new <title> --body <body>         Create a multi-plan objective
engram objective plan <number> --node <id>         Create a plan for one node
engram objective view <number>                     Show objective and node statuses
engram objective list                              List open objectives
```

---

## Workflow

### 1. Create a plan

```
engram plan new "Add rate limiting to the API"
```

This opens a GitHub issue tagged `engram-plan`. The issue body should cover seven sections that determine the quality of the memory files produced when the work lands (see [Plan body format](#plan-body-format) below). If you're using Claude Code, the `/engram-plan` skill walks you through writing a thorough body.

### 2. Implement and ship

Create a branch and open a PR. The PR body **must** include `closes #N` (or `fixes #N` / `resolves #N`) so GitHub links the PR to the issue — `engram plan learn` finds the PR via that close event. The branch can be named anything.

### 3. Land the work

After the PR merges:

```
engram plan land 42
```

`land` does three things:
1. Calls Claude to synthesize cross-cutting learnings from the issue and PR diff into `.engram/memory/`
2. Closes the issue (if not already auto-closed by GitHub)
3. Deletes the local branch

It opens a PR tagged `engram-learned` with the new memory files. Review and merge that PR to commit the learnings to the repo.

If you want to inspect the memory files before closing the issue, use `engram plan learn 42` first, then `engram plan land 42` after reviewing.

### 4. Batch-process unlearned issues

```
engram plan learn --all
```

Processes all closed `engram-plan` issues that haven't been learned yet in a single branch and PR. Useful after a burst of shipping.

### 5. Compact periodically

```
engram compact
```

Over time, memory accumulates files that describe *how the code works* rather than *what not to do* — these are better read directly from the source. `compact` calls Claude to audit every memory file and delete or merge ones that don't pass the quality bar. Run it after every few `learn` cycles.

---

## Objectives

For work that spans multiple PRs — a migration, a multi-phase feature, a refactor — objectives let you group related plans under a shared goal and track progress as each piece lands.

### Create an objective

```
engram objective new "Migrate auth layer" --body "## Goal
Replace legacy session tokens with JWTs across all services.

## Design Decisions
- Keep auth middleware thin; push token logic into a dedicated module
- Do not change the public API shape during migration

## Roadmap
- 1.1: Extract token validation into auth module
- 1.2: Migrate session storage (depends: 1.1)
- 1.3: Remove legacy token code (depends: 1.2)
"
```

This creates a GitHub issue tagged `engram-objective` with a rendered roadmap table and a hidden machine-readable node graph. Nodes accept `(depends: 1.1, 1.2)` syntax for ordering.

### Start work on a node

```
engram objective plan 42 --node 1.1
```

Creates an `engram-plan` issue for node 1.1 and marks it `in_progress` in the objective. If you omit `--body`, Claude generates the plan body from the node description and objective context. You can also provide one explicitly:

```
engram objective plan 42 --node 1.1 --body "**Why**\n..."
```

### Track progress

```
engram objective view 42
engram objective list
```

`view` prints the goal, design decisions, and current roadmap table showing each node's status, linked plan issue, and which nodes are blocked by unfinished dependencies.

### Automatic status updates

When you run `engram plan land <plan>` or `engram plan learn <plan>` on a plan that was created via `objective plan`, engram automatically marks the corresponding node as `done` in the objective issue. No manual update needed.

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

The Approach section is what `engram plan learn` draws on most heavily when synthesizing pattern and architecture memory files. Vague bullets produce vague memory.

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

`.engram/prompt-hooks/` contains Markdown files injected into the Claude prompt during `engram plan learn`. Use them to customize how learnings are classified for your repo:

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
  prompt-hooks/            Per-repo rules injected into engram plan learn prompts

.claude/
  skills/
    engram-plan/           /engram-plan slash command
    engram-learn/          /engram-learn slash command
    engram-memory/         /engram-memory slash command

CLAUDE.md                  Auto-updated with link to memory index
```
