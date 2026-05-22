mod claude;
mod cli;
mod compact;
mod config;
mod github;
mod learn;
mod memory;
mod objective;
mod plan;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use include_dir::{include_dir, Dir};
use std::process::Command;

static SKILLS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/.claude/skills");
static ISSUE_TEMPLATES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/.github/ISSUE_TEMPLATE");

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init => cmd_init(),
        Commands::Plan { subcommand } => cmd_plan(subcommand),
        Commands::Doctor => cmd_doctor(),
        Commands::Compact => cmd_compact(),
        Commands::Objective { subcommand } => cmd_objective(subcommand),
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

    memory::rebuild_index(&repo_root)?;

    memory::write_claude_md_section(&repo_root)?;
    println!("Updated CLAUDE.md");

    install_skills(&repo_root)?;
    install_issue_templates(&repo_root)?;

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
        github::ensure_label(
            repo,
            "engram-objective",
            "5319e7",
            "Objective issue created by engram",
        )?;
        println!("GitHub labels ensured: engram-plan, engram-learned, engram-objective");
    }

    println!("engram initialized.");
    Ok(())
}

fn cmd_plan(subcmd: cli::PlanCommands) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    match subcmd {
        cli::PlanCommands::New {
            title,
            body,
            conversation,
        } => plan::new(&repo_root, &title, body.as_deref(), conversation.as_deref()),
        cli::PlanCommands::List => plan::list(&repo_root),
        cli::PlanCommands::Learn { issue, all } => {
            if all {
                plan::learn_all(&repo_root)
            } else if let Some(n) = issue {
                plan::learn_single(&repo_root, n)
            } else {
                anyhow::bail!("specify an issue number or pass --all")
            }
        }
        cli::PlanCommands::Land { issue } => plan::land(&repo_root, issue),
        cli::PlanCommands::Status => plan::status(&repo_root),
    }
}

fn cmd_compact() -> Result<()> {
    let repo_root = config::find_repo_root()?;
    compact::run(&repo_root)
}

const PROMPT_HOOKS_README: &str = r#"# Prompt Hooks

Markdown files in this directory are injected into the Claude prompt during
`engram plan learn` under a "Project-Specific Rules" section.

Use hooks to customize how learnings are classified for this repo. Examples:

- "Always classify Rust lifetime errors as tripwires."
- "This repo uses pytest — testing learnings should reference pytest patterns."
- "Prefer architecture entries for any change to the public API surface."

Files are loaded in alphabetical order. Only `.md` files are included.
This directory is committed to the repo so rules are shared across the team.
"#;

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

fn cmd_objective(subcmd: cli::ObjectiveCommands) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let cfg = config::Config::load(&repo_root)?;
    let repo = config::resolve_repo(&cfg, &repo_root)?;

    match subcmd {
        cli::ObjectiveCommands::New { title, body } => objective::new(&repo, &title, &body),
        cli::ObjectiveCommands::List => objective::list_open(&repo),
        cli::ObjectiveCommands::View { number } => objective::view(&repo, number),
        cli::ObjectiveCommands::Plan {
            number,
            node,
            all_unblocked,
            body,
        } => objective::plan(
            &repo,
            number,
            node.as_deref(),
            all_unblocked,
            body.as_deref(),
        ),
        cli::ObjectiveCommands::Land { number } => objective::land(&repo_root, &repo, number),
    }
}

fn install_issue_templates(repo_root: &std::path::Path) -> Result<()> {
    use anyhow::Context;
    let dest = repo_root.join(".github/ISSUE_TEMPLATE");
    std::fs::create_dir_all(&dest).context("creating .github/ISSUE_TEMPLATE")?;
    ISSUE_TEMPLATES_DIR
        .extract(&dest)
        .context("installing GitHub issue templates")?;
    println!("Installed GitHub issue template: engram-plan");
    Ok(())
}

fn install_skills(repo_root: &std::path::Path) -> Result<()> {
    use anyhow::Context;
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

pub(crate) fn days_ago(iso: &str) -> String {
    let date_part = iso.split('T').next().unwrap_or(iso);
    let parts: Vec<u32> = date_part
        .split('-')
        .filter_map(|s| s.parse().ok())
        .collect();
    if parts.len() != 3 {
        return iso.to_string();
    }
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

pub(crate) fn days_from_ymd(y: i32, m: u32, d: u32) -> i32 {
    let m = m as i32;
    let d = d as i32;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
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
        assert_eq!(days_from_ymd(2000, 1, 1), 10957);
    }

    #[test]
    fn days_from_ymd_leap_day() {
        assert_eq!(days_from_ymd(2000, 2, 29), 11016);
    }

    #[test]
    fn days_from_ymd_end_of_year() {
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
    fn install_issue_template_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        install_issue_templates(root).unwrap();
        assert!(root.join(".github/ISSUE_TEMPLATE/engram-plan.md").exists());
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
        std::fs::write(
            root.join(".claude/skills/engram-plan/SKILL.md"),
            b"outdated",
        )
        .unwrap();
        assert!(!skills_current(root));
    }
}
