# Engram

Plan-based agentic development helper. CLI tool that wraps GitHub issues, Claude, and git into a structured learn/remember workflow.

## Layout

```
src/
  main.rs cli.rs        command routing and clap definitions only
  config.rs init.rs     .engram/config.toml; `engram init`
  guide.rs              the embedded guides behind `engram guide`
  process.rs date.rs    running external commands; calendar dates
  integrations/         the CLIs engram drives: github.rs (gh), claude.rs (claude -p), kata/
  memory/               .engram/memory topic files (mod.rs), capture, index (search), compact
  workflow/             the GitHub plan lifecycle: plan, objective, learn
  dispatch/             `engram dispatch` (see Dispatch below)
```

New code goes in the module for its domain; a module that outgrows one file becomes a directory with a `mod.rs`.

## Error handling

Use `anyhow::Result` and `anyhow::Error` everywhere. Never introduce `unwrap()` or `expect()` in non-test code.

```rust
// add context at every call boundary
let issue = github::get_issue(&repo, n).context("fetching issue")?;

// early exit with a message
anyhow::bail!("issue #{n} is not closed (state: {})", issue.state);
```

Do not introduce `thiserror` or custom error enums — the codebase has no HTTP layer that needs structured error variants.

## Derives

Always include `Debug`. Add others only when needed:

```rust
#[derive(Debug, Deserialize)]          // data coming in from JSON
#[derive(Debug, Serialize)]            // data going out to JSON
#[derive(Debug, Clone)]                // when ownership needs to be shared
#[derive(Debug, Serialize, Deserialize)] // round-trip types
```

Add `serde` attributes for field name mismatches or optional fields:

```rust
#[serde(rename = "createdAt")]
pub created_at: String,

#[serde(default)]
pub tripwires: Vec<Tripwire>,

#[serde(skip_serializing_if = "Option::is_none")]
pub body: Option<String>,
```

## External processes

All external I/O goes through `std::process::Command`. Never use raw API tokens or HTTP clients — use the `gh`, `git`, and `claude` CLIs, and run them through `src/process.rs`, which checks the exit status and puts stderr in the error:

```rust
// a command whose failure is an error: trimmed stdout, or `cmd` failed: <stderr>
let head = process::run_checked(process::git(repo_root).args(["rev-parse", "HEAD"]))?;

// GitHub — always via gh, with the configured identity (see github.rs `gh()`)
process::run_checked(config::gh_command()?.args([...]))?;

// git that talks to origin — process::git_remote applies the same identity
process::run_checked(process::git_remote(repo_root)?.args(["push", "-u", "origin", branch]))?;

// Claude — always claude::run_prompt, which runs `claude -p` from temp_dir()
let text = claude::run_prompt(&prompt)?;
```

`process::run_tolerating` accepts one known benign failure (e.g. "already exists"); `process::spawn` + `process::failure` are for the rare caller that reads the exit code itself. Plain `Command::new(...).output()` is only for availability probes such as `gh --version`.

## kata (optional issue mirror)

kata is a separate local-first issue ledger. It is optional — check for `.kata.toml` at the repo root before doing anything below. If it's absent, skip kata entirely.

When `.kata.toml` exists, mirror every `engram-plan` and `engram-objective` issue into kata as it is created or edited:

- Title: same as the GitHub issue title.
- Body: the **full** GitHub issue body — all seven plan sections, or Goal/Background/Roadmap/Acceptance criteria/Scope for an objective — prefixed with `GitHub: <issue URL>`. Never write just a link; kata must be searchable standalone.
- Relationships: a plan node's kata issue gets `--parent <objective's kata ref>`. Anything blocked on another objective/plan completing first gets `--blocked-by <that kata ref>`.
- Idempotency: use `--idempotency-key "engram-plan-<issue-number>"` or `"engram-objective-<issue-number>"` so retries don't duplicate issues.

Native kata mirroring for issue *creation* is implemented: `plan new`, `objective new`, and `objective plan` (both `--node` and `--all-unblocked`) each create the kata mirror automatically and set `--parent` where applicable (`--no-kata` skips it per-invocation). Those same commands set kata metadata `jira` and `type` (read as `{meta[jira]}`/`{meta[type]}` by `engram dispatch`'s `[dispatch]` branch/PR-title templates) from `--jira`/`--type`, else a `Jira:`/`Type:` body line, else the title (`[KEY-12] …`, `KEY-12: …`, `feat: …`); node plans inherit missing fields from their objective. Nothing is written when no key is found. `--blocked-by` between plans is only derived by `engram kata sync`, not at creation time. Automatic *close*/*comment* mirroring from `land`/`learn` beyond `plan land`'s existing kata close is still unimplemented — do that by hand until it lands.

## Skills and guides

The workflow content behind the `engram-*` skills lives in `guides/<topic>.md`, embedded in the binary (`src/guide.rs`) and printed by `engram guide <topic>`. `.claude/skills/engram-*/SKILL.md` are thin stubs that call `engram guide`; `engram init` installs exactly those stubs (`SKILL_STUBS` in `src/init.rs`) and doctor compares only them. Edit the guides, not the stubs. Guides must not use `$ARGUMENTS`: only the stub has it substituted. The learn and compact prompts embed `guides/memory-quality.md` and `guides/memory.md`, so editing those changes `claude -p` behaviour too.

## Dispatch

`engram dispatch` lives in `src/dispatch/` (`mod.rs` runs the pipeline; `inspect.rs` plans a dispatch, `execute.rs` carries it out, `workspace.rs` owns kwt/tmux/the Claude launch, `stack.rs` is `--stack`, plus `preflight.rs`, `template.rs` and `render.rs`): collect plan issues (`collect_work`, or `collect_stack_work` for `--stack`) → `plan_dispatch` → confirm → `carry_out`. A stack only adds an order (`order_for_stack`) and a base per node (`base_branch`). Planning never mutates anything: `--dry-run` is planning plus an early return, so every new check belongs there, not in `carry_out`. Keep pure logic (`fill_template`, `branch_slug`, `order_for_stack`, `straight_chains`, `ready_nodes`, `merge_node_progress`, `format_plan`, `build_prompt`, `launch_command`, kwt JSON parsing) in plain functions and unit-test those with literal inputs; never spawn or mock gh/git/kata/kwt/tmux in tests. Check side effects by hand with `--dry-run`, then `--no-launch` on a throwaway plan.

**Preflight is dispatch-only.** Only `cmd_dispatch` calls `preflight` (gh, kata, kwt, tmux, claude, `.kata.toml`); `doctor` shows `unavailable_reasons` as a non-failing line. kwt, tmux and claude are called through the resolved `Toolchain` paths.

**tmux/kwt invariants**, learned the hard way in the Python dispatcher this replaced. Don't simplify them away:

- Remove `TMUX_TMPDIR` from every tmux call and address tmux as `-L <tmux_socket_name>` with the session from `kwt open --start-session --json` (kwt runs its own server; never use `tmux ls`).
- `--start-session` is the only `kwt open` mode that never attaches the terminal; it fails transiently ("fork failed") and is idempotent, so it is retried (`KWT_OPEN_ATTEMPTS`).
- The launch command starts with `cd <worktree> &&` (the pane may sit in a deleted directory), is only typed into a pane running a shell, and counts as launched only once Claude has taken the pane over and is still there after `LAUNCH_SETTLE`.
- kwt falls back to the `GHOSTHUB_KWT` path, since it is often only a shell alias. Worktrees are found by exact branch in `kwt list --json`; `kwt get` matches fuzzily.
- The prompt file lives under `$XDG_STATE_HOME/engram/dispatch/`, outside every worktree. The claude command is `ENGRAM_DISPATCH_CLAUDE` (tests), else `[dispatch] claude`, else the resolved claude path; the flags and prompt are always appended.
- Branches are made with `git commit-tree` + `git branch` on the fetched `origin/<base>`, never by checkout. `gh stack` runs with `GH_REPO`, since gh-stack can't map SSH host aliases.

**Plain command line.** `engram dispatch <args>` must work exactly as typed: auto mode denies command lines that configure an agent or pass a credential (a `Bash(engram dispatch:*)` allow rule is optional and never installed by engram). Configuration goes in `.engram/config.toml` or git config (`engram.githubUser`). GitHub-facing `gh` calls and `git` calls that talk to `origin` (`process::git_remote`) go through `config::gh_command()`/`with_github_identity`; the pane resolves its own token with `GH_TOKEN="$(gh auth token --user …)" && export GH_TOKEN && …` so a failed lookup never starts Claude. Never write a token into keystrokes, prompt files, logs or errors.

**Idempotency.** Every `carry_out` step looks up existing state first, so a re-run resumes. Keep `Closes #N` in the draft PR body: GitHub never links stacked PRs, so re-runs find them by that line. Never steal another actor's kata claim.

## Constants

Prefer file-level `const` over magic literals:

```rust
const ENGRAM_START: &str = "<!-- engram:start -->
## Engram Memory

@.engram/memory/index.md

<!-- engram:end -->
