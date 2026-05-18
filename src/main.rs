mod claude;
mod cli;
mod compact;
mod config;
mod github;
mod learn;
mod memory;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands};
use include_dir::{include_dir, Dir};
use std::process::Command;

static SKILLS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/.claude/skills");

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init => cmd_init(),
        Commands::Plan { title, body } => cmd_plan(title, body.as_deref()),
        Commands::Learn { issue, all } => {
            if all {
                cmd_learn_all()
            } else if let Some(n) = issue {
                cmd_learn(n)
            } else {
                anyhow::bail!("specify an issue number or pass --all")
            }
        }
        Commands::Doctor => cmd_doctor(),
        Commands::List => cmd_list(),
        Commands::Land { issue } => cmd_land(issue),
        Commands::Status => cmd_status(),
        Commands::Compact => cmd_compact(),
    }
}

fn cmd_init() -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let mut cfg = config::Config::load(&repo_root)?;

    if cfg.github.repo.is_none() {
        cfg.github.repo = infer_repo(&repo_root);
    }

    cfg.save(&repo_root)?;
    println!("Wrote .engram/config.toml");

    std::fs::create_dir_all(repo_root.join(".engram/memory"))?;

    let hooks_dir = repo_root.join(".engram/prompt-hooks");
    if !hooks_dir.exists() {
        std::fs::create_dir_all(&hooks_dir)?;
        std::fs::write(hooks_dir.join("README.md"), PROMPT_HOOKS_README)?;
        println!("Created .engram/prompt-hooks/");
    }

    // Migrate legacy flat memory files if present
    let migrated = memory::migrate_flat_files(&repo_root)?;
    if migrated > 0 {
        println!("Migrated {migrated} legacy memory item(s) to topic-file structure.");
    }
    memory::rebuild_index(&repo_root)?;

    memory::write_claude_md_section(&repo_root)?;
    println!("Updated CLAUDE.md");

    install_skills(&repo_root)?;

    if let Some(repo) = cfg.repo() {
        github::ensure_label(
            repo,
            "engram-plan",
            "0075ca",
            "Plan issue created by engram",
        )?;
        github::ensure_label(
            repo,
            "engram-learned",
            "e4e669",
            "Learning PR created by engram",
        )?;
        println!("GitHub labels ensured: engram-plan, engram-learned");
    }

    println!("engram initialized.");
    Ok(())
}

fn cmd_plan(title: String, body: Option<&str>) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let cfg = config::Config::load(&repo_root)?;
    let repo = cfg
        .repo()
        .map(|s| s.to_string())
        .or_else(|| infer_repo(&repo_root))
        .ok_or_else(|| anyhow::anyhow!("GitHub repo not configured — run `engram init`"))?;

    let url = github::create_issue(&repo, &title, body.unwrap_or(""), "engram-plan")?;
    println!("{}", url.trim());
    Ok(())
}

fn cmd_learn(issue: u64) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let cfg = config::Config::load(&repo_root)?;
    learn::run(&repo_root, &cfg, issue)
}

fn cmd_learn_all() -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let cfg = config::Config::load(&repo_root)?;
    let repo = cfg
        .repo()
        .map(|s| s.to_string())
        .or_else(|| infer_repo(&repo_root))
        .ok_or_else(|| anyhow::anyhow!("GitHub repo not configured — run `engram init`"))?;

    let issues = github::list_unlearned_plans(&repo)?;
    if issues.is_empty() {
        println!("No closed plan issues without the engram-learned label.");
        return Ok(());
    }

    // Capture the current branch so each engram/learn-N branch starts from the same base.
    let base_branch = {
        let out = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&repo_root)
            .output()?;
        String::from_utf8(out.stdout)?.trim().to_string()
    };
    if base_branch.is_empty() {
        anyhow::bail!("not on a branch — check out a branch before running --all");
    }

    println!("Found {} unlearned issue(s).", issues.len());
    for issue in &issues {
        println!("\nLearning from issue #{}: {}", issue.number, issue.title);
        learn::run(&repo_root, &cfg, issue.number)?;
        Command::new("git")
            .args(["checkout", &base_branch])
            .current_dir(&repo_root)
            .status()
            .context("returning to base branch")?;
    }
    Ok(())
}

fn cmd_compact() -> Result<()> {
    let repo_root = config::find_repo_root()?;
    compact::run(&repo_root)
}

const PROMPT_HOOKS_README: &str = r#"# Prompt Hooks

Markdown files in this directory are injected into the Claude prompt during
`engram learn` under a "Project-Specific Rules" section.

Use hooks to customize how learnings are classified for this repo. Examples:

- "Always classify Rust lifetime errors as tripwires."
- "This repo uses pytest — testing learnings should reference pytest patterns."
- "Prefer architecture entries for any change to the public API surface."

Files are loaded in alphabetical order. Only `.md` files are included.
This directory is committed to the repo so rules are shared across the team.
"#;

fn cmd_status() -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let cfg = config::Config::load(&repo_root)?;
    let repo = cfg
        .repo()
        .map(|s| s.to_string())
        .or_else(|| infer_repo(&repo_root))
        .ok_or_else(|| anyhow::anyhow!("GitHub repo not configured — run `engram init`"))?;

    let branch = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&repo_root)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    if branch.is_empty() {
        println!("Not on a branch.");
        return Ok(());
    }
    println!("Branch: {branch}");

    // Look for a PR on this branch
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

    // Find open engram-plan issues referencing this branch by number
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

    // Fall back: show all open plans for context
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

fn cmd_land(issue: u64) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let cfg = config::Config::load(&repo_root)?;
    let repo = cfg
        .repo()
        .map(|s| s.to_string())
        .or_else(|| infer_repo(&repo_root))
        .ok_or_else(|| anyhow::anyhow!("GitHub repo not configured — run `engram init`"))?;

    // Synthesize learnings and open learn PR
    learn::run(&repo_root, &cfg, issue)?;

    // Close the issue if it's still open (GitHub may have auto-closed it via PR)
    let gh_issue = github::get_issue(&repo, issue)?;
    if gh_issue.state != "CLOSED" {
        Command::new("gh")
            .args(["issue", "close", &issue.to_string(), "--repo", &repo])
            .status()?;
        println!("Closed issue #{issue}.");
    } else {
        println!("Issue #{issue} already closed.");
    }

    // Delete local branch matching common naming patterns if it exists
    let candidates = [
        format!("fix/issue-{issue}"),
        format!("feat/issue-{issue}"),
        format!("issue-{issue}"),
    ];
    for branch in &candidates {
        let exists = Command::new("git")
            .args(["branch", "--list", branch])
            .current_dir(&repo_root)
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);
        if exists {
            Command::new("git")
                .args(["branch", "-d", branch])
                .current_dir(&repo_root)
                .status()?;
            println!("Deleted local branch {branch}.");
            break;
        }
    }

    Ok(())
}

fn cmd_list() -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let cfg = config::Config::load(&repo_root)?;
    let repo = cfg
        .repo()
        .map(|s| s.to_string())
        .or_else(|| infer_repo(&repo_root))
        .ok_or_else(|| anyhow::anyhow!("GitHub repo not configured — run `engram init`"))?;

    let plans = github::list_open_plans(&repo)?;
    if plans.is_empty() {
        println!("No open plans.");
        return Ok(());
    }

    for p in &plans {
        let age = days_ago(&p.created_at);
        println!("#{:<4} {} ({})", p.number, p.title, age);
    }
    Ok(())
}

fn days_ago(iso: &str) -> String {
    // Parse YYYY-MM-DDTHH:MM:SSZ and compute days since then
    let date_part = iso.split('T').next().unwrap_or(iso);
    let parts: Vec<u32> = date_part
        .split('-')
        .filter_map(|s| s.parse().ok())
        .collect();
    if parts.len() != 3 {
        return iso.to_string();
    }
    // Use a simple days-since-epoch comparison
    let created_days = days_from_ymd(parts[0] as i32, parts[1], parts[2]);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86400) as i32)
        .unwrap_or(0);
    let diff = now - created_days;
    match diff {
        0 => "today".to_string(),
        1 => "1 day ago".to_string(),
        d => format!("{d} days ago"),
    }
}

fn days_from_ymd(y: i32, m: u32, d: u32) -> i32 {
    // Days since Unix epoch (1970-01-01) for a given date — Gregorian proleptic
    let m = m as i32;
    let d = d as i32;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

type Check = (&'static str, Box<dyn Fn() -> bool>);

fn cmd_doctor() -> Result<()> {
    let mut all_ok = true;

    let checks: &[Check] = &[
        ("git repo", Box::new(|| config::find_repo_root().is_ok())),
        (
            "gh installed",
            Box::new(|| {
                Command::new("gh")
                    .arg("--version")
                    .output()
                    .is_ok_and(|o| o.status.success())
            }),
        ),
        (
            "gh authenticated",
            Box::new(|| {
                Command::new("gh")
                    .args(["auth", "status"])
                    .output()
                    .is_ok_and(|o| o.status.success())
            }),
        ),
        (
            "claude installed",
            Box::new(|| {
                Command::new("claude")
                    .arg("--version")
                    .output()
                    .is_ok_and(|o| o.status.success())
            }),
        ),
        (
            ".engram/config.toml exists",
            Box::new(|| {
                config::find_repo_root()
                    .map(|r| r.join(".engram/config.toml").exists())
                    .unwrap_or(false)
            }),
        ),
        (
            "github repo configured",
            Box::new(|| {
                config::find_repo_root()
                    .and_then(|r| config::Config::load(&r))
                    .map(|c| c.repo().is_some())
                    .unwrap_or(false)
            }),
        ),
        (
            "claude skills current",
            Box::new(|| {
                config::find_repo_root()
                    .map(|r| skills_current(&r))
                    .unwrap_or(false)
            }),
        ),
    ];

    for (label, check) in checks {
        let ok = check();
        println!("{} {}", if ok { "✓" } else { "✗" }, label);
        if !ok {
            all_ok = false;
        }
    }

    if !all_ok {
        anyhow::bail!("one or more checks failed");
    }
    Ok(())
}

fn install_skills(repo_root: &std::path::Path) -> Result<()> {
    SKILLS_DIR
        .extract(repo_root.join(".claude/skills"))
        .context("installing Claude skills")?;
    let skill_names: Vec<&str> = SKILLS_DIR
        .dirs()
        .map(|d| d.path().file_name().and_then(|n| n.to_str()).unwrap_or("?"))
        .collect();
    println!("Installed Claude skills: {}", skill_names.join(", "));
    Ok(())
}

fn skills_current(repo_root: &std::path::Path) -> bool {
    SKILLS_DIR
        .find("**/*")
        .expect("valid glob")
        .filter_map(|e| e.as_file())
        .all(|embedded| {
            let dest = repo_root.join(".claude/skills").join(embedded.path());
            std::fs::read(dest)
                .map(|bytes| bytes == embedded.contents())
                .unwrap_or(false)
        })
}

fn infer_repo(repo_root: &std::path::Path) -> Option<String> {
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_from_ymd_epoch() {
        assert_eq!(days_from_ymd(1970, 1, 1), 0);
    }

    #[test]
    fn days_from_ymd_next_day() {
        assert_eq!(days_from_ymd(1970, 1, 2), 1);
    }

    #[test]
    fn days_from_ymd_y2k() {
        // 30 years × 365 + 7 leap days (1972,76,80,84,88,92,96) = 10957
        assert_eq!(days_from_ymd(2000, 1, 1), 10957);
    }

    #[test]
    fn days_from_ymd_leap_day() {
        // 2000-01-01 = 10957, +31 (Jan) +28 (Feb 1-28) = 11016
        assert_eq!(days_from_ymd(2000, 2, 29), 11016);
    }

    #[test]
    fn days_from_ymd_end_of_year() {
        // 1970-12-31 = day 364
        assert_eq!(days_from_ymd(1970, 12, 31), 364);
    }

    #[test]
    fn days_ago_malformed_returns_input() {
        assert_eq!(days_ago("not-a-date"), "not-a-date");
    }

    #[test]
    fn days_ago_epoch_is_many_days() {
        let result = days_ago("1970-01-01T00:00:00Z");
        assert!(
            result.ends_with("days ago"),
            "expected 'N days ago', got {result}"
        );
    }

    #[test]
    fn install_skills_writes_all_four_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        install_skills(root).unwrap();
        assert!(root.join(".claude/skills/engram-plan/SKILL.md").exists());
        assert!(root.join(".claude/skills/engram-learn/SKILL.md").exists());
        assert!(root
            .join(".claude/skills/engram-learn/references/memory-quality.md")
            .exists());
        assert!(root.join(".claude/skills/engram-memory/SKILL.md").exists());
    }

    #[test]
    fn skills_current_true_after_install() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        install_skills(root).unwrap();
        assert!(skills_current(root));
    }

    #[test]
    fn skills_current_false_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!skills_current(dir.path()));
    }

    #[test]
    fn skills_current_false_when_stale() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        install_skills(root).unwrap();
        // Overwrite one skill with stale content
        std::fs::write(
            root.join(".claude/skills/engram-plan/SKILL.md"),
            b"outdated",
        )
        .unwrap();
        assert!(!skills_current(root));
    }
}
