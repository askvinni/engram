# Engram

Plan-based agentic development helper. CLI tool that wraps GitHub issues, Claude, and git into a structured learn/remember workflow.

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

## kata (optional issue mirror)

kata is a separate local-first issue ledger. It is optional — check for `.kata.toml` at the repo root before doing anything below. If it's absent, skip kata entirely.

When `.kata.toml` exists, mirror every `engram-plan` and `engram-objective` issue into kata as it is created or edited:

- Title: same as the GitHub issue title.
- Body: the **full** GitHub issue body — all seven plan sections, or Goal/Background/Roadmap/Acceptance criteria/Scope for an objective — prefixed with `GitHub: <issue URL>`. Never write just a link; kata must be searchable standalone.
- Relationships: a plan node's kata issue gets `--parent <objective's kata ref>`. Anything blocked on another objective/plan completing first gets `--blocked-by <that kata ref>`.
- Idempotency: use `--idempotency-key "engram-plan-<issue-number>"` or `"engram-objective-<issue-number>"` so retries don't duplicate issues.

Native kata mirroring for issue *creation* is implemented: `plan new`, `objective new`, and `objective plan` (both `--node` and `--all-unblocked`) each create the kata mirror automatically and set `--parent` where applicable (`--no-kata` skips it per-invocation). Those same commands set kata metadata `jira` and `type` (read by kdispatch's branch/PR-title templates) from `--jira`/`--type`, else a `Jira:`/`Type:` body line, else the title (`[KEY-12] …`, `KEY-12: …`, `feat: …`); node plans inherit missing fields from their objective. Nothing is written when no key is found. `--blocked-by` between plans is only derived by `engram kata sync`, not at creation time. Automatic *close*/*comment* mirroring from `land`/`learn` beyond `plan land`'s existing kata close is still unimplemented — do that by hand until it lands.

## Constants

Prefer file-level `const` over magic literals:

```rust
const ENGRAM_START: &str = "<!-- engram:start -->
## Engram Memory

@.engram/memory/index.md

<!-- engram:end -->
