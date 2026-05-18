use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct LearningItem {
    pub category: String,
    pub content: String,
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
        r#"You are analyzing a completed GitHub issue and its associated pull request to extract learnings.

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

Extract 1–5 concise learnings from this issue+PR. Use existing categories from current memory when applicable.

Preferred categories: patterns, tripwires, architecture, testing
- patterns: successful approaches worth repeating
- tripwires: things to avoid; past failures or gotchas
- architecture: structural or design decisions
- testing: testing strategies

Return ONLY a JSON array, no other text:
[
  {{"category": "patterns", "content": "one concise actionable sentence"}},
  {{"category": "tripwires", "content": "one concise actionable sentence"}}
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
