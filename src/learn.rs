use anyhow::{Context, Result};
use std::path::Path;

use crate::{claude, github, kata, memory, process};

/// Marks the plan comment holding the planning conversation (see `plan new --conversation`).
pub const CONVERSATION_SENTINEL: &str = "<!-- engram:conversation -->";
/// Paths a memory PR commits.
const MEMORY_PATHS: &[&str] = &[memory::MEMORY_DIR, memory::CLAUDE_MD];
/// Branch prefix of the PR learning from one plan issue.
const LEARN_BRANCH_PREFIX: &str = "engram/learn-";
/// Footer of every PR body engram writes for memory changes.
pub const PR_FOOTER: &str = "\n\n---\n*Created by engram*";

/// Validates the issue, synthesizes learnings, and writes them to memory on disk.
/// Returns true if any learnings were written, false if none were found.
/// The synthesis is also posted to the linked kata issue when kata is on.
pub fn write_memory(repo_root: &Path, issue_number: u64, repo: &str) -> Result<bool> {
    println!("Fetching issue #{issue_number}...");
    let issue = github::get_issue(repo, issue_number)?;
    if !issue.is_closed() {
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

    let conversation = planning_conversation(repo, issue_number)?;

    println!("Reading existing memory...");
    let current_memory = memory::read_all(repo_root)?;

    let prompt_hooks = claude::load_prompt_hooks(repo_root);
    let instructions = memory::read_always_loaded_instructions(repo_root)
        .context("reading always-loaded instructions")?;

    println!("Synthesizing learnings with Claude...");
    let items = claude::synthesize_learnings(&claude::SynthesisInput {
        issue_title: &issue.title,
        issue_body: issue.body.as_deref().unwrap_or(""),
        pr_title: &pr.title,
        pr_body: pr.body.as_deref().unwrap_or(""),
        pr_diff: &diff,
        conversation: &conversation,
        current_memory: &current_memory,
        prompt_hooks: &prompt_hooks,
        instructions: &instructions,
    })?;

    if items.is_empty() {
        println!("No learnings extracted.");
        return Ok(false);
    }

    println!("Writing {} learning(s)...", items.len());
    for item in &items {
        memory::write_topic_file(repo_root, repo, item, issue_number)?;
        println!("  [{}] {} — {}", item.category, item.slug, item.title);
    }

    println!("Rebuilding memory index...");
    memory::rebuild_index(repo_root)?;

    println!("Updating CLAUDE.md...");
    memory::write_claude_md_section(repo_root)?;

    post_synthesis(
        repo,
        issue_number,
        kata::parse_ref(issue.body_text()),
        &items,
    )?;
    Ok(true)
}

/// The planning conversation `plan new --conversation` attached to the
/// issue, or "" when there is none.
fn planning_conversation(repo: &str, issue_number: u64) -> Result<String> {
    println!("Fetching issue comments...");
    let comments = github::get_issue_comments(repo, issue_number)?;
    Ok(comments
        .iter()
        .find(|c| c.body.contains(CONVERSATION_SENTINEL))
        .map(|c| {
            c.body
                .replacen(CONVERSATION_SENTINEL, "", 1)
                .trim()
                .to_string()
        })
        .unwrap_or_default())
}

/// Post the synthesized learnings on the issue, and on its kata mirror when
/// kata is on (a kata failure is only a warning).
fn post_synthesis(
    repo: &str,
    issue_number: u64,
    kata_ref: Option<&str>,
    items: &[claude::LearningItem],
) -> Result<()> {
    println!("Posting synthesis comment to issue #{issue_number}...");
    let synthesis_text = format_synthesis_comment(items);
    github::add_issue_comment(repo, issue_number, &synthesis_text)
        .context("posting synthesis comment to GitHub issue")?;

    if let Some(kata_ref) = kata_ref.filter(|_| kata::kata_available()) {
        match kata::comment(kata_ref, &synthesis_text) {
            Ok(()) => println!("Posted synthesis comment to kata {kata_ref}."),
            Err(e) => {
                eprintln!("warning: could not post synthesis comment to kata {kata_ref}: {e:#}");
            }
        }
    }
    Ok(())
}

/// Format synthesized learning items as a comment body shared verbatim
/// between the GitHub issue and its linked kata issue.
fn format_synthesis_comment(items: &[claude::LearningItem]) -> String {
    let mut out = String::from("## Learnings synthesized by engram\n");
    for item in items {
        out.push_str(&format!(
            "\n### [{}] {} (`{}`)\n\n{}\n",
            item.category, item.title, item.slug, item.body
        ));
    }
    out
}

/// How learning from one plan issue ended. Decides whether the issue is
/// labelled `engram-learned`, which keeps `plan learn --all` from picking it up again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearnOutcome {
    /// Synthesis ran and returned no learnings.
    NoLearnings,
    /// Learnings were written but memory on disk did not change (nothing staged).
    Unchanged,
    /// Learnings were committed and a PR was opened.
    PrCreated,
    /// Fetching, synthesis, commit, push or PR creation failed — retry later.
    Failed,
}

impl LearnOutcome {
    /// Once synthesis ran successfully the issue counts as learned; only a
    /// failure leaves it unlabelled so a later `--all` run retries it.
    pub fn should_label(self) -> bool {
        !matches!(self, LearnOutcome::Failed)
    }

    fn reason(self) -> &'static str {
        match self {
            LearnOutcome::NoLearnings => "no learnings",
            LearnOutcome::Unchanged => "memory unchanged",
            LearnOutcome::PrCreated => "learnings PR created",
            LearnOutcome::Failed => "failed",
        }
    }
}

/// Label `issue_number` as learned if `outcome` warrants it. No-op for `Failed`.
pub fn mark_learned(repo: &str, issue_number: u64, outcome: LearnOutcome) -> Result<()> {
    if !outcome.should_label() {
        return Ok(());
    }
    github::add_label_to_issue(repo, issue_number, github::LEARNED_LABEL)
        .with_context(|| format!("labelling #{issue_number} {}", github::LEARNED_LABEL))?;
    println!(
        "Labelled #{issue_number} {} ({}).",
        github::LEARNED_LABEL,
        outcome.reason()
    );
    Ok(())
}

/// Like [`mark_learned`], but a labelling failure is only a warning.
pub fn mark_learned_or_warn(repo: &str, issue_number: u64, outcome: LearnOutcome) {
    if let Err(e) = mark_learned(repo, issue_number, outcome) {
        eprintln!("warning: could not label #{issue_number}: {e:#}");
    }
}

/// `#a, #b, ...` for a list of issue numbers.
pub fn issue_list(numbers: &[u64]) -> String {
    numbers
        .iter()
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `engram plan learn <issue>`: synthesize, open a PR with the learnings and
/// label the issue learned.
pub fn run(repo_root: &Path, repo: &str, issue_number: u64) -> Result<()> {
    if !write_memory(repo_root, issue_number, repo)? {
        return mark_learned(repo, issue_number, LearnOutcome::NoLearnings);
    }
    let pr = MemoryPr {
        branch: &format!("{LEARN_BRANCH_PREFIX}{issue_number}"),
        commit_message: &format!("engram: learn from issue #{issue_number}"),
        title: &format!("engram: learn from #{issue_number}"),
        body: &format!("Learnings extracted from issue #{issue_number}.{PR_FOOTER}"),
    };
    let outcome = match open_memory_pr(repo_root, repo, &pr)? {
        Some(url) => {
            println!("PR created: {url}");
            LearnOutcome::PrCreated
        }
        None => LearnOutcome::Unchanged,
    };
    mark_learned(repo, issue_number, outcome)
}

/// A PR carrying memory changes.
#[derive(Debug)]
pub struct MemoryPr<'a> {
    pub branch: &'a str,
    pub commit_message: &'a str,
    pub title: &'a str,
    pub body: &'a str,
}

/// Commit the memory changes to a new branch, push it and open a PR labelled
/// `engram-learned`. Returns the PR URL, or `None` when memory didn't change.
pub fn open_memory_pr(repo_root: &Path, repo: &str, pr: &MemoryPr) -> Result<Option<String>> {
    let git = |args: &[&str]| process::run_checked(process::git(repo_root).args(args));
    git(&["checkout", "-b", pr.branch])?;
    git(&[&["add"], MEMORY_PATHS].concat())?;
    if !process::has_staged_changes(repo_root)? {
        println!("Nothing to commit — memory unchanged.");
        return Ok(None);
    }
    git(&["commit", "-m", pr.commit_message])?;
    process::run_checked(
        process::git_remote(repo_root)?.args(["push", "-u", "origin", pr.branch]),
    )?;
    let url = github::create_pr(repo, pr.title, pr.body, github::LEARNED_LABEL)
        .context("opening the memory PR")?;
    Ok(Some(url.trim().to_string()))
}

/// [`open_memory_pr`] for learnings from several plan issues, then label each
/// of them learned (failures are warnings). If the commit, push or PR fails,
/// nothing is labelled.
pub fn commit_learned(
    repo_root: &Path,
    repo: &str,
    learned: &[u64],
    pr: &MemoryPr,
) -> Result<Option<String>> {
    let url = open_memory_pr(repo_root, repo, pr)?;
    let outcome = if url.is_some() {
        LearnOutcome::PrCreated
    } else {
        LearnOutcome::Unchanged
    };
    for &n in learned {
        mark_learned_or_warn(repo, n, outcome);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn issue_list_formats_numbers() {
        assert_eq!(issue_list(&[3, 14]), "#3, #14");
        assert_eq!(issue_list(&[]), "");
    }

    #[rstest]
    #[case::no_learnings(LearnOutcome::NoLearnings, true)]
    #[case::memory_unchanged(LearnOutcome::Unchanged, true)]
    #[case::pr_created(LearnOutcome::PrCreated, true)]
    #[case::failed(LearnOutcome::Failed, false)]
    fn should_label_only_after_successful_synthesis(
        #[case] outcome: LearnOutcome,
        #[case] expected: bool,
    ) {
        assert_eq!(outcome.should_label(), expected);
    }

    #[test]
    fn mark_learned_is_noop_on_failure() {
        // Must not shell out to gh: Failed short-circuits before any I/O.
        assert!(mark_learned("owner/repo", 1, LearnOutcome::Failed).is_ok());
    }

    fn make_item(category: &str, slug: &str, title: &str) -> claude::LearningItem {
        claude::LearningItem {
            category: category.to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            read_when: vec!["when testing".to_string()],
            tripwires: vec![],
            body: format!("Body of {slug}."),
        }
    }

    #[test]
    fn format_synthesis_comment_includes_every_item() {
        let items = vec![
            make_item("patterns", "race-condition", "Race condition"),
            make_item("tripwires", "no-mocked-db", "Don't mock the database"),
        ];
        let comment = format_synthesis_comment(&items);
        assert!(comment.contains("[patterns] Race condition (`race-condition`)"));
        assert!(comment.contains("Body of race-condition."));
        assert!(comment.contains("[tripwires] Don't mock the database (`no-mocked-db`)"));
        assert!(comment.contains("Body of no-mocked-db."));
    }

    #[test]
    fn format_synthesis_comment_empty_items() {
        let comment = format_synthesis_comment(&[]);
        assert_eq!(comment, "## Learnings synthesized by engram\n");
    }
}
