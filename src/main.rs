mod capture;
mod claude;
mod cli;
mod compact;
mod config;
mod dispatch;
mod github;
mod index;
mod kata;
mod learn;
mod memory;
mod objective;
mod plan;

use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands, OutputMode};
use include_dir::{include_dir, Dir};
use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

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

static SKILLS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/.claude/skills");
static ISSUE_TEMPLATES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/.github/ISSUE_TEMPLATE");

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mode = cli.output_mode();
    match cli.command {
        Commands::Init => cmd_init(),
        Commands::Plan { subcommand } => cmd_plan(subcommand),
        Commands::Search { query } => cmd_search(mode, &query),
        Commands::Doctor => cmd_doctor(),
        Commands::Compact => cmd_compact(),
        Commands::Objective { subcommand } => cmd_objective(subcommand),
        Commands::Read { permalink } => cmd_read(&permalink, mode),
        Commands::Kata { subcommand } => cmd_kata(subcommand),
        Commands::Memory { subcommand } => cmd_memory(subcommand),
        Commands::Dispatch { issues, opts } => cmd_dispatch(&issues, opts, mode),
        Commands::Capture {
            category,
            title,
            slug,
            read_when,
            tripwires,
            body,
            session,
        } => {
            let slug = slug.unwrap_or_else(|| capture::slugify(&title));
            let body = capture::resolve_body(body)?;
            let mut entry = capture::Capture {
                category,
                slug,
                title,
                read_when,
                tripwires,
                body,
                ..Default::default()
            };
            if session {
                entry.mark_session();
            }
            cmd_capture(mode, entry)
        }
    }
}

fn cmd_capture(mode: OutputMode, entry: capture::Capture) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    capture::write(&repo_root, &entry)?;
    let rel = format!("{}/{}.md", entry.category, entry.slug);
    let origin = entry.origin().map(str::to_string);
    match mode {
        OutputMode::Json => {
            let out = CaptureResultJson {
                path: format!(".engram/memory/{rel}"),
                category: entry.category,
                slug: entry.slug,
                origin,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        OutputMode::Agent => match origin {
            Some(o) => println!("OK capture path=.engram/memory/{rel} origin={o}"),
            None => println!("OK capture path=.engram/memory/{rel}"),
        },
        OutputMode::Human => println!("Captured .engram/memory/{rel} — {}", entry.title),
    }
    Ok(())
}

fn cmd_init() -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let mut cfg = config::Config::load(&repo_root)?;

    if cfg.github.repo.is_none() {
        cfg.github.repo = config::infer_repo(&repo_root)?;
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
    install_dispatch_permission(&repo_root);

    // GitHub setup is optional: local-only repos (no remote, or no `gh`) skip it
    // and still get a fully working memory directory.
    match cfg.repo() {
        Some(repo) if config::gh_available() => {
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
        Some(_) => println!("Skipped GitHub labels: `gh` CLI not found."),
        None => println!(
            "No GitHub repo detected — local-only mode. Memory capture works; \
             `plan`/`learn`/`objective` require GitHub."
        ),
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
            jira,
            kind,
            no_kata,
        } => plan::new(
            &repo_root,
            &title,
            body.as_deref(),
            conversation.as_deref(),
            &kata::TrackerMeta { jira, kind },
            no_kata,
        ),
        cli::PlanCommands::List => plan::list(&repo_root),
        cli::PlanCommands::Learn {
            issue,
            all,
            no_kata,
        } => {
            if all {
                plan::learn_all(&repo_root, no_kata)
            } else if let Some(n) = issue {
                plan::learn_single(&repo_root, n, no_kata)
            } else {
                anyhow::bail!("specify an issue number or pass --all")
            }
        }
        cli::PlanCommands::Land { issue, no_kata } => plan::land(&repo_root, issue, no_kata),
        cli::PlanCommands::Status => plan::status(&repo_root),
    }
}

fn cmd_compact() -> Result<()> {
    let repo_root = config::find_repo_root()?;
    compact::run(&repo_root)
}

fn cmd_memory(subcmd: cli::MemoryCommands) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    match subcmd {
        cli::MemoryCommands::Migrate => cmd_memory_migrate(&repo_root),
    }
}

fn cmd_memory_migrate(repo_root: &std::path::Path) -> Result<()> {
    let cfg = config::Config::load(repo_root)?;
    let repo = config::resolve_repo(&cfg, repo_root)
        .context("resolving GitHub repo for memory migrate")?;

    let (processed, skipped) = memory::migrate_all(repo_root, &repo)?;
    println!("Migrated {processed} file(s), skipped {skipped}.");

    memory::rebuild_index(repo_root)?;
    memory::write_claude_md_section(repo_root)?;
    println!("Rebuilt index and CLAUDE.md section.");

    Ok(())
}

fn cmd_search(mode: cli::OutputMode, query: &str) -> Result<()> {
    use cli::OutputMode;
    let repo_root = config::find_repo_root()?;
    let results = index::search(&repo_root, query)?;
    match mode {
        OutputMode::Json => {
            let out: Vec<SearchResultJson> = results
                .iter()
                .map(|r| SearchResultJson {
                    path: r.path.clone(),
                    category: r.category.clone(),
                    snippet: r.snippet.clone(),
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
                println!("No results for {:?}", query);
            } else {
                for r in &results {
                    println!("{} ({})\n  {}\n", r.path, r.category, r.snippet);
                }
            }
        }
    }
    Ok(())
}

fn scan_memory_matches(
    memory_dir: &std::path::Path,
    categories: &[String],
    needle: &str,
) -> Vec<std::path::PathBuf> {
    let mut matches = Vec::new();
    for cat in categories {
        let cat_dir = memory_dir.join(cat);
        if !cat_dir.exists() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&cat_dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if stem == needle {
                    matches.push(path);
                }
            }
        }
    }
    matches
}

fn cmd_read(permalink: &str, mode: OutputMode) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let memory_dir = repo_root.join(".engram/memory");

    // 1. Try exact path under memory_dir
    let exact = memory_dir.join(permalink);
    if exact.exists() {
        let content = std::fs::read_to_string(&exact)?;
        if mode == OutputMode::Json {
            let out = ReadResultJson {
                path: permalink.to_string(),
                content,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            print!("{content}");
        }
        return Ok(());
    }

    // 2. Scan all categories for stem match
    let needle = std::path::Path::new(permalink)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| permalink.to_string());

    let cfg = config::Config::load(&repo_root)?;
    let matches = scan_memory_matches(&memory_dir, &cfg.memory.default_categories, &needle);

    match matches.len() {
        0 => {
            match mode {
                OutputMode::Json => eprintln!("{{\"error\":\"not_found\",\"id\":\"{permalink}\"}}"),
                OutputMode::Agent => eprintln!("ERR not_found: {permalink}"),
                OutputMode::Human => eprintln!("not found: {permalink}"),
            }
            std::process::exit(1);
        }
        1 => {
            let rel = matches[0].strip_prefix(&memory_dir).unwrap_or(&matches[0]);
            let content = std::fs::read_to_string(&matches[0])?;
            if mode == OutputMode::Json {
                let out = ReadResultJson {
                    path: rel.display().to_string(),
                    content,
                };
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                print!("{content}");
            }
            Ok(())
        }
        _ => {
            let candidates: Vec<String> = matches
                .iter()
                .map(|p| {
                    p.strip_prefix(&memory_dir)
                        .unwrap_or(p)
                        .display()
                        .to_string()
                })
                .collect();
            match mode {
                OutputMode::Json => {
                    eprintln!(
                        "{{\"error\":\"ambiguous\",\"id\":\"{permalink}\",\"candidates\":{}}}",
                        serde_json::to_string(&candidates)?
                    );
                }
                OutputMode::Agent => {
                    eprintln!("ERR ambiguous: {permalink} matches multiple files:");
                    for c in &candidates {
                        eprintln!("  {c}");
                    }
                }
                OutputMode::Human => {
                    eprintln!("ambiguous: {permalink} matches multiple files:");
                    for c in &candidates {
                        eprintln!("  {c}");
                    }
                }
            }
            std::process::exit(1);
        }
    }
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
                config::gh_command().is_ok_and(|mut gh| {
                    gh.args(["auth", "status"])
                        .output()
                        .is_ok_and(|o| o.status.success())
                })
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
        // A bad `engram.githubUser` fails `gh` auth; say how to fix it.
        let why = (!ok && *label == "gh authenticated")
            .then(|| config::gh_command().err())
            .flatten()
            .map(|e| format!(" ({e:#})"))
            .unwrap_or_default();
        println!("{} {label}{why}", if ok { "✓" } else { "✗" });
        if !ok {
            all_ok = false;
        }
    }

    // Only `engram dispatch` needs kwt/tmux/kata, so this line never fails doctor.
    let dispatch_root = config::find_repo_root().unwrap_or_else(|_| PathBuf::from("."));
    match dispatch::unavailable_reasons(&dispatch_root).as_slice() {
        [] => println!("✓ dispatch available"),
        reasons => println!("- dispatch unavailable ({})", reasons.join("; ")),
    }

    // Only needed to run dispatch from auto-mode sessions, so never fails doctor.
    let settings: Vec<_> = DISPATCH_RULE_SETTINGS
        .iter()
        .map(|f| (*f, std::fs::read_to_string(dispatch_root.join(f)).ok()))
        .collect();
    match settings_allowing_dispatch(&settings) {
        Some(file) => println!("✓ {file} allows `engram dispatch`"),
        None => println!(
            "- no `{DISPATCH_PERMISSION_RULE}` rule in {} (run `engram init`; \
             needed to dispatch from auto-mode sessions)",
            DISPATCH_RULE_SETTINGS.join(" or ")
        ),
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
        cli::ObjectiveCommands::New {
            title,
            body,
            jira,
            kind,
            no_kata,
        } => objective::new(
            &repo,
            &title,
            &body,
            &kata::TrackerMeta { jira, kind },
            no_kata,
        ),
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
        } => objective::plan(
            &repo,
            number,
            node.as_deref(),
            all_unblocked,
            body.as_deref(),
            &kata::TrackerMeta { jira, kind },
            no_kata,
        ),
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

fn cmd_kata(subcmd: cli::KataCommands) -> Result<()> {
    let repo_root = config::find_repo_root()?;
    let cfg = config::Config::load(&repo_root)?;
    let repo = config::resolve_repo(&cfg, &repo_root)?;

    match subcmd {
        cli::KataCommands::Sync { issue, all } => {
            if all {
                let results = kata::sync_all(&repo)?;
                let mut failed = 0usize;
                for result in results {
                    match result {
                        Ok(report) => print_sync_report(&report),
                        Err(e) => {
                            eprintln!("warning: {e:#}");
                            failed += 1;
                        }
                    }
                }
                if failed > 0 {
                    anyhow::bail!("{failed} pair(s) failed to sync — see warnings above");
                }
                Ok(())
            } else if let Some(number) = issue {
                let kata_ref = kata::find_kata_ref_for_issue(&repo, number)?;
                let report = kata::sync_pair(&repo, number, &kata_ref)?;
                print_sync_report(&report);
                Ok(())
            } else {
                anyhow::bail!("specify --issue <N> or pass --all")
            }
        }
    }
}

fn print_sync_report(report: &kata::SyncReport) {
    println!("#{} <-> {}", report.github_number, report.kata_ref);
    if report.synced_fields.is_empty()
        && report.conflicts.is_empty()
        && report.blocked_close_note.is_none()
    {
        println!("  no changes");
    }
    for field in &report.synced_fields {
        println!("  synced: {field}");
    }
    for conflict in &report.conflicts {
        println!("  conflict: {conflict}");
    }
    if let Some(note) = &report.blocked_close_note {
        println!("  blocked: {note}");
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

/// Project Claude settings `engram init` adds the dispatch permission rule to.
const CLAUDE_SETTINGS: &str = ".claude/settings.json";
/// A narrow prefix rule, which auto mode honours (broad rules like `Bash(*)` are
/// suspended there). It only matches when the command starts `engram dispatch`.
const DISPATCH_PERMISSION_RULE: &str = "Bash(engram dispatch:*)";
/// Rules that already allow dispatch: init's, and the equivalent space form.
const DISPATCH_PERMISSION_RULES: &[&str] = &[DISPATCH_PERMISSION_RULE, "Bash(engram dispatch *)"];
/// Project settings doctor looks for the rule in (init only writes the first).
const DISPATCH_RULE_SETTINGS: &[&str] = &[CLAUDE_SETTINGS, ".claude/settings.local.json"];

fn allows_dispatch(settings: &serde_json::Value) -> bool {
    settings["permissions"]["allow"]
        .as_array()
        .is_some_and(|a| {
            a.iter().any(|r| {
                r.as_str()
                    .is_some_and(|r| DISPATCH_PERMISSION_RULES.contains(&r))
            })
        })
}

/// The first settings file (name, contents if readable) that allows dispatch.
fn settings_allowing_dispatch<'a>(files: &[(&'a str, Option<String>)]) -> Option<&'a str> {
    files.iter().find_map(|(name, text)| {
        let v: serde_json::Value = serde_json::from_str(text.as_deref()?).ok()?;
        allows_dispatch(&v).then_some(*name)
    })
}

/// `existing` settings with the dispatch rule added to `permissions.allow`,
/// creating what is missing and keeping every other key in order. `Ok(None)`
/// when dispatch is already allowed; an error when the settings can't be
/// edited safely (not JSON, e.g. hand-written comments, or an odd shape).
fn merge_dispatch_rule(existing: Option<&str>) -> Result<Option<String>> {
    use serde_json::{Map, Value};
    let mut settings = match existing.map(str::trim) {
        None | Some("") => Value::Object(Map::new()),
        Some(text) => serde_json::from_str(text).context("not valid JSON")?,
    };
    if allows_dispatch(&settings) {
        return Ok(None);
    }
    settings
        .as_object_mut()
        .context("top level is not an object")?
        .entry("permissions")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("`permissions` is not an object")?
        .entry("allow")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .context("`permissions.allow` is not an array")?
        .push(DISPATCH_PERMISSION_RULE.into());
    Ok(Some(serde_json::to_string_pretty(&settings)? + "\n"))
}

/// Let `engram dispatch` run from auto-mode Claude sessions in this repo.
/// Never fails init: settings it can't read, parse or write are left alone
/// with a warning naming the rule to add by hand.
fn install_dispatch_permission(repo_root: &std::path::Path) {
    let path = repo_root.join(CLAUDE_SETTINGS);
    let add = || -> Result<bool> {
        let existing = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).context("reading it"),
        };
        let Some(updated) = merge_dispatch_rule(existing.as_deref())? else {
            return Ok(false);
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, updated).context("writing it")?;
        Ok(true)
    };
    match add() {
        Ok(false) => println!("{CLAUDE_SETTINGS} already allows `engram dispatch`"),
        Ok(true) => println!("Allowed `{DISPATCH_PERMISSION_RULE}` in {CLAUDE_SETTINGS}"),
        Err(e) => eprintln!(
            "warning: left {CLAUDE_SETTINGS} untouched ({e:#}); add \"{DISPATCH_PERMISSION_RULE}\" \
             to its permissions.allow by hand"
        ),
    }
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
    use rstest::rstest;

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

    fn write_memory_file(root: &std::path::Path, cat: &str, slug: &str, body: &str) {
        let dir = root.join(format!(".engram/memory/{cat}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{slug}.md")), body).unwrap();
    }

    fn cats(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn scan_memory_matches_finds_and_misses() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_memory_file(root, "patterns", "foo", "foo body");
        write_memory_file(root, "tripwires", "foo", "tripwires body");
        let memory_dir = root.join(".engram/memory");

        let found = scan_memory_matches(&memory_dir, &cats(&["patterns", "tripwires"]), "foo");
        assert_eq!(found.len(), 2);

        let missing = scan_memory_matches(&memory_dir, &cats(&["patterns"]), "nope");
        assert!(missing.is_empty());
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
        assert!(root
            .join(".claude/skills/engram-dispatch/SKILL.md")
            .exists());
    }

    #[rstest]
    #[case::no_file(None, r#"{"permissions":{"allow":["Bash(engram dispatch:*)"]}}"#)]
    #[case::no_permissions(
        Some(r#"{"model": "opus"}"#),
        r#"{"model":"opus","permissions":{"allow":["Bash(engram dispatch:*)"]}}"#
    )]
    #[case::other_rules(
        Some(r#"{"hooks": {"Stop": []}, "permissions": {"deny": ["Bash(rm:*)"], "allow": ["Bash(graft:*)"]}, "statusLine": {"type": "command"}}"#),
        r#"{"hooks":{"Stop":[]},"permissions":{"deny":["Bash(rm:*)"],"allow":["Bash(graft:*)","Bash(engram dispatch:*)"]},"statusLine":{"type":"command"}}"#
    )]
    fn merge_dispatch_rule_adds_the_rule_keeping_other_settings(
        #[case] existing: Option<&str>,
        #[case] want: &str,
    ) {
        let merged = merge_dispatch_rule(existing).unwrap().unwrap();
        let merged: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // Compared as text so key order counts.
        assert_eq!(merged.to_string(), want);
    }

    #[rstest]
    fn merge_dispatch_rule_skips_settings_already_allowing_dispatch(
        #[values(DISPATCH_PERMISSION_RULE, "Bash(engram dispatch *)")] rule: &str,
    ) {
        let settings = serde_json::json!({"permissions": {"allow": [rule]}}).to_string();
        assert_eq!(merge_dispatch_rule(Some(&settings)).unwrap(), None);
    }

    #[rstest]
    #[case::jsonc("{ // comment\n}", "not valid JSON")]
    #[case::top_level_array("[]", "top level is not an object")]
    #[case::allow_not_an_array(
        r#"{"permissions": {"allow": "x"}}"#,
        "`permissions.allow` is not an array"
    )]
    fn merge_dispatch_rule_refuses_settings_it_cannot_edit(
        #[case] settings: &str,
        #[case] why: &str,
    ) {
        let err = merge_dispatch_rule(Some(settings)).unwrap_err();
        assert!(format!("{err:#}").contains(why), "{err:#}");
    }

    fn allowing(rule: &str) -> Option<String> {
        Some(format!(r#"{{"permissions": {{"allow": ["{rule}"]}}}}"#))
    }

    #[rstest]
    #[case::shared_file(allowing("Bash(engram dispatch:*)"), None, Some(CLAUDE_SETTINGS))]
    #[case::local_file_space_form(
        allowing("Bash(engram:*)"),
        allowing("Bash(engram dispatch *)"),
        Some(DISPATCH_RULE_SETTINGS[1])
    )]
    #[case::unparsable_and_broad_rule(Some("{ // x\n}".into()), allowing("Bash(engram:*)"), None)]
    #[case::no_files(None, None, None)]
    fn settings_allowing_dispatch_checks_both_files_and_forms(
        #[case] shared: Option<String>,
        #[case] local: Option<String>,
        #[case] want: Option<&str>,
    ) {
        let files = [
            (CLAUDE_SETTINGS, shared),
            (DISPATCH_RULE_SETTINGS[1], local),
        ];
        assert_eq!(settings_allowing_dispatch(&files), want);
    }

    #[test]
    fn install_dispatch_permission_adds_the_rule_once() {
        let dir = tempfile::tempdir().unwrap();
        install_dispatch_permission(dir.path());
        install_dispatch_permission(dir.path());
        let text = std::fs::read_to_string(dir.path().join(CLAUDE_SETTINGS)).unwrap();
        assert_eq!(text.matches(DISPATCH_PERMISSION_RULE).count(), 1);
    }

    #[rstest]
    #[case::unparsable(b"{ // mine\n}")]
    #[case::not_utf8(&[0xff, 0xfe, b'{'])]
    fn install_dispatch_permission_leaves_unusable_settings_untouched(#[case] bytes: &[u8]) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CLAUDE_SETTINGS);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        install_dispatch_permission(dir.path());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
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
