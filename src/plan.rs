use anyhow::{Context, Result};
use std::path::Path;

use crate::{config, date, github, kata, learn, objective, process};

/// Branch the learnings from `plan learn --all` are committed on.
const LEARN_ALL_BRANCH: &str = "engram/learn-all";

pub fn new(
    repo_root: &Path,
    title: &str,
    body: Option<&str>,
    conversation: Option<&str>,
    tracker: &kata::TrackerMeta,
) -> Result<()> {
    let repo = config::github_repo(repo_root)?;
    let body = body.unwrap_or("");
    warn_missing_sections(body);
    let issue = github::create_issue(&repo, title, body, github::PLAN_LABEL)?;

    kata::mirror_new_issue(
        &repo,
        &issue,
        &kata::MirrorSpec {
            title,
            body,
            idempotency_key: kata::plan_idempotency_key(issue.number),
            parent: None,
            meta: tracker.resolve_for(title, body),
        },
    );

    if let Some(conv) = conversation.filter(|c| !c.is_empty()) {
        let comment_body = format!("{}\n{conv}", learn::CONVERSATION_SENTINEL);
        github::add_issue_comment(&repo, issue.number, &comment_body)
            .context("posting conversation comment")?;
    }

    println!("{}", issue.url);
    Ok(())
}

/// Warn, without failing, when a plan body lacks some of the seven sections.
pub fn warn_missing_sections(body: &str) {
    let missing = missing_plan_sections(body);
    if !missing.is_empty() {
        eprintln!(
            "warning: plan body is missing sections: {}",
            missing.join(", ")
        );
    }
}

pub fn list(repo_root: &Path) -> Result<()> {
    let repo = config::github_repo(repo_root)?;
    let plans = github::list_open_issues(&repo, github::PLAN_LABEL)?;
    if plans.is_empty() {
        println!("No open plans.");
        return Ok(());
    }
    for p in &plans {
        println!(
            "#{:<4} {} ({})",
            p.number,
            p.title,
            date::days_ago(&p.created_at)
        );
    }
    Ok(())
}

pub fn learn_single(repo_root: &Path, issue: u64) -> Result<()> {
    learn::run(repo_root, &config::github_repo(repo_root)?, issue)
}

pub fn learn_all(repo_root: &Path) -> Result<()> {
    let repo = config::github_repo(repo_root)?;

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
        // Issues that wrote learnings are labelled by commit_learned once the
        // PR exists (or memory turned out unchanged); the rest are settled here.
        let outcome = match learn::write_memory(repo_root, issue.number, &repo) {
            Ok(true) => {
                learned.push(issue.number);
                continue;
            }
            Ok(false) => learn::LearnOutcome::NoLearnings,
            Err(e) => {
                eprintln!("  skipping #{}: {e:#}", issue.number);
                failed += 1;
                learn::LearnOutcome::Failed
            }
        };
        learn::mark_learned_or_warn(&repo, issue.number, outcome);
    }

    if learned.is_empty() {
        println!("\nNo learnings extracted from any issue.");
    } else {
        let issue_list = learn::issue_list(&learned);
        let title = format!("engram: learn from {issue_list}");
        let pr = learn::MemoryPr {
            branch: LEARN_ALL_BRANCH,
            commit_message: &title,
            title: &title,
            body: &format!(
                "Learnings extracted from: {issue_list}.{}",
                learn::PR_FOOTER
            ),
        };
        if let Some(pr_url) = learn::commit_learned(repo_root, &repo, &learned, &pr)? {
            println!("\nPR created: {pr_url}");
        }
    }

    if failed > 0 {
        anyhow::bail!("{failed} issue(s) failed — see errors above");
    }
    Ok(())
}

/// `engram plan land`: learn from a merged plan, close it and its kata mirror,
/// mark its objective node done and delete its local branch.
pub fn land(repo_root: &Path, issue: u64) -> Result<()> {
    let repo = config::github_repo(repo_root)?;

    learn::run(repo_root, &repo, issue)?;

    let gh_issue = github::get_issue(&repo, issue)?;
    if gh_issue.is_closed() {
        println!("Issue #{issue} already closed.");
    } else {
        match github::close_issue(&repo, issue) {
            Ok(()) => println!("Closed issue #{issue}."),
            // The learnings PR is already open; finish landing, then report.
            Err(e) => eprintln!("warning: could not close issue #{issue}: {e:#}"),
        }
    }

    let pr = match github::find_linked_pr(&repo, issue) {
        Ok(Some(pr)) => Some(pr),
        Ok(None) => {
            eprintln!(
                "warning: no merged PR found for issue #{issue}; skipping kata close and branch cleanup"
            );
            None
        }
        Err(e) => {
            eprintln!(
                "warning: could not look up the PR for issue #{issue}; skipping kata close and \
                 branch cleanup: {e:#}"
            );
            None
        }
    };

    if let (Some(pr), Some(kata_ref)) = (&pr, kata::parse_ref(gh_issue.body_text())) {
        if kata::kata_available() {
            close_kata_mirror(issue, kata_ref, pr);
        }
    }

    if let Err(e) = objective::maybe_mark_node_done(&repo, gh_issue.body_text()) {
        eprintln!("warning: could not update objective node: {e:#}");
    }

    if let Some(pr) = &pr {
        delete_local_branch(repo_root, pr);
    }
    Ok(())
}

/// Close plan `issue`'s kata mirror with the merge commit as evidence (warnings only).
fn close_kata_mirror(issue: u64, kata_ref: &str, pr: &github::PullRequest) {
    let Some(sha) = &pr.merge_commit_sha else {
        eprintln!(
            "warning: no merge commit found for PR #{}; skipping kata close for {kata_ref}",
            pr.number
        );
        return;
    };
    let message = format!(
        "Closed by engram plan land for GitHub issue #{issue}, merged in PR #{}.",
        pr.number
    );
    match kata::close(kata_ref, &message, sha) {
        Ok(()) => println!("Closed kata issue {kata_ref}."),
        Err(e) => eprintln!("warning: could not close kata issue {kata_ref}: {e:#}"),
    }
}

/// Delete the merged PR's local branch, if git agrees it is merged (warnings only).
fn delete_local_branch(repo_root: &Path, pr: &github::PullRequest) {
    let Some(branch) = &pr.head_ref_name else {
        eprintln!(
            "warning: PR #{} has no branch name recorded; skipping branch cleanup",
            pr.number
        );
        return;
    };
    match process::run_checked(process::git(repo_root).args(["branch", "-d", branch])) {
        Ok(_) => println!("Deleted local branch {branch}."),
        Err(e) => eprintln!(
            "warning: could not delete branch {branch} (it may have unmerged commits or not \
             exist locally): {e:#}"
        ),
    }
}

pub fn status(repo_root: &Path) -> Result<()> {
    let repo = config::github_repo(repo_root)?;

    let branch = process::run_checked(process::git(repo_root).args(["branch", "--show-current"]))
        .context("reading the current branch")?;
    if branch.is_empty() {
        println!("Not on a branch.");
        return Ok(());
    }
    println!("Branch: {branch}");

    let pr = github::pr_for_branch(&repo, &branch, github::PrState::Open)?;
    match &pr {
        Some(p) => println!(
            "PR:     #{} {} [{}]",
            p.pr.number,
            p.pr.title,
            p.body.lines().next().unwrap_or("")
        ),
        None => println!("PR:     none"),
    }

    let issue_num = pr
        .as_ref()
        .and_then(|p| github::closed_issues(&p.body, &repo).first().copied())
        .or_else(|| {
            branch
                .split(|c: char| !c.is_ascii_digit())
                .find_map(|s| s.parse::<u64>().ok())
        });

    if let Some(n) = issue_num {
        if let Ok(issue) = github::get_issue(&repo, n) {
            if !issue.is_closed() {
                println!("Issue:  #{n} {} [{}]", issue.title, issue.state);
                return Ok(());
            }
        }
    }

    let plans = github::list_open_issues(&repo, github::PLAN_LABEL)?;
    if plans.is_empty() {
        println!("Issue:  no open {} issues", github::PLAN_LABEL);
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
