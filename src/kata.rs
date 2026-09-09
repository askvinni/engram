use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::process::Command;
use std::sync::OnceLock;

use crate::{github, objective};

const KATA_REF_PREFIX: &str = "Kata: ";
const GITHUB_PREFIX: &str = "GitHub: ";
const SYNC_META_KEY: &str = "engram.sync";
const CONFLICT_LABEL: &str = "kata-sync-conflict";

#[allow(dead_code)]
static KATA_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Returns true iff `kata` is on PATH and the current repo has a `.kata.toml` binding.
/// The result is cached for the lifetime of the process.
pub fn kata_available() -> bool {
    *KATA_AVAILABLE.get_or_init(detect)
}

fn detect() -> bool {
    let binary_ok = Command::new("kata")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());

    if !binary_ok {
        return false;
    }

    crate::config::find_repo_root()
        .map(|root| root.join(".kata.toml").exists())
        .unwrap_or(false)
}

/// Create a kata issue and return its short ref (e.g. "abc4").
/// The idempotency key ensures retries after partial failure don't duplicate the issue.
pub fn create(title: &str, body: &str, idempotency_key: &str) -> Result<String> {
    let output = Command::new("kata")
        .args([
            "create",
            title,
            "--body",
            body,
            "--idempotency-key",
            idempotency_key,
            "--json",
        ])
        .output()
        .context("running kata create")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("kata create failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8(output.stdout).context("kata create output not UTF-8")?;
    let v: serde_json::Value =
        serde_json::from_str(&stdout).context("parsing kata create JSON output")?;
    v["issue"]["short_id"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| {
            anyhow::anyhow!("kata create JSON missing issue.short_id: {}", stdout.trim())
        })
}

/// Close a kata issue with commit evidence. If the issue was already deleted or
/// never existed (out-of-band), treats that as success rather than failure —
/// the intent (closed) has effectively been satisfied.
pub fn close(kata_ref: &str, message: &str, commit_sha: &str) -> Result<()> {
    let output = Command::new("kata")
        .args([
            "close",
            kata_ref,
            "--done",
            "--message",
            message,
            "--commit",
            commit_sha,
            "--json",
        ])
        .output()
        .context("running kata close")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let kind = serde_json::from_str::<serde_json::Value>(&stderr)
        .ok()
        .and_then(|v| v["error"]["kind"].as_str().map(|s| s.to_string()));

    if kind.as_deref() == Some("not_found") {
        return Ok(());
    }

    anyhow::bail!("kata close failed: {}", stderr.trim());
}

/// Post a comment to a kata issue.
pub fn comment(kata_ref: &str, body: &str) -> Result<()> {
    let output = Command::new("kata")
        .args(["comment", kata_ref, "--body", body])
        .output()
        .context("running kata comment")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("kata comment failed: {}", stderr.trim());
    }
    Ok(())
}

/// Parse the `Kata: <ref>` line that `plan::new` appends to a GitHub issue body.
pub fn parse_ref(issue_body: &str) -> Option<String> {
    issue_body
        .lines()
        .find_map(|line| line.trim().strip_prefix("Kata:"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Shell wrappers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct KataRefShort {
    short_id: String,
}

#[derive(Debug, Deserialize)]
struct KataLinkEndpoint {
    short_id: String,
}

#[derive(Debug, Deserialize)]
struct KataLink {
    from: KataLinkEndpoint,
    to: KataLinkEndpoint,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct KataIssueCore {
    short_id: String,
    title: String,
    #[serde(default)]
    body: String,
    status: String,
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ShowResponse {
    issue: KataIssueCore,
    #[serde(default)]
    links: Vec<KataLink>,
    #[serde(default)]
    parent: Option<KataRefShort>,
}

/// A kata issue as seen through `kata show`, with the `blocked_by` links already
/// resolved to short_ids (kata's `links` array is a mix of relationship kinds).
#[derive(Debug)]
pub struct KataIssue {
    pub title: String,
    pub body: String,
    pub status: String,
    pub metadata: serde_json::Value,
    pub parent: Option<String>,
    pub blocked_by: Vec<String>,
}

fn kata_command(args: &[&str]) -> Result<String> {
    let output = Command::new("kata")
        .args(args)
        .output()
        .context("running kata CLI (is it installed?)")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("kata {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn show(kata_ref: &str) -> Result<KataIssue> {
    let stdout = kata_command(&["show", kata_ref, "--json"])?;
    let resp: ShowResponse = serde_json::from_str(&stdout).context("parsing kata show JSON")?;
    let blocked_by = resp
        .links
        .iter()
        .filter(|l| l.kind == "blocked_by" && l.from.short_id == resp.issue.short_id)
        .map(|l| l.to.short_id.clone())
        .collect();
    Ok(KataIssue {
        title: resp.issue.title,
        body: resp.issue.body,
        status: resp.issue.status,
        metadata: resp.issue.metadata,
        parent: resp.parent.map(|p| p.short_id),
        blocked_by,
    })
}

#[derive(Debug, Deserialize)]
struct KataListEntry {
    short_id: String,
    #[serde(default)]
    body: String,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    issues: Vec<KataListEntry>,
}

fn list_all() -> Result<Vec<KataListEntry>> {
    let stdout = kata_command(&["list", "--status", "all", "--limit", "0", "--json"])?;
    let resp: ListResponse = serde_json::from_str(&stdout).context("parsing kata list JSON")?;
    Ok(resp.issues)
}

/// Fields to change on a kata issue in a single `kata edit` call. `None`/empty
/// fields are left untouched.
#[derive(Default)]
struct EditFields<'a> {
    title: Option<&'a str>,
    body: Option<&'a str>,
    parent: Option<&'a str>,
    blocked_by_add: &'a [String],
    blocked_by_remove: &'a [String],
}

impl EditFields<'_> {
    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.body.is_none()
            && self.parent.is_none()
            && self.blocked_by_add.is_empty()
            && self.blocked_by_remove.is_empty()
    }
}

fn edit(kata_ref: &str, fields: &EditFields) -> Result<()> {
    if fields.is_empty() {
        return Ok(());
    }
    let mut args: Vec<&str> = vec!["edit", kata_ref];
    if let Some(t) = fields.title {
        args.push("--title");
        args.push(t);
    }
    if let Some(b) = fields.body {
        args.push("--body");
        args.push(b);
    }
    if let Some(p) = fields.parent {
        args.push("--parent");
        args.push(p);
    }
    for r in fields.blocked_by_add {
        args.push("--blocked-by");
        args.push(r);
    }
    for r in fields.blocked_by_remove {
        args.push("--remove-blocked-by");
        args.push(r);
    }
    kata_command(&args)?;
    Ok(())
}

/// Close a kata issue as part of a sync (no commit evidence — use `close` for that).
fn sync_close(kata_ref: &str, message: &str) -> Result<()> {
    kata_command(&["close", kata_ref, "--done", "--message", message])?;
    Ok(())
}

fn reopen(kata_ref: &str, message: &str) -> Result<()> {
    let mut args = vec!["reopen", kata_ref];
    if !message.is_empty() {
        args.push("--comment");
        args.push(message);
    }
    kata_command(&args)?;
    Ok(())
}

fn label_add(kata_ref: &str, label: &str) -> Result<()> {
    kata_command(&["label", "add", kata_ref, label])?;
    Ok(())
}

fn meta_set(kata_ref: &str, key: &str, value: &serde_json::Value) -> Result<()> {
    let value_str = serde_json::to_string(value).context("serializing kata metadata value")?;
    kata_command(&["meta", "set", kata_ref, key, &value_str, "--json-value"])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Pure logic
// ---------------------------------------------------------------------------

/// Inline FNV-1a — the codebase has no hash/digest crate and doesn't need
/// cryptographic strength here, just stable change-detection.
fn content_hash(s: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in s.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Strip the trailing `Kata: <ref>` line node 2.2 appends to a GitHub plan body,
/// so the remaining content is comparable to kata's own body.
fn strip_kata_ref_line(body: &str) -> String {
    body.lines()
        .filter(|line| !line.starts_with(KATA_REF_PREFIX))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

/// Strip the leading `GitHub: <url>\n\n` prefix CLAUDE.md mandates on every kata
/// mirror body, so the remaining content is comparable to GitHub's own body.
fn strip_github_prefix(body: &str, repo: &str) -> String {
    let marker = format!("{GITHUB_PREFIX}https://github.com/{repo}/issues/");
    let Some(rest) = body.strip_prefix(&marker) else {
        return body.to_string();
    };
    let Some(newline) = rest.find('\n') else {
        return String::new();
    };
    rest[newline..].trim_start_matches('\n').to_string()
}

/// Parse a `Kata: <ref>` line node 2.2 appends to a GitHub plan body.
fn find_kata_ref_line(gh_body: &str) -> Option<&str> {
    gh_body
        .lines()
        .find_map(|line| line.strip_prefix(KATA_REF_PREFIX))
        .map(str::trim)
}

/// Parse the GitHub issue number out of a kata mirror's `GitHub: <url>` prefix.
fn find_github_issue_number(kata_body: &str, repo: &str) -> Option<u64> {
    let marker = format!("{GITHUB_PREFIX}https://github.com/{repo}/issues/");
    let rest = kata_body.strip_prefix(&marker)?;
    let first_token = rest.split(['\n', ' ']).next()?;
    first_token.parse().ok()
}

#[derive(Debug, Default, Serialize, Deserialize)]
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
    match baseline {
        None => true,
        Some(b) => b != current,
    }
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
        (true, true) => {
            if gh_val == kata_val {
                FieldDecision::NoOp
            } else {
                FieldDecision::Conflict
            }
        }
    }
}

struct DerivedLinks {
    parent: Option<String>,
    blocked_by: Vec<String>,
}

/// Derive `--parent`/`--blocked-by` for a plan's kata mirror from GitHub's
/// objective/node dependency graph. Returns `None` (no-op — never blanks an
/// existing hand-set link) when the plan has no `Objective: #N (node ID)`
/// marker, or the objective's own kata mirror can't be found.
fn derive_parent_and_blocked_by(repo: &str, plan_gh_body: &str) -> Result<Option<DerivedLinks>> {
    let Some((objective_number, node_id)) = objective::parse_objective_marker(plan_gh_body) else {
        return Ok(None);
    };

    let Ok(objective_kata_ref) = find_kata_ref_for_issue(repo, objective_number) else {
        return Ok(None);
    };

    let objective_issue = github::get_issue(repo, objective_number)
        .context("fetching objective issue to derive kata links")?;
    let Some(nodes) =
        objective::parse_nodes_from_comment(objective_issue.body.as_deref().unwrap_or(""))
    else {
        return Ok(None);
    };
    let Some(node) = nodes.iter().find(|n| n.id == node_id) else {
        return Ok(None);
    };

    let mut blocked_by = Vec::new();
    for dep_id in objective::blocked_by(node, &nodes) {
        let Some(dep_node) = nodes.iter().find(|n| n.id == dep_id) else {
            continue;
        };
        let Some(dep_plan_issue) = dep_node.plan_issue else {
            continue;
        };
        if let Ok(dep_kata_ref) = find_kata_ref_for_issue(repo, dep_plan_issue) {
            blocked_by.push(dep_kata_ref);
        }
    }

    Ok(Some(DerivedLinks {
        parent: Some(objective_kata_ref),
        blocked_by,
    }))
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct SyncReport {
    pub github_number: u64,
    pub kata_ref: String,
    pub synced_fields: Vec<String>,
    pub conflicts: Vec<String>,
    pub blocked_close_note: Option<String>,
}

/// Fast path: parse the `Kata: <ref>` line node 2.2 writes on a GitHub plan.
/// Falls back to scanning every kata issue for the `GitHub: <url>` prefix,
/// since hand-mirrored issues (nodes 2.1-2.6 among them) never got that line.
pub fn find_kata_ref_for_issue(repo: &str, github_number: u64) -> Result<String> {
    let issue = github::get_issue(repo, github_number).context("fetching GitHub issue")?;
    if let Some(kata_ref) = find_kata_ref_line(issue.body.as_deref().unwrap_or("")) {
        return Ok(kata_ref.to_string());
    }
    list_all()?
        .into_iter()
        .find(|k| find_github_issue_number(&k.body, repo) == Some(github_number))
        .map(|k| k.short_id)
        .ok_or_else(|| anyhow::anyhow!("no kata mirror found for GitHub issue #{github_number}"))
}

pub fn sync_pair(repo: &str, github_number: u64, kata_ref: &str) -> Result<SyncReport> {
    let gh_issue = github::get_issue(repo, github_number).context("fetching GitHub issue")?;
    let kata_issue = show(kata_ref).context("fetching kata issue")?;

    let gh_raw_body = gh_issue.body.as_deref().unwrap_or("");
    let canonical_gh_body = strip_kata_ref_line(gh_raw_body);
    let canonical_kata_body = strip_github_prefix(&kata_issue.body, repo);

    let gh_status = gh_issue.state.to_lowercase();
    let kata_status = kata_issue.status.clone();

    let state = load_sync_state(&kata_issue.metadata);

    let gh_title_hash = content_hash(&gh_issue.title);
    let kata_title_hash = content_hash(&kata_issue.title);
    let gh_body_hash = content_hash(&canonical_gh_body);
    let kata_body_hash = content_hash(&canonical_kata_body);

    let title_decision = decide_field(
        is_dirty(&gh_title_hash, state.gh_title_hash.as_ref()),
        is_dirty(&kata_title_hash, state.kata_title_hash.as_ref()),
        &gh_issue.title,
        &kata_issue.title,
    );
    let body_decision = decide_field(
        is_dirty(&gh_body_hash, state.gh_body_hash.as_ref()),
        is_dirty(&kata_body_hash, state.kata_body_hash.as_ref()),
        &canonical_gh_body,
        &canonical_kata_body,
    );
    let status_decision = decide_field(
        is_dirty(&gh_status, state.gh_status.as_ref()),
        is_dirty(&kata_status, state.kata_status.as_ref()),
        &gh_status,
        &kata_status,
    );

    let mut synced_fields = Vec::new();
    let mut conflicts = Vec::new();

    // --- title ---
    let mut gh_title_update: Option<String> = None;
    let mut kata_title_update: Option<String> = None;
    match title_decision {
        FieldDecision::NoOp => {}
        FieldDecision::PushToKata => {
            if gh_issue.title != kata_issue.title {
                kata_title_update = Some(gh_issue.title.clone());
                synced_fields.push("title (github -> kata)".to_string());
            }
        }
        FieldDecision::PushToGitHub => {
            if gh_issue.title != kata_issue.title {
                gh_title_update = Some(kata_issue.title.clone());
                synced_fields.push("title (kata -> github)".to_string());
            }
        }
        FieldDecision::Conflict => conflicts.push(format!(
            "title — github: {:?}, kata: {:?}",
            gh_issue.title, kata_issue.title
        )),
    }

    // --- body ---
    let final_gh_canonical = if body_decision == FieldDecision::PushToGitHub {
        &canonical_kata_body
    } else {
        &canonical_gh_body
    };
    let final_kata_canonical = if body_decision == FieldDecision::PushToKata {
        &canonical_gh_body
    } else {
        &canonical_kata_body
    };

    let needs_kata_ref_line = find_kata_ref_line(gh_raw_body) != Some(kata_ref);
    let mut gh_body_update: Option<String> = None;
    if body_decision == FieldDecision::PushToGitHub || needs_kata_ref_line {
        let new_gh_body = format!(
            "{}\n{KATA_REF_PREFIX}{kata_ref}",
            final_gh_canonical.trim_end()
        );
        if new_gh_body != gh_raw_body {
            gh_body_update = Some(new_gh_body);
        }
    }
    if body_decision == FieldDecision::PushToGitHub && canonical_gh_body != canonical_kata_body {
        synced_fields.push("body (kata -> github)".to_string());
    } else if needs_kata_ref_line && gh_body_update.is_some() {
        synced_fields.push("kata ref line added to github body".to_string());
    }

    let new_kata_body = format!(
        "{GITHUB_PREFIX}https://github.com/{repo}/issues/{github_number}\n\n{final_kata_canonical}"
    );
    let kata_body_update = if new_kata_body != kata_issue.body {
        Some(new_kata_body)
    } else {
        None
    };
    if body_decision == FieldDecision::PushToKata && canonical_gh_body != canonical_kata_body {
        synced_fields.push("body (github -> kata)".to_string());
    }
    if body_decision == FieldDecision::Conflict {
        conflicts.push(
            "body — github and kata diverged, see each issue for its current content".to_string(),
        );
    }

    // --- status ---
    let mut status_blocked = false;
    match status_decision {
        FieldDecision::NoOp => {}
        FieldDecision::PushToKata => {
            if gh_status == "closed" && kata_status == "open" {
                sync_close(kata_ref, "Synced from GitHub: issue closed.")?;
                synced_fields.push("status (github -> kata)".to_string());
            } else if gh_status == "open" && kata_status == "closed" {
                reopen(kata_ref, "Synced from GitHub: issue reopened.")?;
                synced_fields.push("status (github -> kata)".to_string());
            }
        }
        FieldDecision::PushToGitHub => {
            if kata_status == "closed" && gh_status == "open" {
                let nodes = objective::parse_nodes_from_comment(gh_raw_body);
                let gated = nodes.as_ref().map(|n| !objective::all_nodes_done(n));
                if gated == Some(true) {
                    status_blocked = true;
                } else {
                    if let Some(nodes) = &nodes {
                        let comment = objective::build_close_comment(nodes);
                        github::add_issue_comment(repo, github_number, &comment)?;
                    }
                    github::close_issue(repo, github_number)?;
                    synced_fields.push("status (kata -> github)".to_string());
                }
            } else if kata_status == "open" && gh_status == "closed" {
                github::reopen_issue(repo, github_number)?;
                synced_fields.push("status (kata -> github)".to_string());
            }
        }
        FieldDecision::Conflict => {
            conflicts.push(format!("status — github: {gh_status}, kata: {kata_status}"))
        }
    }
    let blocked_close_note = status_blocked.then(|| {
        format!(
            "kata {kata_ref} is closed but GitHub #{github_number} is an objective with pending nodes — not closing"
        )
    });

    // --- derived --parent/--blocked-by ---
    let derived = derive_parent_and_blocked_by(repo, gh_raw_body)?;
    let parent_update = derived
        .as_ref()
        .and_then(|d| d.parent.as_deref())
        .filter(|p| Some(*p) != kata_issue.parent.as_deref());
    let (blocked_by_add, blocked_by_remove): (Vec<String>, Vec<String>) = match &derived {
        Some(d) => {
            let desired: HashSet<&str> = d.blocked_by.iter().map(String::as_str).collect();
            let current: HashSet<&str> = kata_issue.blocked_by.iter().map(String::as_str).collect();
            (
                desired
                    .difference(&current)
                    .map(|s| s.to_string())
                    .collect(),
                current
                    .difference(&desired)
                    .map(|s| s.to_string())
                    .collect(),
            )
        }
        None => (Vec::new(), Vec::new()),
    };

    // --- apply GitHub-side write ---
    if gh_title_update.is_some() || gh_body_update.is_some() {
        github::update_issue(
            repo,
            github_number,
            gh_title_update.as_deref(),
            gh_body_update.as_deref(),
        )
        .context("updating GitHub issue")?;
    }

    // --- apply kata-side write (title/body/derived links in one call) ---
    let kata_fields = EditFields {
        title: kata_title_update.as_deref(),
        body: kata_body_update.as_deref(),
        parent: parent_update,
        blocked_by_add: &blocked_by_add,
        blocked_by_remove: &blocked_by_remove,
    };
    edit(kata_ref, &kata_fields).context("updating kata issue")?;

    // --- conflicts: comment + label both sides ---
    if !conflicts.is_empty() {
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
            "d73a4a",
            "engram kata sync detected a conflicting edit",
        )?;
        github::add_label_to_issue(repo, github_number, CONFLICT_LABEL)?;
        github::add_issue_comment(repo, github_number, &message)?;
        label_add(kata_ref, CONFLICT_LABEL)?;
        comment(kata_ref, &message)?;
    }

    // --- write back sync state ---
    let new_title_hash = if title_decision == FieldDecision::Conflict {
        (state.gh_title_hash, state.kata_title_hash)
    } else {
        let resolved = if title_decision == FieldDecision::PushToGitHub {
            kata_title_hash
        } else {
            gh_title_hash
        };
        (Some(resolved), Some(resolved))
    };
    let new_body_hash = if body_decision == FieldDecision::Conflict {
        (state.gh_body_hash, state.kata_body_hash)
    } else {
        let resolved = if body_decision == FieldDecision::PushToGitHub {
            kata_body_hash
        } else {
            gh_body_hash
        };
        (Some(resolved), Some(resolved))
    };
    let new_status = if status_decision == FieldDecision::Conflict || status_blocked {
        (state.gh_status.clone(), state.kata_status.clone())
    } else {
        let resolved = if status_decision == FieldDecision::PushToGitHub {
            kata_status.clone()
        } else {
            gh_status.clone()
        };
        (Some(resolved.clone()), Some(resolved))
    };

    let new_state = SyncState {
        gh_title_hash: new_title_hash.0,
        kata_title_hash: new_title_hash.1,
        gh_body_hash: new_body_hash.0,
        kata_body_hash: new_body_hash.1,
        gh_status: new_status.0,
        kata_status: new_status.1,
    };
    meta_set(
        kata_ref,
        SYNC_META_KEY,
        &serde_json::to_value(&new_state).context("serializing sync state")?,
    )
    .context("writing kata sync state")?;

    Ok(SyncReport {
        github_number,
        kata_ref: kata_ref.to_string(),
        synced_fields,
        conflicts,
        blocked_close_note,
    })
}

/// Discover every kata-mirrored GitHub issue in `repo` and sync each pair.
/// A single bad pair (e.g. a kata ref whose GitHub issue was deleted) is
/// reported inline via its `Result::Err` rather than aborting the batch.
pub fn sync_all(repo: &str) -> Result<Vec<Result<SyncReport>>> {
    let pairs: Vec<(String, u64)> = list_all()?
        .into_iter()
        .filter_map(|k| {
            let github_number = find_github_issue_number(&k.body, repo)?;
            Some((k.short_id, github_number))
        })
        .collect();

    Ok(pairs
        .into_iter()
        .map(|(kata_ref, github_number)| {
            sync_pair(repo, github_number, &kata_ref)
                .with_context(|| format!("syncing kata {kata_ref} <-> github #{github_number}"))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kata_available_returns_bool() {
        // Just verify it runs without panicking; actual value depends on the environment.
        let _ = kata_available();
    }

    #[test]
    fn parse_ref_finds_kata_line() {
        let body = "**Why**\nSome text.\n\nKata: abc4";
        assert_eq!(parse_ref(body), Some("abc4".to_string()));
    }

    #[test]
    fn parse_ref_returns_none_when_absent() {
        let body = "**Why**\nSome text with no kata ref.";
        assert_eq!(parse_ref(body), None);
    }

    #[test]
    fn parse_ref_ignores_blank_ref() {
        let body = "Kata: \n";
        assert_eq!(parse_ref(body), None);
    }

    #[test]
    fn content_hash_stable_for_same_input() {
        assert_eq!(content_hash("hello"), content_hash("hello"));
    }

    #[test]
    fn content_hash_differs_for_different_input() {
        assert_ne!(content_hash("hello"), content_hash("world"));
    }

    #[test]
    fn strip_kata_ref_line_removes_trailing_marker() {
        let body = "Some body text.\nMore text.\nKata: abc4";
        assert_eq!(strip_kata_ref_line(body), "Some body text.\nMore text.");
    }

    #[test]
    fn strip_kata_ref_line_noop_when_absent() {
        let body = "Some body text.\nMore text.";
        assert_eq!(strip_kata_ref_line(body), body);
    }

    #[test]
    fn strip_github_prefix_removes_leading_marker() {
        let body = "GitHub: https://github.com/acme/widgets/issues/42\n\nActual content.";
        assert_eq!(strip_github_prefix(body, "acme/widgets"), "Actual content.");
    }

    #[test]
    fn strip_github_prefix_noop_when_repo_mismatches() {
        let body = "GitHub: https://github.com/other/repo/issues/42\n\nActual content.";
        assert_eq!(strip_github_prefix(body, "acme/widgets"), body);
    }

    #[test]
    fn strip_kata_ref_and_github_prefix_round_trip() {
        let repo = "acme/widgets";
        let user_body = "Line one.\nLine two.";
        let gh_body = format!("{user_body}\nKata: abc4");
        let kata_body = format!("GitHub: https://github.com/{repo}/issues/7\n\n{user_body}");
        assert_eq!(strip_kata_ref_line(&gh_body), user_body);
        assert_eq!(strip_github_prefix(&kata_body, repo), user_body);
    }

    #[test]
    fn find_kata_ref_line_finds_it() {
        let body = "Some content.\nKata: abc4";
        assert_eq!(find_kata_ref_line(body), Some("abc4"));
    }

    #[test]
    fn find_kata_ref_line_none_when_absent() {
        assert_eq!(find_kata_ref_line("no marker here"), None);
    }

    #[test]
    fn find_github_issue_number_parses_it() {
        let body = "GitHub: https://github.com/acme/widgets/issues/42\n\nContent.";
        assert_eq!(find_github_issue_number(body, "acme/widgets"), Some(42));
    }

    #[test]
    fn find_github_issue_number_none_for_wrong_repo() {
        let body = "GitHub: https://github.com/other/repo/issues/42\n\nContent.";
        assert_eq!(find_github_issue_number(body, "acme/widgets"), None);
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
}
