use anyhow::Result;
use std::path::Path;
use std::process::Command;

use crate::{claude, memory};

pub fn run(repo_root: &Path) -> Result<()> {
    println!("Reading memory files...");
    let topics = memory::list_all_topics(repo_root)?;
    if topics.is_empty() {
        println!("No memory files found.");
        return Ok(());
    }
    println!("  {} topic files found.", topics.len());

    println!("Auditing with Claude...");
    let actions = claude::compact_learnings(&topics)?;

    let deletes: Vec<_> = actions.iter().filter(|a| a.action == "delete").collect();
    let merges: Vec<_> = actions.iter().filter(|a| a.action == "merge_into").collect();
    let keeps: Vec<_> = actions.iter().filter(|a| a.action == "keep").collect();

    println!(
        "  {} keep, {} delete, {} merge",
        keeps.len(),
        deletes.len(),
        merges.len()
    );

    if !deletes.is_empty() {
        println!("Deleting:");
        for a in &deletes {
            println!(
                "  [-] {}/{} — {}",
                a.category,
                a.slug,
                a.reason.as_deref().unwrap_or("")
            );
        }
    }
    if !merges.is_empty() {
        println!("Merging:");
        for a in &merges {
            println!(
                "  [~] {}/{} → {}/{} — {}",
                a.category,
                a.slug,
                a.target_category.as_deref().unwrap_or("?"),
                a.target_slug.as_deref().unwrap_or("?"),
                a.reason.as_deref().unwrap_or("")
            );
        }
    }

    if deletes.is_empty() && merges.is_empty() {
        println!("Nothing to compact.");
        return Ok(());
    }

    println!("Applying changes...");
    let (deleted, merged) = memory::apply_compact_actions(repo_root, &actions, &topics)?;

    println!("Rebuilding memory index...");
    memory::rebuild_index(repo_root)?;

    println!("Updating CLAUDE.md...");
    memory::write_claude_md_section(repo_root)?;

    let branch = "engram/compact";
    git(repo_root, &["checkout", "-b", branch])?;
    git(repo_root, &["add", ".engram/memory", "CLAUDE.md"])?;

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
        &["commit", "-m", &format!("engram: compact memory ({deleted} deleted, {merged} merged)")],
    )?;
    git(repo_root, &["push", "-u", "origin", branch])?;

    let pr_body = format!(
        "Compacted memory: {deleted} files deleted, {merged} files merged.\n\n---\n*Created by engram compact*"
    );
    let pr_url = crate::github::create_pr(
        &resolve_repo(repo_root)?,
        "engram: compact memory",
        &pr_body,
        "engram-learned",
    )?;

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

fn resolve_repo(repo_root: &Path) -> Result<String> {
    let cfg = crate::config::Config::load(repo_root)?;
    if let Some(repo) = cfg.repo() {
        return Ok(repo.to_string());
    }
    let output = Command::new("gh")
        .args(["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"])
        .current_dir(repo_root)
        .output()?;
    if output.status.success() {
        return Ok(String::from_utf8(output.stdout)?.trim().to_string());
    }
    anyhow::bail!("could not determine GitHub repo")
}
