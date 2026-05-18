use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct Issue {
    pub title: String,
    pub body: Option<String>,
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
}

fn gh(args: &[&str]) -> Result<String> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .context("running gh CLI (is it installed and authenticated?)")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8(output.stdout)?)
}

pub fn ensure_label(repo: &str, name: &str, color: &str, description: &str) -> Result<()> {
    let output = Command::new("gh")
        .args([
            "label", "create", name,
            "--repo", repo,
            "--color", color,
            "--description", description,
        ])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("already exists") {
            anyhow::bail!("failed to create label {}: {}", name, stderr.trim());
        }
    }
    Ok(())
}

pub fn create_issue(repo: &str, title: &str, body: &str, label: &str) -> Result<String> {
    gh(&[
        "issue", "create",
        "--repo", repo,
        "--title", title,
        "--body", body,
        "--label", label,
    ])
}

pub fn get_issue(repo: &str, number: u64) -> Result<Issue> {
    let out = gh(&[
        "issue", "view", &number.to_string(),
        "--repo", repo,
        "--json", "title,body,state",
    ])?;
    serde_json::from_str(&out).context("parsing issue JSON")
}

pub fn find_linked_pr(repo: &str, issue_number: u64) -> Result<Option<PullRequest>> {
    let query = format!("closes #{issue_number} is:merged is:pr");
    let out = gh(&[
        "pr", "list",
        "--repo", repo,
        "--search", &query,
        "--json", "number,title,body",
        "--limit", "5",
    ])?;
    let prs: Vec<PullRequest> = serde_json::from_str(&out).context("parsing PR list JSON")?;
    Ok(prs.into_iter().next())
}

pub fn get_pr_diff(repo: &str, pr_number: u64) -> Result<String> {
    gh(&["pr", "diff", &pr_number.to_string(), "--repo", repo])
}

pub fn create_pr(repo: &str, title: &str, body: &str, label: &str) -> Result<String> {
    gh(&[
        "pr", "create",
        "--repo", repo,
        "--title", title,
        "--body", body,
        "--label", label,
    ])
}
