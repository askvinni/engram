//! `engram init`: config, memory directory, CLAUDE.md section, skill stubs,
//! issue templates and (when GitHub is reachable) labels.

use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use std::path::Path;

use crate::{claude, config, github, memory};

/// The engram skill stubs `engram init` installs, by skill directory name. Only
/// these are managed: other skills in `.claude/skills` (e.g. graft) are never touched.
static SKILL_STUBS: &[(&str, &str)] = &[
    (
        "engram-plan",
        include_str!("../.claude/skills/engram-plan/SKILL.md"),
    ),
    (
        "engram-objective",
        include_str!("../.claude/skills/engram-objective/SKILL.md"),
    ),
    (
        "engram-learn",
        include_str!("../.claude/skills/engram-learn/SKILL.md"),
    ),
    (
        "engram-memory",
        include_str!("../.claude/skills/engram-memory/SKILL.md"),
    ),
    (
        "engram-dispatch",
        include_str!("../.claude/skills/engram-dispatch/SKILL.md"),
    ),
];
/// Installed by engram before skills became stubs; its content is now `engram guide memory-quality`.
const LEGACY_SKILL_FILES: &[&str] = &["engram-learn/references/memory-quality.md"];
const SKILLS_DIR: &str = ".claude/skills";
const ISSUE_TEMPLATES_DEST: &str = ".github/ISSUE_TEMPLATE";
static ISSUE_TEMPLATES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/.github/ISSUE_TEMPLATE");

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

pub fn run(repo_root: &Path) -> Result<()> {
    let mut cfg = config::Config::load(repo_root)?;
    if cfg.github.repo.is_none() {
        cfg.github.repo = config::infer_repo(repo_root)?;
    }
    cfg.save(repo_root)?;
    println!("Wrote {}", config::CONFIG_PATH);

    let memory_dir = memory::memory_dir(repo_root);
    std::fs::create_dir_all(&memory_dir)
        .with_context(|| format!("creating {}", memory_dir.display()))?;

    let hooks_dir = repo_root.join(claude::PROMPT_HOOKS_DIR);
    if !hooks_dir.exists() {
        std::fs::create_dir_all(&hooks_dir)
            .with_context(|| format!("creating {}", hooks_dir.display()))?;
        std::fs::write(hooks_dir.join("README.md"), PROMPT_HOOKS_README)
            .context("writing the prompt hooks README")?;
        println!("Created {}/", claude::PROMPT_HOOKS_DIR);
    }

    memory::rebuild_index(repo_root)?;
    memory::write_claude_md_section(repo_root)?;
    println!("Updated {}", memory::CLAUDE_MD);

    install_skills(repo_root)?;
    install_issue_templates(repo_root)?;

    // GitHub setup is optional: local-only repos (no remote, or no `gh`) skip it
    // and still get a fully working memory directory.
    match cfg.repo() {
        Some(repo) if config::gh_available() => {
            for (name, color, description) in github::ENGRAM_LABELS {
                github::ensure_label(repo, name, color, description)?;
            }
            let names: Vec<&str> = github::ENGRAM_LABELS.iter().map(|l| l.0).collect();
            println!("GitHub labels ensured: {}", names.join(", "));
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

fn install_issue_templates(repo_root: &Path) -> Result<()> {
    let dest = repo_root.join(ISSUE_TEMPLATES_DEST);
    std::fs::create_dir_all(&dest).with_context(|| format!("creating {ISSUE_TEMPLATES_DEST}"))?;
    ISSUE_TEMPLATES_DIR
        .extract(&dest)
        .context("installing GitHub issue templates")?;
    let names: Vec<String> = ISSUE_TEMPLATES_DIR
        .files()
        .filter_map(|f| f.path().file_stem())
        .map(|s| s.to_string_lossy().to_string())
        .collect();
    println!("Installed GitHub issue templates: {}", names.join(", "));
    Ok(())
}

fn install_skills(repo_root: &Path) -> Result<()> {
    let skills_root = repo_root.join(SKILLS_DIR);
    for (name, stub) in SKILL_STUBS {
        let dest = skills_root.join(name);
        std::fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
        std::fs::write(dest.join("SKILL.md"), stub)
            .with_context(|| format!("installing Claude skill {name}"))?;
    }
    for legacy in LEGACY_SKILL_FILES {
        let path = skills_root.join(legacy);
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            if let Some(dir) = path.parent() {
                // Only succeeds when empty, so files the user added there stay;
                // a non-empty directory is not an error.
                if std::fs::remove_dir(dir).is_err() {
                    eprintln!("note: kept {} (it holds other files)", dir.display());
                }
            }
        }
    }
    let skill_names: Vec<&str> = SKILL_STUBS.iter().map(|(name, _)| *name).collect();
    println!("Installed Claude skills: {}", skill_names.join(", "));
    Ok(())
}

/// Whether every engram skill stub is installed and identical to this binary's.
pub fn skills_current(repo_root: &Path) -> bool {
    SKILL_STUBS.iter().all(|(name, stub)| {
        let dest = repo_root.join(SKILLS_DIR).join(name).join("SKILL.md");
        std::fs::read(dest).is_ok_and(|bytes| bytes == stub.as_bytes())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_issue_template_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        install_issue_templates(root).unwrap();
        assert!(root.join(".github/ISSUE_TEMPLATE/engram-plan.md").exists());
    }

    #[test]
    fn install_skills_writes_only_engram_stubs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        install_skills(root).unwrap();
        for name in ["plan", "objective", "learn", "memory", "dispatch"] {
            let stub = std::fs::read_to_string(
                root.join(format!(".claude/skills/engram-{name}/SKILL.md")),
            )
            .unwrap();
            assert!(
                stub.contains(&format!("engram guide {name}")),
                "engram-{name}"
            );
            assert!(stub.lines().count() <= 15, "engram-{name} is not a stub");
        }
        assert!(!root.join(".claude/skills/graft").exists());
    }

    #[test]
    fn install_skills_replaces_old_full_skills() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let skills = root.join(".claude/skills");
        std::fs::create_dir_all(skills.join("engram-learn/references")).unwrap();
        std::fs::write(skills.join("engram-learn/SKILL.md"), "old full skill").unwrap();
        std::fs::write(
            skills.join("engram-learn/references/memory-quality.md"),
            "old",
        )
        .unwrap();
        install_skills(root).unwrap();
        let stub = std::fs::read_to_string(skills.join("engram-learn/SKILL.md")).unwrap();
        assert!(stub.contains("engram guide learn"));
        assert!(!skills.join("engram-learn/references").exists());
        assert!(skills_current(root));
    }

    #[test]
    fn skills_current_ignores_other_skills() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        install_skills(root).unwrap();
        std::fs::create_dir_all(root.join(".claude/skills/graft")).unwrap();
        std::fs::write(
            root.join(".claude/skills/graft/SKILL.md"),
            b"rewritten by graft",
        )
        .unwrap();
        assert!(skills_current(root));
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
