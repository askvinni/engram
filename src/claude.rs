use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

// Skill reference files are included here so the same quality rules that guide
// interactive Claude Code sessions also govern the claude -p synthesis calls.
const LEARN_QUALITY_GUIDE: &str =
    include_str!("../.claude/skills/engram-learn/references/memory-quality.md");
const COMPACT_QUALITY_GUIDE: &str = include_str!("../.claude/skills/engram-memory/SKILL.md");

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
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
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

## Quality Guide
{LEARN_QUALITY_GUIDE}

---

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

Extract 1–4 learnings. Only include a learning if it passes the extraction bar in the quality guide above.

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

    let json_start = stripped
        .find('[')
        .context("claude output contained no JSON array")?;
    let json_end = stripped
        .rfind(']')
        .context("claude output had no closing ]")?;
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
        r#"You are aggressively pruning an AI agent memory system. The default action is DELETE. Only keep a file if it clears a high bar.

## Memory system reference
{COMPACT_QUALITY_GUIDE}

---

## The only reasons to KEEP a file

1. **It records a failure that already happened** — a bug, a wrong approach, a gotcha that was discovered the hard way. The file exists so the same mistake is not repeated.
2. **It captures non-obvious external behaviour** — something about a third-party tool, API, or environment that a developer could not infer from reading the source code (e.g. a CLI flag that silently does the wrong thing, an API with surprising semantics).
3. **It explains WHY an architectural decision was made** when the alternative was plausible and the reason is not in any source file or commit message — and getting it wrong would cause a real problem.

## DELETE everything else, including

- Any file that describes HOW the code works (a developer can read the code)
- Any file that documents the current design or current command behaviour
- Any pattern that is the obvious/natural approach in Rust or in this codebase
- Any file whose insight is "use X" where X is already used everywhere in the code
- Any file that amounts to a reminder or note-to-self about a decision that is already baked in
- Any file describing a single-command or single-function implementation detail
- Any UX or output-formatting decision already visible in the code
- Anything where a competent developer reading the source for 10 minutes would say "yes, obviously"

## MERGE only when two files that both pass the KEEP bar cover the same root insight

Provide a `target_updated_body` paragraph that synthesises both. Merge targets must not themselves be merge_into sources.

## Files to audit

{files_section}

---

Return ONLY a JSON array. Every file must appear exactly once. Be aggressive — when in doubt, delete.

[
  {{"action": "keep", "category": "tripwires", "slug": "invoking-claude-p-from-within-a-repo-directory-causes-claude"}},
  {{"action": "delete", "category": "patterns", "slug": "compute-human-readable-issue-age-today-1-day-ago-n-days-ago-", "reason": "Describes a done implementation; self-evident from src/main.rs"}},
  {{"action": "merge_into", "category": "patterns", "slug": "source-slug",
    "target_category": "architecture", "target_slug": "target-slug",
    "target_updated_body": "Synthesised paragraph.",
    "reason": "Same root insight"}}
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
    let json_start = stripped
        .find('[')
        .context("claude output contained no JSON array")?;
    let json_end = stripped
        .rfind(']')
        .context("claude output had no closing ]")?;
    let json = &stripped[json_start..=json_end];

    serde_json::from_str(json).context("parsing compact actions from claude output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_code_fence_bare_json() {
        assert_eq!(strip_code_fence("[1, 2]"), "[1, 2]");
    }

    #[test]
    fn strip_code_fence_json_fenced() {
        let input = "```json\n[1, 2]\n```";
        assert_eq!(strip_code_fence(input), "[1, 2]");
    }

    #[test]
    fn strip_code_fence_plain_fenced() {
        let input = "```\n{\"key\": \"val\"}\n```";
        assert_eq!(strip_code_fence(input), "{\"key\": \"val\"}");
    }

    #[test]
    fn strip_code_fence_multiline() {
        let input = "```json\nline1\nline2\n```";
        assert_eq!(strip_code_fence(input), "line1\nline2");
    }

    #[test]
    fn strip_code_fence_no_closing_fence() {
        let input = "```json\n[1, 2]";
        assert_eq!(strip_code_fence(input), "[1, 2]");
    }
}
