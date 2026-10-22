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

`gh` and a GitHub remote are only needed for the GitHub-backed commands (`plan`, `objective`, `compact`, `memory migrate`, `dispatch`). See [Dispatch](#dispatch) for its extra tools (kwt, tmux, kata, claude). `engram init` works in a local-only repo — no `origin` remote, no `gh` — and skips the GitHub label setup; the GitHub-backed commands then fail with a clear "this command requires GitHub" error.

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
engram memory migrate                      Rewrite memory files into OKF v0.2 frontmatter
engram capture --category <c> --title <t>  Write a memory file directly (body via --body or stdin)
engram capture ... --session               Same, marked `origin: session` as a mid-session observation

engram plan new <title> [--body <body>]    Create a GitHub issue as a plan
engram plan learn <issue> [--all]          Synthesize learnings from a closed issue+PR
engram plan land <issue>                   learn + close issue + delete local branch
engram plan list                           List open engram-plan issues
engram plan status                         Show linked issue/PR for the current branch

engram objective new <title> --body <body>         Create a multi-plan objective
engram objective plan <number> --node <id>         Create a plan for one node
engram objective view <number>                     Show objective and node statuses
engram objective list                              List open objectives

engram dispatch <issue>... [--dry-run] [--yes] [--no-launch]
                                           Start plans (or an objective's unblocked nodes) in background Claude sessions
engram dispatch --stack <objective>        Dispatch an objective's dependency chains as stacked PRs
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

This creates a GitHub issue tagged `engram-objective` with a rendered roadmap table and a hidden machine-readable node graph. Node IDs are free-form labels you choose (e.g. `1.1` or just `1`), used verbatim; nodes accept `(depends: 1.1, 1.2)` syntax for ordering.

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

## Dispatch

`engram dispatch` hands plan issues to background Claude sessions. Each plan gets its own branch, draft PR, kwt worktree and tmux workspace, and Claude starts there in auto mode with the plan as its prompt. The sessions show up in Ghosthub without taking over your terminal.

```
engram dispatch 42                 # one plan
engram dispatch 42 43 44           # several plans, each based on the default branch
engram dispatch 41                 # an objective: its unblocked nodes
engram dispatch 41 50              # any mix of plans and objectives
engram dispatch --stack 41         # an objective's dependency chains as stacked PRs
```

| Flag | Effect |
|------|--------|
| `--dry-run` | Print what would happen (kata ref, branch and base, PR title, PR, worktree, launch) and change nothing |
| `-y`, `--yes` | Skip the confirmation. Required without a terminal |
| `--no-launch` | Do everything except start Claude; the pane stays at a shell |
| `--stack` | Stacked dispatch of exactly one objective |

**Tools.** Dispatch is the only command that needs `kwt`, `tmux` and `kata` (with the repo bound by `kata init`), on top of `gh` and `claude`. It checks them all up front and names every problem; `engram doctor` reports the same as a non-failing line. If `kwt` is only a shell alias, the copy bundled at `/Applications/Ghosthub.app/Contents/Helpers/kwt` is used. `--stack` links PRs with the optional gh-stack extension.

**What one dispatch does**, after you confirm:

1. **kata**: find the plan's mirror (or create it with the same idempotency key as `plan new`) and claim it; only then append `Kata: <ref>` to a just-mirrored GitHub issue. A mirror claimed by someone else stops that plan before anything changes.
2. **Branch** from `origin/<base>` with one empty `wip: #N <title>` commit, without checking anything out, then push it and open a **draft PR** ending in `Closes #N`.
3. **Worktree**: `kwt add <branch> --no-launch`, then `kwt open --start-session` (detached).
4. **Launch** `claude --permission-mode auto` in the pane with a prompt built from the plan, the PR and how to finish (push, then `gh pr ready` with a summary). Dispatch waits until Claude has taken over the pane and reports it if Claude exits straight away.

**Modes.** A plan that belongs to an objective (`Objective: #N (node X)`) is refused while node X waits on unfinished nodes, or if that objective or node can't be found. An objective dispatches every node that isn't `done` and waits on nothing, creating missing plan issues first (as `engram objective plan` does) and marking dispatched nodes `in_progress`; blocked nodes are listed, and a node whose plan is already closed fails the plan checks (land it with `engram plan land`). `--stack` takes every unfinished node, dependencies first, each based on its unfinished dependency's branch (or, with several, on the one that depends on all the others; otherwise it is skipped as a *diamond*, with its dependents). Nodes whose PR is ready for review are only used as bases. Linear chains are linked with `gh stack link` (run with `GH_REPO`, since gh-stack can't resolve SSH host aliases); without gh-stack the PRs are still chained by their bases. A failed node skips its dependents. To dispatch only some of an objective's nodes, pass their plan numbers instead.

**Re-running is safe.** Every step looks up what exists first: mirror and claim, branch (or tracks `origin`'s), PR (through GitHub's closing link, else `Closes #N` in an open PR's body, which is how stacked PRs are found), worktree and session. A retitled plan or changed template doesn't start a second branch, a reused PR keeps its base (with a warning if it differs), Claude only launches into a pane at a shell prompt and not once the PR is ready for review, and a branch whose PR already merged is refused.

### Configuration

```toml
[dispatch]
branch = "{meta[type]}/{meta[jira]}/{slug}"   # default "{number}-{slug}"
pr_title = "[{meta[jira]}] {title}"           # default "{title}"
claude = "claude --model opus"                # default: claude from PATH
```

Placeholders: `{number}` (plan issue), `{kata_ref}` (the mirror's short ref, shown as `<kata-ref>` in a dry run until it exists), `{title}`, `{slug}` (lowercase and dashes, at most 40 characters, cut at a word, a leading `[5.1]` dropped) and `{meta[key]}` (kata metadata, e.g. `jira`, `type`, set by `plan new`/`objective plan` from `--jira`/`--type`, a body line or the title). `{{`/`}}` are literal braces. A missing key fails at planning with the fix (`kata meta set <kata-ref> jira ENG-1234`), and the branch name is checked with `git check-ref-format`. The old Python dispatcher's git-config templates are not read: move them here (`prTitle` → `pr_title`, `{short_id}` → `{kata_ref}`; the old default was `{short_id}-{slug}`).

`claude` is used verbatim; ` --permission-mode auto "$(cat <prompt>)"` is appended either way, and the prompt file lives under `$XDG_STATE_HOME/engram/dispatch/`, outside every worktree. `ENGRAM_DISPATCH_CLAUDE` overrides it, for tests.

**Multiple GitHub accounts.** If your active `gh` account can't see the repo, set `git config engram.githubUser <login>` (git config, since the identity is yours, not the repo's). Unless `GH_TOKEN` or `GITHUB_TOKEN` is already set, engram then passes `gh auth token --user <login>` as `GH_TOKEN` to its `gh` calls and to dispatch's `git` fetch/push/ls-remote, and never runs `gh auth switch`; an account that isn't logged in fails with a `gh auth login` hint (`engram doctor` shows it too). tmux panes don't inherit engram's environment, so the pane resolves the token itself: `cd <worktree> && GH_TOKEN="$(gh auth token --user '<login>')" && export GH_TOKEN && claude …`. The token never appears in keystrokes or scrollback, and a failed lookup leaves the pane at the shell (reported as a failed launch) instead of starting Claude as the wrong account. The pane's shell must accept POSIX assignments (not fish).

### From an auto-mode Claude session

`engram init` adds `Bash(engram dispatch:*)` to `.claude/settings.json` unless it (or `Bash(engram dispatch *)`) is already there, re-serialising the file with key order kept; a file it can't read, parse (e.g. JSONC) or write is left untouched with a warning, and init carries on. `engram doctor` reports whether `.claude/settings.json` or `.claude/settings.local.json` has the rule. The `engram-dispatch` skill dry-runs, confirms with you, then dispatches. The rule only matches commands that *start* with `engram dispatch`, so run it from the repo root with nothing in front (`cd … &&` or `GH_TOKEN=…` won't match); that is what `engram.githubUser` and `[dispatch] claude` are for. If the classifier still denies it, add an exception to your **user** `~/.claude/settings.json` (project settings can't carry `autoMode`):

```json
{
  "autoMode": {
    "allow": [
      "Running `engram dispatch` to start Claude sessions in kwt worktrees for engram plan issues is allowed when the user asked to dispatch them"
    ]
  }
}
```

---

## kata integration (optional)

[`kata`](https://kata.kenn.io) is a separate local-first issue ledger. Engram works standalone on GitHub Issues alone — kata is entirely optional. If you also run kata, we recommend mirroring every `engram-plan` and `engram-objective` issue into it, so agents have one searchable ledger across every tool in the kenn.io stack (kata, engram, roborev, kwt) instead of having to check GitHub separately.

Native mirroring (kata issues created/closed automatically by `plan`/`land`/`learn`) is tracked as its own objective and not yet built. Until then, mirror by hand:

```
kata create "<same title as the GitHub issue>" \
  --body-file <full-body-file> \
  --parent <kata ref of the objective, for a plan node> \
  --idempotency-key "engram-plan-<issue-number>"
```

**The kata issue body must contain the complete GitHub issue body** — every plan section (Why/Background/Approach/...) or objective section (Goal/Background/Roadmap/...) — prefixed with `GitHub: <issue URL>`. Don't just paste a link: kata is meant to be searched on its own, and a bare link forces a context switch back to GitHub on every hit. Use `--parent`/`--blocked-by` to mirror the same objective/dependency structure the GitHub issues have.

---

## Claude Code skills

`engram init` installs its skills into `.claude/skills/`. They are invocable as slash commands in Claude Code, among them:

### `/engram-plan <title>`

Guides you through writing a plan body before opening the issue. Claude reads the relevant source files first, drafts all seven sections, presents the draft for your review, and only creates the issue after you confirm.

```
/engram-plan Add rate limiting to the API
```

### `/engram-learn`

Explains the learn/land workflow, what gets written to memory, and how to review the synthesized files.

### `/engram-memory`

Helps you navigate, evaluate, and manually maintain `.engram/memory/`. Use it when deciding whether a file is worth keeping, or when running `engram compact`.

### `/engram-dispatch <issues>`

Dispatches plans or objectives from a Claude session, including one in auto mode: runs `engram dispatch <issues> --dry-run`, shows you the plan and confirms, then runs it with `--yes` and relays the summary. See [From an auto-mode Claude session](#from-an-auto-mode-claude-session).

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
