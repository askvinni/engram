use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct Tripwire {
    pub action: String,
    pub warning: String,
}

#[derive(Debug, Deserialize)]
pub struct LearningItem {
    pub category: String,
    pub slug: String,
    pub title: String,
    pub read_when: Vec<String>,
    #[serde(default)]
    pub tripwires: Vec<Tripwire>,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct CompactAction {
    pub action: String, // "delete", "keep", "merge_into"
    pub category: String,
    pub slug: String,
    pub reason: Option<String>,
    pub target_category: Option<String>,
    pub target_slug: Option<String>,
    pub target_updated_body: Option<String>,
}

fn strip_code_fence(s: &str) -> String {
    let mut lines = s.lines();
    if let Some(first) = lines.next() {
        if first.starts_with("```") {
            let inner: Vec<&str> = lines.take_while(|l| !l.starts_with("```")).collect();
            return inner.join("\n");
        }
    }
    s.to_string()
}

pub fn load_prompt_hooks(repo_root: &std::path::Path) -> String {
    let hooks_dir = repo_root.join(".engram/prompt-hooks");
    if !hooks_dir.exists() {
        return String::new();
    }
    let mut entries: Vec<_> = std::fs::read_dir(&hooks_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
        .collect();
    entries.sort_by_key(|e| e.path());
    entries
        .iter()
        .filter(|e| e.file_name() != "README.md")
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn synthesize_learnings(
    issue_title: &str,
    issue_body: &str,
    pr_title: &str,
    pr_body: &str,
    pr_diff: &str,
    current_memory: &str,
    prompt_hooks: &str,
) -> Result<Vec<LearningItem>> {
    let diff = if pr_diff.len() > 8000 {
        &pr_diff[..8000]
    } else {
        pr_diff
    };

    let memory_section = if current_memory.is_empty() {
        "_none yet_".to_string()
    } else {
        current_memory.to_string()
    };

    let hooks_section = if prompt_hooks.is_empty() {
        String::new()
    } else {
        format!("\n## Project-Specific Rules\n{prompt_hooks}\n")
    };

    let prompt = format!(
        r#"You are analyzing a completed GitHub issue and its associated pull request to extract learnings for an AI agent memory system.

## Closed Issue: {issue_title}

{issue_body}

## Merged PR: {pr_title}

{pr_body}

## PR Diff
{diff}

## Current Memory
{memory_section}
{hooks_section}
---

## Knowledge placement hierarchy (use the LOWEST level that fits)
1. Type artifacts (constants, enums) — put it there
2. Code comments — put it there
3. Docstrings — put it there
4. Learned docs — ONLY for cross-cutting insight spanning multiple files

Never extract: import paths, function signatures, single-file knowledge, symbol catalogs.

## Content rules
- Write for AI agents, not humans
- Capture WHY, not WHAT (agents can read source for WHAT)
- Never reproduce source code except: data formats, third-party API quirks, anti-patterns clearly labelled WRONG, CLI examples
- Use source pointers (e.g. "see src/github.rs:find_linked_pr") over code blocks

## Categories
- patterns: successful approaches worth repeating across multiple files/features
- tripwires: things to avoid; past failures or gotchas that span multiple callsites
- architecture: structural or design decisions affecting multiple modules
- testing: testing strategies applicable across the codebase

Extract 1–4 learnings. Only include a learning if it is cross-cutting (would be useful in at least 2 different future situations). Skip trivial or single-file insights.

For tripwire-category items, populate the tripwires array with structured action/warning pairs. The action must start with a gerund (e.g. "Calling", "Invoking", "Using") or "Before". The warning must be imperative (tell the agent what to do instead).

Return ONLY a JSON array, no other text:
[
  {{
    "category": "tripwires",
    "slug": "claude-repo-context",
    "title": "Run claude -p from temp_dir to avoid repo CLAUDE.md",
    "read_when": [
      "about to invoke claude -p programmatically",
      "building a tool that shells out to Claude Code"
    ],
    "tripwires": [
      {{
        "action": "Invoking claude -p from inside a repo directory",
        "warning": "CLAUDE.md gets loaded as context; always use current_dir(temp_dir()) for non-interactive calls"
      }}
    ],
    "body": "One paragraph explaining WHY this matters and the cross-cutting context. No code blocks."
  }},
  {{
    "category": "patterns",
    "slug": "graphql-closed-event",
    "title": "Use GraphQL CLOSED_EVENT to find the PR that closed an issue",
    "read_when": [
      "implementing GitHub API integration",
      "looking up which PR closed a given issue"
    ],
    "tripwires": [],
    "body": "One paragraph explaining WHY this approach is better than alternatives."
  }}
]"#
    );

    // Run from a temp dir so Claude Code doesn't pick up the repo's CLAUDE.md
    // and try to act on it rather than just synthesizing JSON.
    let output = Command::new("claude")
        .args(["-p", &prompt, "--output-format", "text"])
        .current_dir(std::env::temp_dir())
        .output()
        .context("running claude CLI (is Claude Code installed and authenticated?)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude -p failed: {}", stderr.trim());
    }

    let text = String::from_utf8(output.stdout)?;

    // Strip markdown code fences (```json ... ``` or ``` ... ```) if present
    let stripped = strip_code_fence(text.trim());

    let json_start = stripped.find('[').context("claude output contained no JSON array")?;
    let json_end = stripped.rfind(']').context("claude output had no closing ]")?;
    let json = &stripped[json_start..=json_end];

    serde_json::from_str(json).context("parsing learning items from claude output")
}

pub fn compact_learnings(topics: &[crate::memory::TopicFile]) -> Result<Vec<CompactAction>> {
    if topics.is_empty() {
        return Ok(Vec::new());
    }

    let files_section: String = topics
        .iter()
        .map(|t| format!("### {}/{}\n{}\n", t.category, t.slug, t.content))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        r#"You are auditing an AI agent memory system. Each file is a "learned doc" — knowledge stored to help future agents make better decisions.

## Keep standard
Keep a file ONLY if it would prevent a future agent from making a mistake or wrong design decision that is NOT obvious from reading the source code. Good candidates:
- Tripwires: non-obvious gotchas with a clear "do this instead"
- Architectural WHY that can't be inferred from a single file
- Patterns that save real investigation time across multiple future tasks

## Delete if
- The insight is self-evident from the existing code structure
- It documents an implementation choice already visible to any reader of the source
- It's a task log ("we did X for issue N") rather than future guidance
- An agent starting a new feature would naturally arrive at this approach without the hint

## Merge if
Two files cover the same core insight from different angles. Provide a combined body paragraph that synthesises both; the target file survives, the source is deleted.

Note: merge targets must NOT themselves be merge_into sources in the same response.

## Files to audit

{files_section}

---

Return ONLY a JSON array where every file appears exactly once:
[
  {{"action": "keep", "category": "tripwires", "slug": "invoking-claude-p-from-within-a-repo-directory-causes-claude"}},
  {{"action": "delete", "category": "patterns", "slug": "compute-human-readable-issue-age-today-1-day-ago-n-days-ago-", "reason": "Self-evident from src/main.rs; no cross-cutting guidance"}},
  {{"action": "merge_into", "category": "patterns", "slug": "use-path-imports-in-claude-md-to-reference-engram-memory-md-",
    "target_category": "architecture", "target_slug": "claude-md-should-hold-only-structural-pointers-path-refs-all",
    "target_updated_body": "Single paragraph synthesising both insights.",
    "reason": "Same core insight about CLAUDE.md referencing vs inlining"}}
]"#
    );

    let output = Command::new("claude")
        .args(["-p", &prompt, "--output-format", "text"])
        .current_dir(std::env::temp_dir())
        .output()
        .context("running claude CLI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude -p failed: {}", stderr.trim());
    }

    let text = String::from_utf8(output.stdout)?;
    let stripped = strip_code_fence(text.trim());
    let json_start = stripped.find('[').context("claude output contained no JSON array")?;
    let json_end = stripped.rfind(']').context("claude output had no closing ]")?;
    let json = &stripped[json_start..=json_end];

    serde_json::from_str(json).context("parsing compact actions from claude output")
}
