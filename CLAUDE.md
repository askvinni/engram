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
const ENGRAM_START: &str = "<!-- engram:start -->
## Engram Memory

@.engram/memory/index.md

<!-- engram:end -->
