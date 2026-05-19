use anyhow::Result;
use std::path::Path;
use std::process::Command;

use crate::{config, github, learn, objective};

pub fn new(repo_root: &Path, title: &str, body: Option<&str>) -> Result<()> {
    let cfg = config::Config::load(repo_root)?;
    let repo = config::resolve_repo(&cfg, repo_root)?;
    let body = body.unwrap_or("");
    let missing = missing_plan_sections(body);
    if !missing.is_empty() {
        eprintln!(
            "warning: plan body is missing sections: {}",
            missing.join(", ")
        );
    }
    let url = github::create_issue(&repo, title, body, "engram-plan")?;
    println!("{}", url.trim());
    Ok(())
}

pub fn list(repo_root: &Path) -> Result<()> {
    let cfg = config::Config::load(repo_root)?;
    let repo = config::resolve_repo(&cfg, repo_root)?;
    let plans = github::list_open_plans(&repo)?;
    if plans.is_empty() {
        println!("No open plans.");
        return Ok(());
    }
    for p in &plans {
        let age = crate::days_ago(&p.created_at);
        println!("#{:<4} {} ({})", p.number, p.title, age);
    }
    Ok(())
}

pub fn learn_single(repo_root: &Path, issue: u64) -> Result<()> {
    let cfg = config::Config::load(repo_root)?;
    learn::run(repo_root, &cfg, issue)
}

pub fn learn_all(repo_root: &Path) -> Result<()> {
    let cfg = config::Config::load(repo_root)?;
    let repo = config::resolve_repo(&cfg, repo_root)?;

    let issues = github::list_unlearned_plans(&repo)?;
    if issues.is_empty() {
        println!("No closed plan issues without the engram-learned label.");
        return Ok(());
    }

    println!("Found {} unlearned issue(s).", issues.len());
    let mut learned: Vec<u64> = vec![];
    let mut failed = 0usize;
    for issue in &issues {
        println!("\nLearning from issue #{}: {}", issue.number, issue.title);
        match learn::write_memory(repo_root, issue.number, &repo) {
            Ok(true) => learned.push(issue.number),
            Ok(false) => {}
            Err(e) => {
                eprintln!("  skipping #{}: {e:#}", issue.number);
                failed += 1;
            }
        }
    }

    if learned.is_empty() {
        println!("\nNo learnings extracted from any issue.");
        if failed > 0 {
            anyhow::bail!("{failed} issue(s) failed — see errors above");
        }
        return Ok(());
    }

    let issue_list = learned
        .iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let pr_title = format!("engram: learn from {issue_list}");
    let pr_body = format!("Learnings extracted from: {issue_list}.\n\n---\n*Created by engram*");

    if let Some(pr_url) = learn::commit_memory_pr(
        repo_root,
        &repo,
        "engram/learn-all",
        &learned,
        &pr_title,
        &pr_body,
    )? {
        println!("\nPR created: {pr_url}");
    }

    if failed > 0 {
        anyhow::bail!("{failed} issue(s) failed — see errors above");
    }
    Ok(())
}

pub fn land(repo_root: &Path, issue: u64) -> Result<()> {
    let cfg = config::Config::load(repo_root)?;
    let repo = config::resolve_repo(&cfg, repo_root)?;

    learn::run(repo_root, &cfg, issue)?;

    let gh_issue = github::get_issue(&repo, issue)?;
    if gh_issue.state != "CLOSED" {
        Command::new("gh")
            .args(["issue", "close", &issue.to_string(), "--repo", &repo])
            .status()?;
        println!("Closed issue #{issue}.");
    } else {
        println!("Issue #{issue} already closed.");
    }

    if let Err(e) = objective::maybe_mark_node_done(&repo, gh_issue.body.as_deref().unwrap_or("")) {
        eprintln!("warning: could not update objective node: {e:#}");
    }

    let candidates = [
        format!("fix/issue-{issue}"),
        format!("feat/issue-{issue}"),
        format!("issue-{issue}"),
    ];
    for branch in &candidates {
        let exists = Command::new("git")
            .args(["branch", "--list", branch])
            .current_dir(repo_root)
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);
        if exists {
            Command::new("git")
                .args(["branch", "-d", branch])
                .current_dir(repo_root)
                .status()?;
            println!("Deleted local branch {branch}.");
            break;
        }
    }

    Ok(())
}

pub fn status(repo_root: &Path) -> Result<()> {
    let cfg = config::Config::load(repo_root)?;
    let repo = config::resolve_repo(&cfg, repo_root)?;

    let branch = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo_root)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    if branch.is_empty() {
        println!("Not on a branch.");
        return Ok(());
    }
    println!("Branch: {branch}");

    if let Some(pr) = github::find_pr_for_branch(&repo, &branch)? {
        println!(
            "PR:     #{} {} [{}]",
            pr.number,
            pr.title,
            pr.body
                .as_deref()
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("")
        );
    } else {
        println!("PR:     none");
    }

    let issue_num = branch
        .split(|c: char| !c.is_ascii_digit())
        .find_map(|s| s.parse::<u64>().ok());

    if let Some(n) = issue_num {
        if let Ok(issue) = github::get_issue(&repo, n) {
            if issue.state != "CLOSED" {
                println!("Issue:  #{n} {} [{}]", issue.title, issue.state);
                return Ok(());
            }
        }
    }

    let plans = github::list_open_plans(&repo)?;
    if plans.is_empty() {
        println!("Issue:  no open engram-plan issues");
    } else {
        println!("Open plans:");
        for p in &plans {
            println!("  #{} {}", p.number, p.title);
        }
    }
    Ok(())
}

pub fn missing_plan_sections(body: &str) -> Vec<&'static str> {
    const SECTIONS: &[(&str, &[&str])] = &[
        ("Why", &["**Why"]),
        ("Background", &["**Background"]),
        ("Approach", &["**Approach"]),
        ("Acceptance criteria", &["**Acceptance criteria"]),
        ("Scope", &["**Scope"]),
        ("Edge cases and risks", &["**Edge cases"]),
        ("Key files", &["**Key files"]),
    ];
    SECTIONS
        .iter()
        .filter(|(_, headers)| !headers.iter().any(|h| body.contains(h)))
        .map(|(name, _)| *name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_plan_sections_empty_body() {
        let missing = missing_plan_sections("");
        assert_eq!(
            missing,
            vec![
                "Why",
                "Background",
                "Approach",
                "Acceptance criteria",
                "Scope",
                "Edge cases and risks",
                "Key files"
            ]
        );
    }

    #[test]
    fn missing_plan_sections_complete_body() {
        let body = "**Why** x\n**Background** x\n**Approach** x\n**Acceptance criteria** x\n**Scope** x\n**Edge cases and risks** x\n**Key files** x";
        assert!(missing_plan_sections(body).is_empty());
    }

    #[test]
    fn missing_plan_sections_partial() {
        let body = "**Why** x\n**Scope** x";
        let missing = missing_plan_sections(body);
        assert!(missing.contains(&"Background"));
        assert!(missing.contains(&"Approach"));
        assert!(!missing.contains(&"Why"));
        assert!(!missing.contains(&"Scope"));
    }
}
