mod cli;
mod config;
mod date;
mod dispatch;
mod guide;
mod init;
mod integrations;
mod memory;
mod process;
mod workflow;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands, OutputMode};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

use integrations::kata;
use memory::{capture, compact, index};
use workflow::{objective, plan};

/// JSON schema for `engram search --json`
#[derive(Serialize)]
struct SearchResultJson {
    path: String,
    category: String,
    snippet: String,
}

/// JSON schema for `engram read --json`
#[derive(Serialize)]
struct ReadResultJson {
    path: String,
    content: String,
}

/// JSON schema for `engram capture --json`
#[derive(Serialize)]
struct CaptureResultJson {
    path: String,
    category: String,
    slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mode = cli.output_mode();
    match cli.command {
        Commands::Init => init::run(&config::find_repo_root()?),
        Commands::Plan { subcommand } => cmd_plan(subcommand),
        Commands::Search { query } => cmd_search(mode, &query),
        Commands::Doctor => cmd_doctor(),
        Commands::Guide { topic } => cmd_guide(topic.as_deref()),
        Commands::Compact => compact::run(&config::find_repo_root()?),
        Commands::Objective { subcommand } => cmd_objective(subcommand),
        Commands::Read { permalink } => cmd_read(&permalink, mode),
        Commands::Kata { subcommand } => cmd_kata(&subcommand),
        Commands::Memory { subcommand } => cmd_memory(&subcommand),
        Commands::Dispatch { issues, opts } => cmd_dispatch(&issues, opts, mode),
        Commands::Capture(args) => cmd_capture(mode, capture::Capture::from_args(args)?),
    }
}

/// `--no-kata`: keep kata out of this invocation entirely.
fn apply_no_kata(no_kata: bool) {
    if no_kata {
        kata::disable();
    }
}

fn cmd_capture(mode: OutputMode, entry: capture::Capture) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    capture::write(&repo_root, &entry)?;
    let path = memory::topic_rel_path(&entry.category, &entry.slug);
    let origin = entry.origin().map(str::to_string);
    match mode {
        OutputMode::Json => {
            let out = CaptureResultJson {
                path,
                category: entry.category,
                slug: entry.slug,
                origin,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        OutputMode::Agent => match origin {
            Some(o) => println!("OK capture path={path} origin={o}"),
            None => println!("OK capture path={path}"),
        },
        OutputMode::Human => println!("Captured {path} — {}", entry.title),
    }
    Ok(())
}

fn cmd_plan(subcmd: cli::PlanCommands) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    match subcmd {
        cli::PlanCommands::New {
            title,
            body,
            conversation,
            jira,
            kind,
            no_kata,
        } => {
            apply_no_kata(no_kata);
            plan::new(
                &repo_root,
                &title,
                body.as_deref(),
                conversation.as_deref(),
                &kata::TrackerMeta { jira, kind },
            )
        }
        cli::PlanCommands::List => plan::list(&repo_root),
        cli::PlanCommands::Learn {
            issue,
            all,
            no_kata,
        } => {
            apply_no_kata(no_kata);
            match (issue, all) {
                (_, true) => plan::learn_all(&repo_root),
                (Some(n), false) => plan::learn_single(&repo_root, n),
                (None, false) => anyhow::bail!("specify an issue number or pass --all"),
            }
        }
        cli::PlanCommands::Land { issue, no_kata } => {
            apply_no_kata(no_kata);
            plan::land(&repo_root, issue)
        }
        cli::PlanCommands::Status => plan::status(&repo_root),
    }
}

fn cmd_memory(subcmd: &cli::MemoryCommands) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    match subcmd {
        cli::MemoryCommands::Migrate => {
            let repo = config::github_repo(&repo_root)
                .context("resolving GitHub repo for memory migrate")?;
            let (processed, skipped) = memory::migrate_all(&repo_root, &repo)?;
            println!("Migrated {processed} file(s), skipped {skipped}.");

            memory::rebuild_index(&repo_root)?;
            memory::write_claude_md_section(&repo_root)?;
            println!("Rebuilt index and CLAUDE.md section.");
            Ok(())
        }
    }
}

fn cmd_search(mode: OutputMode, query: &str) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let results = index::search(&repo_root, query)?;
    match mode {
        OutputMode::Json => {
            let out: Vec<SearchResultJson> = results
                .into_iter()
                .map(|r| SearchResultJson {
                    path: r.path,
                    category: r.category,
                    snippet: r.snippet,
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        OutputMode::Agent => {
            println!("OK search count={}", results.len());
            for r in &results {
                println!(
                    "- path={} category={} snippet={}",
                    r.path, r.category, r.snippet
                );
            }
        }
        OutputMode::Human => {
            if results.is_empty() {
                println!("No results for {query:?}");
            } else {
                for r in &results {
                    println!("{} ({})\n  {}\n", r.path, r.category, r.snippet);
                }
            }
        }
    }
    Ok(())
}

/// `engram read`: print the one memory file `permalink` names. Not found and
/// ambiguous are reported in the output mode's own format on stderr, and the
/// process exits 1 without anyhow's extra error line.
fn cmd_read(permalink: &str, mode: OutputMode) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let memory_dir = memory::memory_dir(&repo_root);
    let matches = memory::find_topics(&repo_root, permalink)?;
    let relative = |p: &Path| {
        p.strip_prefix(&memory_dir)
            .unwrap_or(p)
            .display()
            .to_string()
    };

    let candidates: Vec<String> = match matches.as_slice() {
        [path] => {
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            if mode == OutputMode::Json {
                let out = ReadResultJson {
                    path: relative(path),
                    content,
                };
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                print!("{content}");
            }
            return Ok(());
        }
        many => many.iter().map(|p| relative(p)).collect(),
    };

    match (mode, candidates.is_empty()) {
        (OutputMode::Json, true) => {
            eprintln!(
                "{}",
                serde_json::json!({ "error": "not_found", "id": permalink })
            );
        }
        (OutputMode::Json, false) => eprintln!(
            "{}",
            serde_json::json!({ "error": "ambiguous", "id": permalink, "candidates": candidates })
        ),
        (OutputMode::Agent, true) => eprintln!("ERR not_found: {permalink}"),
        (OutputMode::Human, true) => eprintln!("not found: {permalink}"),
        (OutputMode::Agent | OutputMode::Human, false) => {
            let prefix = if mode == OutputMode::Agent {
                "ERR "
            } else {
                ""
            };
            eprintln!("{prefix}ambiguous: {permalink} matches multiple files:");
            for c in &candidates {
                eprintln!("  {c}");
            }
        }
    }
    std::process::exit(1);
}

type Check = (&'static str, Box<dyn Fn() -> bool>);

fn tool_runs(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn cmd_doctor() -> Result<()> {
    let repo_root = config::find_repo_root().ok();
    let in_repo = |f: fn(&Path) -> bool| {
        let root = repo_root.clone();
        Box::new(move || root.as_deref().is_some_and(f)) as Box<dyn Fn() -> bool>
    };

    let checks: Vec<Check> = vec![
        (
            "git repo",
            Box::new({
                let found = repo_root.is_some();
                move || found
            }),
        ),
        ("gh installed", Box::new(config::gh_available)),
        (
            "gh authenticated",
            Box::new(|| {
                config::gh_command().is_ok_and(|mut gh| {
                    gh.args(["auth", "status"])
                        .output()
                        .is_ok_and(|o| o.status.success())
                })
            }),
        ),
        ("claude installed", Box::new(|| tool_runs("claude"))),
        (
            ".engram/config.toml exists",
            in_repo(|r| r.join(config::CONFIG_PATH).exists()),
        ),
        (
            "github repo configured",
            in_repo(|r| config::Config::load(r).is_ok_and(|c| c.repo().is_some())),
        ),
        ("claude skills current", in_repo(init::skills_current)),
    ];

    let mut all_ok = true;
    for (label, check) in &checks {
        let ok = check();
        // A bad `engram.githubUser` fails `gh` auth; say how to fix it.
        let why = (!ok && *label == "gh authenticated")
            .then(|| config::gh_command().err())
            .flatten()
            .map(|e| format!(" ({e:#})"))
            .unwrap_or_default();
        println!("{} {label}{why}", if ok { "✓" } else { "✗" });
        all_ok &= ok;
    }

    // Only `engram dispatch` needs kwt/tmux/kata, so this line never fails doctor.
    let dispatch_root = repo_root.unwrap_or_else(|| PathBuf::from("."));
    match dispatch::unavailable_reasons(&dispatch_root).as_slice() {
        [] => println!("✓ dispatch available"),
        reasons => println!("- dispatch unavailable ({})", reasons.join("; ")),
    }

    if !all_ok {
        anyhow::bail!("one or more checks failed");
    }
    Ok(())
}

fn cmd_objective(subcmd: cli::ObjectiveCommands) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let repo = config::github_repo(&repo_root)?;

    match subcmd {
        cli::ObjectiveCommands::New {
            title,
            body,
            jira,
            kind,
            no_kata,
        } => {
            apply_no_kata(no_kata);
            objective::new(&repo, &title, &body, &kata::TrackerMeta { jira, kind })
        }
        cli::ObjectiveCommands::List => objective::list_open(&repo),
        cli::ObjectiveCommands::View { number } => objective::view(&repo, number),
        cli::ObjectiveCommands::Plan {
            number,
            node,
            all_unblocked,
            body,
            jira,
            kind,
            no_kata,
        } => {
            apply_no_kata(no_kata);
            let target = match (node.as_deref(), all_unblocked) {
                (_, true) => objective::PlanTarget::AllUnblocked,
                (Some(id), false) => objective::PlanTarget::Node(id),
                (None, false) => anyhow::bail!("pass --node <id> or --all-unblocked"),
            };
            objective::plan(
                &repo,
                number,
                target,
                body.as_deref(),
                &kata::TrackerMeta { jira, kind },
            )
        }
        cli::ObjectiveCommands::Land { number } => objective::land(&repo_root, &repo, number),
        cli::ObjectiveCommands::Append {
            number,
            id,
            description,
            depends,
        } => objective::append(&repo, number, &id, &description, depends.as_deref()),
    }
}

fn cmd_dispatch(issues: &[u64], opts: dispatch::DispatchOptions, mode: OutputMode) -> Result<()> {
    if mode != OutputMode::Human {
        eprintln!("warning: --json/--agent are ignored; dispatch output is human-readable only");
    }
    let repo_root = config::find_repo_root()?;
    let toolchain = dispatch::preflight(&repo_root)?;
    let cfg = config::Config::load(&repo_root)?;
    let repo = config::resolve_repo(&cfg, &repo_root)?;
    dispatch::dispatch(&repo_root, &repo, &cfg, &toolchain, issues, opts)
}

fn cmd_kata(subcmd: &cli::KataCommands) -> Result<()> {
    let repo = config::github_repo(&config::find_repo_root()?)?;
    match subcmd {
        &cli::KataCommands::Sync { issue, all } => {
            let target = match (issue, all) {
                (_, true) => kata::sync::Target::All,
                (Some(n), false) => kata::sync::Target::Issue(n),
                (None, false) => anyhow::bail!("specify --issue <N> or pass --all"),
            };
            kata::sync::run(&repo, target)
        }
    }
}

fn cmd_guide(topic: Option<&str>) -> Result<()> {
    match topic {
        Some(topic) => print!("{}", guide::render(topic)?),
        None => print!(
            "engram {} guides (run `engram guide <topic>`):\n{}",
            env!("CARGO_PKG_VERSION"),
            guide::list()
        ),
    }
    Ok(())
}
