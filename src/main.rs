mod cli;
mod claude;
mod config;
mod github;
mod learn;
mod memory;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use std::process::Command;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init => cmd_init(),
        Commands::Plan { title, body } => cmd_plan(title, body.as_deref()),
        Commands::Learn { issue } => cmd_learn(issue),
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

    memory::write_claude_md_section(&repo_root)?;
    println!("Updated CLAUDE.md");

    if let Some(repo) = cfg.repo() {
        github::ensure_label(repo, "engram-plan", "0075ca", "Plan issue created by engram")?;
        github::ensure_label(repo, "engram-learned", "e4e669", "Learning PR created by engram")?;
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

fn infer_repo(repo_root: &std::path::Path) -> Option<String> {
    let output = Command::new("gh")
        .args(["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"])
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
