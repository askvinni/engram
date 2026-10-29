use anyhow::{Context, Result};
use std::path::Path;

use crate::claude::{CompactAction, CompactKind};
use crate::{claude, config, date, learn, memory};

/// Branch prefix of a compaction PR; the date follows.
const COMPACT_BRANCH_PREFIX: &str = "engram/compact-";

pub fn run(repo_root: &Path) -> Result<()> {
    let repo = config::github_repo(repo_root)?;

    println!("Reading memory files...");
    let topics = memory::list_all_topics(repo_root)?;
    if topics.is_empty() {
        println!("No memory files found.");
        return Ok(());
    }
    println!("  {} topic files found.", topics.len());

    println!("Auditing with Claude...");
    let instructions = memory::read_always_loaded_instructions(repo_root)
        .context("reading always-loaded instructions")?;
    let actions = claude::compact_learnings(&topics, &instructions)?;

    if !print_summary(&actions) {
        println!("Nothing to compact.");
        return Ok(());
    }

    println!("Applying changes...");
    let (deleted, merged) = memory::apply_compact_actions(repo_root, &repo, &actions, &topics)?;

    println!("Rebuilding memory index...");
    memory::rebuild_index(repo_root)?;

    println!("Updating CLAUDE.md...");
    memory::write_claude_md_section(repo_root)?;

    let pr = learn::MemoryPr {
        branch: &format!("{COMPACT_BRANCH_PREFIX}{}", date::today_iso()),
        commit_message: &format!("engram: compact memory ({deleted} deleted, {merged} merged)"),
        title: "engram: compact memory",
        body: &format!(
            "Compacted memory: {deleted} files deleted, {merged} files merged.{}",
            learn::PR_FOOTER
        ),
    };
    if let Some(url) = learn::open_memory_pr(repo_root, &repo, &pr)? {
        println!("PR created: {url}");
    }
    Ok(())
}

/// Print what compaction will do. Returns false when it would change nothing.
fn print_summary(actions: &[CompactAction]) -> bool {
    let of = |kind: CompactKind| -> Vec<&CompactAction> {
        actions.iter().filter(|a| a.action == kind).collect()
    };
    let (keeps, deletes, merges) = (
        of(CompactKind::Keep),
        of(CompactKind::Delete),
        of(CompactKind::MergeInto),
    );
    println!(
        "  {} keep, {} delete, {} merge",
        keeps.len(),
        deletes.len(),
        merges.len()
    );

    let reason = |a: &CompactAction| a.reason.clone().unwrap_or_default();
    if !deletes.is_empty() {
        println!("Deleting:");
        for a in &deletes {
            println!("  [-] {}/{} — {}", a.category, a.slug, reason(a));
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
                reason(a)
            );
        }
    }
    !(deletes.is_empty() && merges.is_empty())
}
