# Engram

Plan-based agentic development helper. CLI tool that wraps GitHub issues, Claude, and git into a structured learn/remember workflow.

## Project layout

```
src/
  main.rs          — command dispatch and all cmd_* handlers
  cli.rs           — clap struct and Commands enum
  config.rs        — Config struct, find_repo_root()
  github.rs        — all gh CLI wrappers (issues, PRs, labels)
  claude.rs        — claude -p invocations and JSON parsing
  memory.rs        — .engram/memory/ file I/O, index, CLAUDE.md section
  learn.rs         — learn workflow (fetch issue → synthesize → commit → PR)
  compact.rs       — compact workflow (audit memory → prune → commit → PR)
```

No `lib.rs` — this is a pure binary crate. All modules are internal.

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

All external I/O goes through `std::process::Command`. Never use raw API tokens or HTTP clients — use the `gh`, `git`, and `claude` CLIs:

```rust
// GitHub — always via gh CLI
let output = Command::new("gh").args([...]).output()?;

// Claude — always via claude -p, always from temp_dir()
let output = Command::new("claude")
    .args(["-p", &prompt, "--output-format", "text"])
    .current_dir(std::env::temp_dir())   // avoid loading repo CLAUDE.md
    .output()?;
```

Check `output.status.success()` and surface stderr as the error message.

## Constants

Prefer file-level `const` over magic literals:

```rust
const ENGRAM_START: &str = "<!-- engram:start -->";
const MAX_DIFF_BYTES: usize = 8_000;
```

## Modules and visibility

Keep everything `pub` only when another module needs it. Internal helpers stay private. There are no re-exports — callers use the full path (`memory::rebuild_index`, not a re-export).

## Tests

`#[cfg(test)] mod tests` goes at the **end of the file**, after all production code.

Use `#[test]` for sync tests. Use `tempfile::tempdir()` for any test that touches the filesystem:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // ...
    }
}
```

Test helpers that are shared across modules go in a `fn make_*()` helper inside the test module that needs them — no shared test_helpers module (the codebase is small enough).

Do not test functions that shell out to `gh`, `git`, or `claude` — those stay as manual/E2E.

## Toolchain and CI

The CI uses the **stable** Rust toolchain. Always format with `cargo +stable fmt`, not nightly. Nightly rustfmt wraps lines differently and will fail CI.

CI runs three jobs in parallel:
- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo fmt --check`

All three must pass before merging. Fix clippy warnings rather than suppressing them; `#[allow(...)]` is a last resort.

## Comments

Use `///` for public items. Skip comments that just restate the function name. Only explain non-obvious WHY:

```rust
// Run from a temp dir so Claude Code doesn't pick up the repo's CLAUDE.md
// and try to act on it rather than just synthesizing JSON.
let output = Command::new("claude")
    .current_dir(std::env::temp_dir())
    ...
```

No multi-line comment blocks. One sentence max per comment.

## Engram workflow

Plans are GitHub issues labelled `engram-plan`. After a plan's PR merges, run `engram learn <issue>` (or `engram land <issue>` to also close and clean up) to synthesize learnings into `.engram/memory/`. Run `engram compact` periodically to prune stale memory files.

<!-- engram:start -->
## Engram Memory

@.engram/memory/index.md

<!-- engram:end -->
