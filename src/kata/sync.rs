//! `engram kata sync`: reconcile a GitHub issue and its kata mirror field by
//! field (title, body, status), plus the parent/blocked-by links derived from
//! the objective graph.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::{
    edit, github_line_prefix, label_add, meta_set, reopen, show, strip_github_prefix,
    strip_ref_line, sync_close, with_ref_line, EditFields, KataIssue, Mirrors, STATUS_CLOSED,
    STATUS_OPEN,
};
use crate::{github, objective};

const SYNC_META_KEY: &str = "engram.sync";
const CONFLICT_LABEL: &str = "kata-sync-conflict";
const CONFLICT_LABEL_COLOR: &str = "d73a4a";
const CONFLICT_LABEL_DESCRIPTION: &str = "engram kata sync detected a conflicting edit";

/// Inline FNV-1a — the codebase has no hash/digest crate and doesn't need
/// cryptographic strength here, just stable change-detection.
fn content_hash(s: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for byte in s.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// What both sides looked like after the last sync, stored in the mirror's
/// `engram.sync` metadata.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct SyncState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gh_title_hash: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gh_body_hash: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gh_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kata_title_hash: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kata_body_hash: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kata_status: Option<String>,
}

fn load_sync_state(metadata: &serde_json::Value) -> SyncState {
    metadata
        .get(SYNC_META_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

/// `true` when there's no recorded baseline (never synced) or the current value
/// differs from it.
fn is_dirty<T: PartialEq>(current: &T, baseline: Option<&T>) -> bool {
    baseline != Some(current)
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum FieldDecision {
    NoOp,
    PushToKata,
    PushToGitHub,
    Conflict,
}

/// Per-field sync decision, driven by which side(s) drifted since the last
/// baseline — not a fixed "source wins". Both dirty but equal collapses to a
/// silent no-op (just re-baselines); both dirty and different is a conflict.
fn decide_field<T: PartialEq>(
    gh_dirty: bool,
    kata_dirty: bool,
    gh_val: &T,
    kata_val: &T,
) -> FieldDecision {
    match (gh_dirty, kata_dirty) {
        (false, false) => FieldDecision::NoOp,
        (true, false) => FieldDecision::PushToKata,
        (false, true) => FieldDecision::PushToGitHub,
        (true, true) if gh_val == kata_val => FieldDecision::NoOp,
        (true, true) => FieldDecision::Conflict,
    }
}

/// The (GitHub, kata) baseline a field leaves for the next sync: the old one
/// when it wasn't resolved (a conflict, or a close held back), else the value
/// both sides now share.
fn next_baseline<T: Clone>(
    decision: FieldDecision,
    resolved: bool,
    current: (&T, &T),
    old: (Option<T>, Option<T>),
) -> (Option<T>, Option<T>) {
    if decision == FieldDecision::Conflict || !resolved {
        return old;
    }
    let (gh, kata) = current;
    let shared = if decision == FieldDecision::PushToGitHub {
        kata
    } else {
        gh
    };
    (Some(shared.clone()), Some(shared.clone()))
}

/// One field's two values, the keys its baseline is kept as (hashes for
/// long text, the value itself for status) and the decision for it.
#[derive(Debug)]
struct Field<T, K> {
    gh: T,
    kata: T,
    keys: (K, K),
    decision: FieldDecision,
}

impl<T: PartialEq, K: PartialEq + Clone> Field<T, K> {
    fn new(gh: T, kata: T, keys: (K, K), baseline: (Option<&K>, Option<&K>)) -> Self {
        let decision = decide_field(
            is_dirty(&keys.0, baseline.0),
            is_dirty(&keys.1, baseline.1),
            &gh,
            &kata,
        );
        Field {
            gh,
            kata,
            keys,
            decision,
        }
    }

    /// See [`next_baseline`].
    fn next_baseline(&self, resolved: bool, old: (Option<K>, Option<K>)) -> (Option<K>, Option<K>) {
        next_baseline(self.decision, resolved, (&self.keys.0, &self.keys.1), old)
    }
}

/// A field whose baseline is a hash of its text.
type TextField = Field<String, u64>;

impl TextField {
    fn text(gh: String, kata: String, baseline: (Option<u64>, Option<u64>)) -> Self {
        let keys = (content_hash(&gh), content_hash(&kata));
        Field::new(gh, kata, keys, (baseline.0.as_ref(), baseline.1.as_ref()))
    }
}

impl SyncState {
    /// The baselines a sync leaves: each resolved field's shared value, the
    /// previous baseline for a conflict or a held-back status.
    fn after(
        self,
        title: &TextField,
        body: &TextField,
        status: &Field<String, String>,
        status_resolved: bool,
    ) -> SyncState {
        let (gh_title_hash, kata_title_hash) =
            title.next_baseline(true, (self.gh_title_hash, self.kata_title_hash));
        let (gh_body_hash, kata_body_hash) =
            body.next_baseline(true, (self.gh_body_hash, self.kata_body_hash));
        let (gh_status, kata_status) =
            status.next_baseline(status_resolved, (self.gh_status, self.kata_status));
        SyncState {
            gh_title_hash,
            gh_body_hash,
            gh_status,
            kata_title_hash,
            kata_body_hash,
            kata_status,
        }
    }
}

#[derive(Debug)]
pub struct SyncReport {
    pub github_number: u64,
    pub kata_ref: String,
    pub synced_fields: Vec<String>,
    pub conflicts: Vec<String>,
    pub blocked_close_note: Option<String>,
}

impl SyncReport {
    fn print(&self) {
        println!("#{} <-> {}", self.github_number, self.kata_ref);
        if self.synced_fields.is_empty()
            && self.conflicts.is_empty()
            && self.blocked_close_note.is_none()
        {
            println!("  no changes");
        }
        for field in &self.synced_fields {
            println!("  synced: {field}");
        }
        for conflict in &self.conflicts {
            println!("  conflict: {conflict}");
        }
        if let Some(note) = &self.blocked_close_note {
            println!("  blocked: {note}");
        }
    }
}

/// The title change each side needs.
#[derive(Debug, Default, PartialEq)]
struct TitleUpdate {
    github: Option<String>,
    kata: Option<String>,
}

fn title_update<K>(title: &Field<String, K>, report: &mut SyncReport) -> TitleUpdate {
    let differ = title.gh != title.kata;
    match title.decision {
        FieldDecision::PushToKata if differ => {
            report.synced_fields.push("title (github -> kata)".into());
            TitleUpdate {
                kata: Some(title.gh.clone()),
                ..TitleUpdate::default()
            }
        }
        FieldDecision::PushToGitHub if differ => {
            report.synced_fields.push("title (kata -> github)".into());
            TitleUpdate {
                github: Some(title.kata.clone()),
                ..TitleUpdate::default()
            }
        }
        FieldDecision::Conflict => {
            report.conflicts.push(format!(
                "title — github: {:?}, kata: {:?}",
                title.gh, title.kata
            ));
            TitleUpdate::default()
        }
        _ => TitleUpdate::default(),
    }
}

/// The full new body each side needs. Bodies are compared without the
/// `Kata:` line (GitHub) and `GitHub:` line (kata); both lines are always
/// written back.
fn body_update<K>(
    body: &Field<String, K>,
    gh_raw: &str,
    kata_raw: &str,
    issue_url: &str,
    kata_ref: &str,
    report: &mut SyncReport,
) -> (Option<String>, Option<String>) {
    let differ = body.gh != body.kata;
    let gh_content = if body.decision == FieldDecision::PushToGitHub {
        &body.kata
    } else {
        &body.gh
    };
    let kata_content = if body.decision == FieldDecision::PushToKata {
        &body.gh
    } else {
        &body.kata
    };

    let needs_ref_line = super::parse_ref(gh_raw) != Some(kata_ref);
    let gh_update = (body.decision == FieldDecision::PushToGitHub || needs_ref_line)
        .then(|| with_ref_line(gh_content.trim_end(), kata_ref))
        .filter(|new| new != gh_raw);
    match body.decision {
        FieldDecision::PushToGitHub if differ => {
            report.synced_fields.push("body (kata -> github)".into());
        }
        _ if needs_ref_line && gh_update.is_some() => {
            report
                .synced_fields
                .push("kata ref line added to github body".into());
        }
        _ => {}
    }

    let kata_update =
        Some(super::mirror_body(issue_url, kata_content)).filter(|new| new != kata_raw);
    match body.decision {
        FieldDecision::PushToKata if differ => {
            report.synced_fields.push("body (github -> kata)".into());
        }
        FieldDecision::Conflict => report
            .conflicts
            .push("body — github and kata diverged, see each issue for its current content".into()),
        _ => {}
    }
    (gh_update, kata_update)
}

/// Carry a status change across. Returns false when a kata close was held
/// back because the GitHub issue is an objective with pending nodes.
fn apply_status(
    repo: &str,
    github_number: u64,
    kata_ref: &str,
    status: &Field<String, String>,
    gh_raw_body: &str,
    report: &mut SyncReport,
) -> Result<bool> {
    let (gh, kata) = (status.gh.as_str(), status.kata.as_str());
    match status.decision {
        FieldDecision::NoOp => {}
        FieldDecision::PushToKata => {
            if gh == STATUS_CLOSED && kata == STATUS_OPEN {
                sync_close(kata_ref, "Synced from GitHub: issue closed.")?;
                report.synced_fields.push("status (github -> kata)".into());
            } else if gh == STATUS_OPEN && kata == STATUS_CLOSED {
                reopen(kata_ref, "Synced from GitHub: issue reopened.")?;
                report.synced_fields.push("status (github -> kata)".into());
            }
        }
        FieldDecision::PushToGitHub => {
            if kata == STATUS_CLOSED && gh == STATUS_OPEN {
                let nodes = objective::parse_nodes_from_comment(gh_raw_body);
                if nodes
                    .as_ref()
                    .is_some_and(|n| !objective::all_nodes_done(n))
                {
                    report.blocked_close_note = Some(format!(
                        "kata {kata_ref} is closed but GitHub #{github_number} is an objective \
                         with pending nodes — not closing"
                    ));
                    return Ok(false);
                }
                if let Some(nodes) = &nodes {
                    let comment = objective::build_close_comment(nodes);
                    github::add_issue_comment(repo, github_number, &comment)?;
                }
                github::close_issue(repo, github_number)?;
                report.synced_fields.push("status (kata -> github)".into());
            } else if kata == STATUS_OPEN && gh == STATUS_CLOSED {
                github::reopen_issue(repo, github_number)?;
                report.synced_fields.push("status (kata -> github)".into());
            }
        }
        FieldDecision::Conflict => report
            .conflicts
            .push(format!("status — github: {gh}, kata: {kata}")),
    }
    Ok(true)
}

/// `--parent`/`--blocked-by` a plan's kata mirror should have, derived from
/// GitHub's objective/node dependency graph.
#[derive(Debug)]
struct DerivedLinks {
    parent: String,
    blocked_by: Vec<String>,
}

/// Derive the links for a plan's mirror. Returns `None` (no-op — never blanks
/// an existing hand-set link) when the plan has no `Objective: #N (node ID)`
/// marker, or the objective, its node or its own kata mirror can't be found.
fn derive_links(repo: &str, mirrors: &Mirrors, plan_gh_body: &str) -> Result<Option<DerivedLinks>> {
    let Some((objective_number, node_id)) = objective::parse_objective_marker(plan_gh_body) else {
        return Ok(None);
    };
    let objective_issue = github::get_issue(repo, objective_number)
        .context("fetching objective issue to derive kata links")?;
    let Some(parent) = mirrors.find(repo, objective_number, objective_issue.body_text())? else {
        return Ok(None);
    };
    let Some(nodes) = objective::parse_nodes_from_comment(objective_issue.body_text()) else {
        return Ok(None);
    };
    let Some(node) = nodes.iter().find(|n| n.id == node_id) else {
        return Ok(None);
    };

    let mut blocked_by = Vec::new();
    for dep_id in objective::blocked_by(node, &nodes) {
        let dep_plan = nodes
            .iter()
            .find(|n| n.id == dep_id)
            .and_then(|n| n.plan_issue);
        if let Some(dep_plan) = dep_plan {
            // One unreadable dependency (e.g. a deleted plan issue) must not
            // block syncing the rest of the pair.
            match mirrors.find_for_issue(repo, dep_plan) {
                Ok(Some(dep_ref)) => blocked_by.push(dep_ref),
                Ok(None) => {}
                Err(e) => eprintln!(
                    "warning: skipping blocked-by link to node {dep_id} (#{dep_plan}): {e:#}"
                ),
            }
        }
    }
    Ok(Some(DerivedLinks { parent, blocked_by }))
}

/// The link edits that bring `current` to `derived`: (new parent, blocked-by
/// refs to add, refs to remove). Nothing to change without derived links.
fn link_changes<'a>(
    derived: Option<&'a DerivedLinks>,
    current: &KataIssue,
) -> (Option<&'a str>, Vec<String>, Vec<String>) {
    let Some(d) = derived else {
        return (None, Vec::new(), Vec::new());
    };
    let parent = Some(d.parent.as_str()).filter(|p| Some(*p) != current.parent.as_deref());
    let desired: HashSet<&str> = d.blocked_by.iter().map(String::as_str).collect();
    let now: HashSet<&str> = current.blocked_by.iter().map(String::as_str).collect();
    let mut add: Vec<String> = desired
        .difference(&now)
        .map(std::string::ToString::to_string)
        .collect();
    let mut remove: Vec<String> = now
        .difference(&desired)
        .map(std::string::ToString::to_string)
        .collect();
    add.sort();
    remove.sort();
    (parent, add, remove)
}

/// Comment on and label both sides of a conflicting pair.
fn flag_conflicts(
    repo: &str,
    github_number: u64,
    kata_ref: &str,
    conflicts: &[String],
) -> Result<()> {
    let message = format!(
        "engram kata sync found a conflict — both sides changed since the last sync:\n{}",
        conflicts
            .iter()
            .map(|c| format!("- {c}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    github::ensure_label(
        repo,
        CONFLICT_LABEL,
        CONFLICT_LABEL_COLOR,
        CONFLICT_LABEL_DESCRIPTION,
    )?;
    github::add_label_to_issue(repo, github_number, CONFLICT_LABEL)?;
    github::add_issue_comment(repo, github_number, &message)?;
    label_add(kata_ref, CONFLICT_LABEL)?;
    super::comment(kata_ref, &message)
}

pub fn sync_pair(
    repo: &str,
    mirrors: &Mirrors,
    github_number: u64,
    kata_ref: &str,
) -> Result<SyncReport> {
    let gh_issue = github::get_issue(repo, github_number).context("fetching GitHub issue")?;
    let kata_issue = show(kata_ref).context("fetching kata issue")?;
    let gh_raw_body = gh_issue.body_text();
    let state = load_sync_state(&kata_issue.metadata);

    let title = TextField::text(
        gh_issue.title.clone(),
        kata_issue.title.clone(),
        (state.gh_title_hash, state.kata_title_hash),
    );
    let body = TextField::text(
        strip_ref_line(gh_raw_body),
        strip_github_prefix(&kata_issue.body, repo),
        (state.gh_body_hash, state.kata_body_hash),
    );
    let (gh_status, kata_status) = (gh_issue.state.to_lowercase(), kata_issue.status.clone());
    let status = Field::new(
        gh_status.clone(),
        kata_status.clone(),
        (gh_status, kata_status),
        (state.gh_status.as_ref(), state.kata_status.as_ref()),
    );

    let mut report = SyncReport {
        github_number,
        kata_ref: kata_ref.to_string(),
        synced_fields: Vec::new(),
        conflicts: Vec::new(),
        blocked_close_note: None,
    };
    let titles = title_update(&title, &mut report);
    let issue_url = github::issue_url(repo, github_number);
    let (gh_body_update, kata_body_update) = body_update(
        &body,
        gh_raw_body,
        &kata_issue.body,
        &issue_url,
        kata_ref,
        &mut report,
    );
    // Derived before anything is written, so a failing lookup can't leave the
    // pair half-synced.
    let derived = derive_links(repo, mirrors, gh_raw_body)?;
    let status_resolved = apply_status(
        repo,
        github_number,
        kata_ref,
        &status,
        gh_raw_body,
        &mut report,
    )?;
    let (parent, blocked_by_add, blocked_by_remove) = link_changes(derived.as_ref(), &kata_issue);

    github::update_issue(
        repo,
        github_number,
        titles.github.as_deref(),
        gh_body_update.as_deref(),
    )
    .context("updating GitHub issue")?;
    edit(
        kata_ref,
        &EditFields {
            title: titles.kata.as_deref(),
            body: kata_body_update.as_deref(),
            parent,
            blocked_by_add: &blocked_by_add,
            blocked_by_remove: &blocked_by_remove,
        },
    )
    .context("updating kata issue")?;

    if !report.conflicts.is_empty() {
        flag_conflicts(repo, github_number, kata_ref, &report.conflicts)?;
    }

    let new_state = state.after(&title, &body, &status, status_resolved);
    meta_set(
        kata_ref,
        SYNC_META_KEY,
        &serde_json::to_value(&new_state).context("serializing sync state")?,
    )
    .context("writing kata sync state")?;

    Ok(report)
}

/// Discover every kata-mirrored GitHub issue in `repo` and sync each pair.
/// A single bad pair (e.g. a kata ref whose GitHub issue was deleted) is
/// reported inline via its `Result::Err` rather than aborting the batch.
fn sync_all(repo: &str, mirrors: &Mirrors) -> Result<Vec<Result<SyncReport>>> {
    Ok(mirrors
        .pairs(repo)?
        .into_iter()
        .map(|(kata_ref, github_number)| {
            sync_pair(repo, mirrors, github_number, &kata_ref)
                .with_context(|| format!("syncing kata {kata_ref} <-> github #{github_number}"))
        })
        .collect())
}

/// What `engram kata sync` syncs.
#[derive(Debug, Clone, Copy)]
pub enum Target {
    Issue(u64),
    All,
}

/// `engram kata sync`: sync `target` and print a report per pair. With
/// `All`, a failing pair is a warning and the command fails at the end.
pub fn run(repo: &str, target: Target) -> Result<()> {
    let mirrors = Mirrors::default();
    match target {
        Target::Issue(number) => {
            let kata_ref = mirrors.find_for_issue(repo, number)?.with_context(|| {
                format!(
                    "no kata mirror found for GitHub issue #{number} \
                         (no `Kata:` line, and no kata issue starts with `{}{number}`)",
                    github_line_prefix(repo)
                )
            })?;
            sync_pair(repo, &mirrors, number, &kata_ref)?.print();
        }
        Target::All => {
            let mut failed = 0usize;
            for result in sync_all(repo, &mirrors)? {
                match result {
                    Ok(report) => report.print(),
                    Err(e) => {
                        eprintln!("warning: {e:#}");
                        failed += 1;
                    }
                }
            }
            if failed > 0 {
                anyhow::bail!("{failed} pair(s) failed to sync — see warnings above");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_stable_for_same_input() {
        assert_eq!(content_hash("hello"), content_hash("hello"));
    }

    #[test]
    fn content_hash_differs_for_different_input() {
        assert_ne!(content_hash("hello"), content_hash("world"));
    }

    #[test]
    fn is_dirty_true_when_no_baseline() {
        assert!(is_dirty(&5u64, None));
    }

    #[test]
    fn is_dirty_false_when_matches_baseline() {
        assert!(!is_dirty(&5u64, Some(&5u64)));
    }

    #[test]
    fn is_dirty_true_when_differs_from_baseline() {
        assert!(is_dirty(&5u64, Some(&6u64)));
    }

    #[test]
    fn decide_field_noop_when_neither_dirty() {
        assert_eq!(decide_field(false, false, &1, &2), FieldDecision::NoOp);
    }

    #[test]
    fn decide_field_pushes_to_kata_when_only_github_dirty() {
        assert_eq!(decide_field(true, false, &1, &2), FieldDecision::PushToKata);
    }

    #[test]
    fn decide_field_pushes_to_github_when_only_kata_dirty() {
        assert_eq!(
            decide_field(false, true, &1, &2),
            FieldDecision::PushToGitHub
        );
    }

    #[test]
    fn decide_field_noop_when_both_dirty_but_equal() {
        assert_eq!(decide_field(true, true, &1, &1), FieldDecision::NoOp);
    }

    #[test]
    fn decide_field_conflict_when_both_dirty_and_different() {
        assert_eq!(decide_field(true, true, &1, &2), FieldDecision::Conflict);
    }

    #[test]
    fn next_baseline_keeps_the_old_one_unless_resolved() {
        let old = (Some(1), Some(2));
        let current = (&10, &20);
        assert_eq!(
            next_baseline(FieldDecision::Conflict, true, current, old),
            old
        );
        assert_eq!(
            next_baseline(FieldDecision::PushToGitHub, false, current, old),
            old
        );
        assert_eq!(
            next_baseline(FieldDecision::PushToGitHub, true, current, old),
            (Some(20), Some(20))
        );
        assert_eq!(
            next_baseline(FieldDecision::PushToKata, true, current, old),
            (Some(10), Some(10))
        );
    }

    fn report() -> SyncReport {
        SyncReport {
            github_number: 7,
            kata_ref: "abc4".into(),
            synced_fields: Vec::new(),
            conflicts: Vec::new(),
            blocked_close_note: None,
        }
    }

    fn field(gh: &str, kata: &str, decision: FieldDecision) -> Field<String, ()> {
        Field {
            gh: gh.into(),
            kata: kata.into(),
            keys: ((), ()),
            decision,
        }
    }

    #[test]
    fn title_update_pushes_the_dirty_side_or_reports_a_conflict() {
        let mut r = report();
        let pushed = title_update(&field("New", "Old", FieldDecision::PushToKata), &mut r);
        assert_eq!(pushed.kata.as_deref(), Some("New"));
        assert_eq!(r.synced_fields, ["title (github -> kata)"]);

        let mut r = report();
        let conflict = title_update(&field("A", "B", FieldDecision::Conflict), &mut r);
        assert_eq!(conflict, TitleUpdate::default());
        assert_eq!(r.conflicts.len(), 1);
    }

    #[test]
    fn body_update_adds_the_ref_line_and_pushes_content() {
        let url = github::issue_url("acme/widgets", 7);
        let kata_raw = super::super::mirror_body(&url, "Old body.");
        let mut r = report();
        let (gh, kata) = body_update(
            &field("New body.", "Old body.", FieldDecision::PushToKata),
            "New body.",
            &kata_raw,
            &url,
            "abc4",
            &mut r,
        );
        assert_eq!(gh.as_deref(), Some("New body.\nKata: abc4"));
        assert_eq!(kata, Some(super::super::mirror_body(&url, "New body.")));
        assert_eq!(
            r.synced_fields,
            [
                "kata ref line added to github body",
                "body (github -> kata)"
            ]
        );
    }

    #[test]
    fn body_update_changes_nothing_in_sync() {
        let url = github::issue_url("acme/widgets", 7);
        let kata_raw = super::super::mirror_body(&url, "Body.");
        let mut r = report();
        let updates = body_update(
            &field("Body.", "Body.", FieldDecision::NoOp),
            "Body.\nKata: abc4",
            &kata_raw,
            &url,
            "abc4",
            &mut r,
        );
        assert_eq!(updates, (None, None));
        assert!(r.synced_fields.is_empty());
    }
}
