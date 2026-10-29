//! kata, the optional local issue ledger engram mirrors GitHub issues into:
//! the `kata` CLI wrapper, the `Kata:`/`GitHub:` lines that link an issue to
//! its mirror, and lookup of mirrors. Ticket metadata lives in `tracker`, the
//! two-way sync in `sync`.

pub mod sync;
mod tracker;

use anyhow::{Context, Result};
use serde::Deserialize;
use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::OnceLock;

use crate::{github, process};

pub use tracker::{parse_jira_arg, parse_type_arg, TrackerMeta, JIRA_META_KEY, TYPE_META_KEY};

/// Repo-root file binding a repo to kata; without it engram never calls kata.
pub const KATA_CONFIG: &str = ".kata.toml";
/// What a GitHub issue body's `Kata: <ref>` line starts with (written with a space after it).
const KATA_REF_KEY: &str = "Kata:";
const GITHUB_PREFIX: &str = "GitHub: ";
const PLAN_IDEMPOTENCY_PREFIX: &str = "engram-plan-";
const OBJECTIVE_IDEMPOTENCY_PREFIX: &str = "engram-objective-";
/// kata's open status; GitHub states are lower-cased to compare with it.
pub const STATUS_OPEN: &str = "open";
/// kata's closed status.
pub const STATUS_CLOSED: &str = "closed";
/// kata link type for "this issue waits on that one".
const BLOCKED_BY_LINK: &str = "blocked_by";
/// `error.kind` kata reports for a ref that doesn't exist.
const NOT_FOUND_KIND: &str = "not_found";

/// Idempotency key for a plan issue's kata mirror. Every path that creates the
/// mirror uses it, so they can never produce two mirrors for one plan.
pub fn plan_idempotency_key(github_number: u64) -> String {
    format!("{PLAN_IDEMPOTENCY_PREFIX}{github_number}")
}

/// Idempotency key for an objective issue's kata mirror.
pub fn objective_idempotency_key(github_number: u64) -> String {
    format!("{OBJECTIVE_IDEMPOTENCY_PREFIX}{github_number}")
}

/// Body of a kata mirror: the GitHub URL, then the full GitHub body, so the
/// mirror stays searchable on its own.
pub fn mirror_body(issue_url: &str, gh_body: &str) -> String {
    format!("{GITHUB_PREFIX}{issue_url}\n\n{gh_body}")
}

/// A GitHub body with the `Kata: <ref>` line appended that points at its mirror.
pub fn with_ref_line(gh_body: &str, kata_ref: &str) -> String {
    format!("{gh_body}\n{KATA_REF_KEY} {kata_ref}")
}

fn is_ref_line(line: &str) -> bool {
    line.trim_start().starts_with(KATA_REF_KEY)
}

/// The ref in a GitHub body's `Kata: <ref>` line, which links it to its kata mirror.
pub fn parse_ref(gh_body: &str) -> Option<&str> {
    gh_body
        .lines()
        .find_map(|line| line.trim().strip_prefix(KATA_REF_KEY))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// A GitHub body without its `Kata: <ref>` line, comparable to kata's own body.
fn strip_ref_line(gh_body: &str) -> String {
    gh_body
        .lines()
        .filter(|line| !is_ref_line(line))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

/// The `GitHub: <url>` prefix a mirror of an issue in `repo` starts with, up to the number.
fn github_line_prefix(repo: &str) -> String {
    format!("{GITHUB_PREFIX}{}", github::issues_url_prefix(repo))
}

/// A kata mirror body without its leading `GitHub: <url>\n\n` line, comparable
/// to GitHub's own body. Bodies mirroring another repo are returned as they are.
fn strip_github_prefix(kata_body: &str, repo: &str) -> String {
    let Some(rest) = kata_body.strip_prefix(&github_line_prefix(repo)) else {
        return kata_body.to_string();
    };
    let Some(newline) = rest.find('\n') else {
        return String::new();
    };
    rest[newline..].trim_start_matches('\n').to_string()
}

/// The GitHub issue number in a kata mirror's `GitHub: <url>` prefix.
fn github_issue_number(kata_body: &str, repo: &str) -> Option<u64> {
    let rest = kata_body.strip_prefix(&github_line_prefix(repo))?;
    rest.split(['\n', ' ']).next()?.parse().ok()
}

/// Read the tracker metadata (`jira`, `type`) already set on a kata issue.
pub fn tracker_meta_of(kata_ref: &str) -> Result<TrackerMeta> {
    Ok(TrackerMeta::from_metadata(&show(kata_ref)?.metadata))
}

static KATA_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Returns true iff `kata` is on PATH and the current repo has a `.kata.toml`
/// binding, unless [`disable`] was called. Cached for the process.
pub fn kata_available() -> bool {
    *KATA_AVAILABLE.get_or_init(detect)
}

/// Turn kata off for the rest of the process (`--no-kata`). Must run before
/// anything asks [`kata_available`]; the first answer is cached.
pub fn disable() {
    if KATA_AVAILABLE.set(false).is_err() {
        eprintln!("warning: --no-kata came too late; kata availability was already decided");
    }
}

fn detect() -> bool {
    let binary_ok = Command::new("kata")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());

    binary_ok && crate::config::find_repo_root().is_ok_and(|root| root.join(KATA_CONFIG).exists())
}

/// Create a kata issue and return its short ref (e.g. "abc4").
/// The idempotency key ensures retries after partial failure don't duplicate the issue.
/// `parent`, when given, links the new issue under that kata ref (containment only).
/// `meta` fields that are set become initial kata metadata (`jira`, `type`).
pub fn create(
    title: &str,
    body: &str,
    idempotency_key: &str,
    parent: Option<&str>,
    meta: &TrackerMeta,
) -> Result<String> {
    let meta_args = meta.create_meta_args();
    let mut args = vec![
        "create",
        title,
        "--body",
        body,
        "--idempotency-key",
        idempotency_key,
    ];
    if let Some(p) = parent {
        args.push("--parent");
        args.push(p);
    }
    for pair in &meta_args {
        args.push("--meta");
        args.push(pair);
    }
    args.push("--json");

    let stdout = kata_command(&args)?;
    let v: serde_json::Value =
        serde_json::from_str(&stdout).context("parsing kata create JSON output")?;
    v["issue"]["short_id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("kata create JSON missing issue.short_id: {stdout}"))
}

/// The kata mirror to create for a GitHub issue engram just opened.
#[derive(Debug)]
pub struct MirrorSpec<'a> {
    pub title: &'a str,
    pub body: &'a str,
    pub idempotency_key: String,
    pub parent: Option<&'a str>,
    pub meta: TrackerMeta,
}

/// Mirror a newly created GitHub issue into kata, then point the issue at its
/// mirror with a `Kata:` line. Best effort, and a no-op when kata is off: a
/// failure is only a warning, since the GitHub issue stands either way.
pub fn mirror_new_issue(repo: &str, issue: &github::CreatedIssue, spec: &MirrorSpec) {
    if !kata_available() {
        return;
    }
    let kata_ref = match create(
        spec.title,
        &mirror_body(&issue.url, spec.body),
        &spec.idempotency_key,
        spec.parent,
        &spec.meta,
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("warning: kata create failed (continuing): {e:#}");
            return;
        }
    };
    let updated_body = with_ref_line(spec.body, &kata_ref);
    if let Err(e) = github::update_issue_body(repo, issue.number, &updated_body) {
        eprintln!("warning: could not append Kata ref to GitHub issue: {e:#}");
    }
}

/// Close a kata issue with commit evidence. If the issue was already deleted or
/// never existed (out-of-band), treats that as success rather than failure —
/// the intent (closed) has effectively been satisfied.
pub fn close(kata_ref: &str, message: &str, commit_sha: &str) -> Result<()> {
    let mut cmd = Command::new("kata");
    cmd.args([
        "close",
        kata_ref,
        "--done",
        "--message",
        message,
        "--commit",
        commit_sha,
        "--json",
    ]);
    let output = process::spawn(&mut cmd)?;
    if output.status.success() || error_kind(&output.stderr).as_deref() == Some(NOT_FOUND_KIND) {
        return Ok(());
    }
    Err(process::failure(&cmd, &output))
}

/// `error.kind` of kata's JSON error output, if it is one.
fn error_kind(stderr: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(stderr).ok()?;
    v["error"]["kind"].as_str().map(str::to_string)
}

/// Post a comment to a kata issue.
pub fn comment(kata_ref: &str, body: &str) -> Result<()> {
    kata_command(&["comment", kata_ref, "--body", body])?;
    Ok(())
}

/// Claim a kata issue for the current actor. kata refuses when someone else owns it.
pub fn claim(kata_ref: &str) -> Result<()> {
    kata_command(&["claim", kata_ref])?;
    Ok(())
}

/// String, number and bool metadata values as strings. Structured values (such
/// as `engram.sync`) are bookkeeping, not template input, and are skipped.
pub fn metadata_map(metadata: &serde_json::Value) -> BTreeMap<String, String> {
    let Some(obj) = metadata.as_object() else {
        return BTreeMap::new();
    };
    obj.iter()
        .filter_map(|(k, v)| {
            let value = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => return None,
            };
            Some((k.clone(), value))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Shell wrappers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct KataRefShort {
    short_id: String,
}

#[derive(Debug, Deserialize)]
struct KataLink {
    from: KataRefShort,
    to: KataRefShort,
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
    owner: Option<String>,
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
/// resolved to `short_ids` (kata's `links` array is a mix of relationship kinds).
#[derive(Debug)]
pub struct KataIssue {
    pub title: String,
    pub body: String,
    pub status: String,
    pub owner: Option<String>,
    pub metadata: serde_json::Value,
    pub parent: Option<String>,
    pub blocked_by: Vec<String>,
}

fn kata_command(args: &[&str]) -> Result<String> {
    process::run_checked(Command::new("kata").args(args))
        .context("running kata CLI (is it installed?)")
}

pub fn show(kata_ref: &str) -> Result<KataIssue> {
    let stdout = kata_command(&["show", kata_ref, "--json"])?;
    let resp: ShowResponse = serde_json::from_str(&stdout).context("parsing kata show JSON")?;
    let blocked_by = resp
        .links
        .iter()
        .filter(|l| l.kind == BLOCKED_BY_LINK && l.from.short_id == resp.issue.short_id)
        .map(|l| l.to.short_id.clone())
        .collect();
    Ok(KataIssue {
        title: resp.issue.title,
        body: resp.issue.body,
        status: resp.issue.status,
        owner: resp.issue.owner,
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
#[derive(Debug, Default)]
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
// Mirror lookup
// ---------------------------------------------------------------------------

/// Finds the kata mirrors of GitHub issues, listing kata's issues at most once.
#[derive(Debug, Default)]
pub struct Mirrors {
    listed: OnceCell<Vec<KataListEntry>>,
}

impl Mirrors {
    fn listed(&self) -> Result<&[KataListEntry]> {
        if let Some(entries) = self.listed.get() {
            return Ok(entries);
        }
        let entries = list_all()?;
        Ok(self.listed.get_or_init(|| entries))
    }

    /// The kata mirror of GitHub issue `github_number`, whose body is already in
    /// hand: its `Kata: <ref>` line, else a kata issue whose `GitHub: <url>`
    /// line points at it (hand-made mirrors never got the `Kata:` line).
    pub fn find(&self, repo: &str, github_number: u64, gh_body: &str) -> Result<Option<String>> {
        if let Some(kata_ref) = parse_ref(gh_body) {
            return Ok(Some(kata_ref.to_string()));
        }
        Ok(self
            .listed()?
            .iter()
            .find(|k| github_issue_number(&k.body, repo) == Some(github_number))
            .map(|k| k.short_id.clone()))
    }

    /// [`Mirrors::find`] for an issue whose body isn't in hand yet.
    pub fn find_for_issue(&self, repo: &str, github_number: u64) -> Result<Option<String>> {
        let issue = github::get_issue(repo, github_number)
            .with_context(|| format!("fetching GitHub issue #{github_number}"))?;
        self.find(repo, github_number, issue.body_text())
    }

    /// Every (kata ref, GitHub issue number) pair whose mirror names `repo`.
    fn pairs(&self, repo: &str) -> Result<Vec<(String, u64)>> {
        Ok(self
            .listed()?
            .iter()
            .filter_map(|k| Some((k.short_id.clone(), github_issue_number(&k.body, repo)?)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_map_keeps_scalars_only() {
        let v = serde_json::json!({
            "jira": "ENG-7", "points": 3, "urgent": true,
            "engram.sync": {"gh_status": "open"}, "tags": ["a"], "none": null,
        });
        let want = [("jira", "ENG-7"), ("points", "3"), ("urgent", "true")]
            .map(|(k, v)| (k.to_string(), v.to_string()));
        assert_eq!(metadata_map(&v), BTreeMap::from(want));
        assert!(metadata_map(&serde_json::Value::Null).is_empty());
    }

    #[test]
    fn parse_ref_finds_kata_line() {
        let body = "**Why**\nSome text.\n\nKata: abc4";
        assert_eq!(parse_ref(body), Some("abc4"));
    }

    #[test]
    fn parse_ref_returns_none_when_absent() {
        let body = "**Why**\nSome text with no kata ref.";
        assert_eq!(parse_ref(body), None);
    }

    #[test]
    fn parse_ref_ignores_blank_ref() {
        assert_eq!(parse_ref("Kata: \n"), None);
    }

    #[test]
    fn parse_ref_trims_and_accepts_indentation() {
        assert_eq!(parse_ref("body text\nKata:   zzzz9  \n"), Some("zzzz9"));
        assert_eq!(parse_ref("body\n  Kata: abc4"), Some("abc4"));
    }

    #[test]
    fn ref_line_round_trips() {
        let body = with_ref_line("Some body text.\nMore text.", "abc4");
        assert_eq!(parse_ref(&body), Some("abc4"));
        assert_eq!(strip_ref_line(&body), "Some body text.\nMore text.");
    }

    #[test]
    fn strip_ref_line_noop_when_absent() {
        let body = "Some body text.\nMore text.";
        assert_eq!(strip_ref_line(body), body);
    }

    #[test]
    fn strip_ref_line_removes_an_indented_line_parse_ref_reads() {
        let body = "Body.\n  Kata: abc4";
        assert_eq!(parse_ref(body), Some("abc4"));
        assert_eq!(strip_ref_line(body), "Body.");
    }

    #[test]
    fn strip_github_prefix_removes_leading_marker() {
        let body = mirror_body(&github::issue_url("acme/widgets", 42), "Actual content.");
        assert_eq!(
            strip_github_prefix(&body, "acme/widgets"),
            "Actual content."
        );
    }

    #[test]
    fn strip_github_prefix_noop_when_repo_mismatches() {
        let body = "GitHub: https://github.com/other/repo/issues/42\n\nActual content.";
        assert_eq!(strip_github_prefix(body, "acme/widgets"), body);
    }

    #[test]
    fn github_issue_number_parses_it() {
        let body = "GitHub: https://github.com/acme/widgets/issues/42\n\nContent.";
        assert_eq!(github_issue_number(body, "acme/widgets"), Some(42));
    }

    #[test]
    fn github_issue_number_none_for_wrong_repo() {
        let body = "GitHub: https://github.com/other/repo/issues/42\n\nContent.";
        assert_eq!(github_issue_number(body, "acme/widgets"), None);
    }

    #[test]
    fn error_kind_reads_kata_json_errors_only() {
        assert_eq!(
            error_kind(br#"{"error":{"kind":"not_found","message":"no such issue"}}"#).as_deref(),
            Some(NOT_FOUND_KIND)
        );
        assert_eq!(error_kind(b"plain text failure"), None);
    }
}
