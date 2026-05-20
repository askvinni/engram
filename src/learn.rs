use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use crate::{claude, config, github, memory};

/// Validates the issue, synthesizes learnings, and writes them to memory on disk.
/// Returns true if any learnings were written, false if none were found.
pub fn write_memory(repo_root: &Path, issue_number: u64, repo: &str) -> Result<bool> {
    println!("Fetching issue #{issue_number}...");
    let issue = github::get_issue(repo, issue_number)?;
    if issue.state != "CLOSED" {
        anyhow::bail!(
            "issue #{issue_number} is not closed (state: {})",
            issue.state
        );
    }

    println!("Finding linked PR...");
    let pr = github::find_linked_pr(repo, issue_number)?
        .with_context(|| format!("no merged PR found that closes #{issue_number}"))?;

    println!("Fetching diff for PR #{}...", pr.number);
    let diff = github::get_pr_diff(repo, pr.number)?;

    println!("Reading existing memory...");
    let current_memory = memory::read_all(repo_root)?;

    let prompt_hooks = claude::load_prompt_hooks(repo_root);

    println!("Synthesizing learnings with Claude...");
    let items = claude::synthesize_learnings(
        &issue.title,
        issue.body.as_deref().unwrap_or(""),
        &pr.title,
        pr.body.as_deref().unwrap_or(""),
        &diff,
        &current_memory,
        &prompt_hooks,
    )?;

    if items.is_empty() {
        println!("No learnings extracted.");
        return Ok(false);
    }

    println!("Writing {} learning(s)...", items.len());
    for item in &items {
        memory::write_topic_file(repo_root, item, issue_number)?;
        println!("  [{}] {} — {}", item.category, item.slug, item.title);
    }

    println!("Rebuilding memory index...");
    memory::rebuild_index(repo_root)?;

    println!("Updating CLAUDE.md...");
    memory::write_claude_md_section(repo_root)?;

    Ok(true)
}

pub fn run(repo_root: &Path, cfg: &config::Config, issue_number: u64) -> Result<()> {
    let repo = config::resolve_repo(cfg, repo_root)?;

    if !write_memory(repo_root, issue_number, &repo)? {
        return Ok(());
    }

    let branch = format!("engram/learn-{issue_number}");
    git(repo_root, &["checkout", "-b", &branch])?;
    git(repo_root, &["add", ".engram/memory", "CLAUDE.md"])?;

    // Skip commit if nothing was staged
    let staged = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(repo_root)
        .status()?;
    if staged.success() {
        println!("Nothing to commit — memory unchanged.");
        return Ok(());
    }

    git(
        repo_root,
        &[
            "commit",
            "-m",
            &format!("engram: learn from issue #{issue_number}"),
        ],
    )?;
    git(repo_root, &["push", "-u", "origin", &branch])?;

    let pr_body =
        format!("Learnings extracted from issue #{issue_number}.\n\n---\n*Created by engram*");
    let pr_url = github::create_pr(
        &repo,
        &format!("engram: learn from #{issue_number}"),
        &pr_body,
        "engram-learned",
    )?;

    github::add_label_to_issue(&repo, issue_number, "engram-learned")?;

    println!("PR created: {}", pr_url.trim());
    Ok(())
}

fn git(repo_root: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .status()?;
    if !status.success() {
        anyhow::bail!("git {} failed", args.join(" "));
    }
    Ok(())
}

/// Commit all memory changes to a new branch and open a PR labeled `engram-learned`.
/// Returns the PR URL if a commit+PR was created, or None if nothing was staged.
pub fn commit_memory_pr(
    repo_root: &Path,
    repo: &str,
    branch: &str,
    learned: &[u64],
    pr_title: &str,
    pr_body: &str,
) -> Result<Option<String>> {
    git(repo_root, &["checkout", "-b", branch])?;
    git(repo_root, &["add", ".engram/memory", "CLAUDE.md"])?;

    let nothing_staged = Command::new("git")
        .args(["diff", "--cached", "--quiet"])
        .current_dir(repo_root)
        .status()?
        .success();

    if nothing_staged {
        println!("Nothing to commit — memory unchanged.");
        return Ok(None);
    }

    let issue_list = learned
        .iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ");

    git(
        repo_root,
        &["commit", "-m", &format!("engram: learn from {issue_list}")],
    )?;
    git(repo_root, &["push", "-u", "origin", branch])?;

    let pr_url = github::create_pr(repo, pr_title, pr_body, "engram-learned")?;
    let pr_url = pr_url.trim().to_string();

    for &n in learned {
        if let Err(e) = github::add_label_to_issue(repo, n, "engram-learned") {
            eprintln!("warning: could not label #{n}: {e:#}");
        }
    }

    Ok(Some(pr_url))
}
