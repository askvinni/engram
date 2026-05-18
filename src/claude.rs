use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct LearningItem {
    pub category: String,
    pub content: String,
}

pub fn synthesize_learnings(
    issue_title: &str,
    issue_body: &str,
    pr_title: &str,
    pr_body: &str,
    pr_diff: &str,
    current_memory: &str,
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

    let output = Command::new("claude")
        .args(["-p", &prompt, "--output-format", "text"])
        .output()
        .context("running claude CLI (is Claude Code installed and authenticated?)")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude -p failed: {}", stderr.trim());
    }

    let text = String::from_utf8(output.stdout)?;
    let json_start = text.find('[').context("claude output contained no JSON array")?;
    let json_end = text.rfind(']').context("claude output had no closing ]")?;
    let json = &text[json_start..=json_end];

    serde_json::from_str(json).context("parsing learning items from claude output")
}
