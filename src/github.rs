use crate::{config, process};
use anyhow::{Context, Result};
use serde::Deserialize;

/// Label on every plan issue engram creates.
pub const PLAN_LABEL: &str = "engram-plan";
/// Label on every objective issue engram creates.
pub const OBJECTIVE_LABEL: &str = "engram-objective";
/// Label marking a plan issue whose learnings have been synthesized (and the PRs that carry them).
pub const LEARNED_LABEL: &str = "engram-learned";

/// The labels `engram init` creates: (name, color, description).
pub const ENGRAM_LABELS: &[(&str, &str, &str)] = &[
    (PLAN_LABEL, "0075ca", "Plan issue created by engram"),
    (LEARNED_LABEL, "e4e669", "Learning PR created by engram"),
    (
        OBJECTIVE_LABEL,
        "5319e7",
        "Objective issue created by engram",
    ),
];

/// Most open plans or objectives listed.
const OPEN_ISSUE_LIMIT: &str = "50";
/// Most closed plans scanned for ones not yet learned.
const CLOSED_PLAN_LIMIT: &str = "100";
/// Most PRs listed when looking for one whose body closes an issue.
const PR_SCAN_LIMIT: &str = "200";

/// GitHub's closing keywords, lower-cased.
const CLOSING_KEYWORDS: &[&str] = &[
    "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
];

/// `https://github.com/<repo>/issues/`, the prefix of every issue URL in `repo`.
pub fn issues_url_prefix(repo: &str) -> String {
    format!("https://github.com/{repo}/issues/")
}

/// The URL of issue `number` in `repo`.
pub fn issue_url(repo: &str, number: u64) -> String {
    format!("{}{number}", issues_url_prefix(repo))
}

#[derive(Debug, Deserialize)]
pub struct Issue {
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<Label>,
    /// `gh issue view` also returns pull requests; their URL says `/pull/`.
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct Label {
    pub name: String,
}

impl Issue {
    pub fn has_label(&self, name: &str) -> bool {
        self.labels.iter().any(|l| l.name == name)
    }

    pub fn is_pull_request(&self) -> bool {
        self.url.contains("/pull/")
    }

    pub fn is_open(&self) -> bool {
        self.state.eq_ignore_ascii_case("open")
    }

    pub fn is_closed(&self) -> bool {
        self.state.eq_ignore_ascii_case("closed")
    }

    pub fn body_text(&self) -> &str {
        self.body.as_deref().unwrap_or("")
    }
}

/// The PR states engram lists PRs by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Merged,
}

impl PrState {
    fn as_arg(self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Merged => "merged",
        }
    }
}

/// A PR found for a plan (open, or merged when looked up by state), with its head and base.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OpenPr {
    #[serde(default)]
    pub number: u64,
    pub url: String,
    #[serde(rename = "baseRefName")]
    pub base: String,
    #[serde(rename = "headRefName", default)]
    pub head: String,
    #[serde(rename = "isDraft", default)]
    pub is_draft: bool,
    #[serde(default)]
    pub title: String,
}

/// JSON fields `gh pr list/view` must return to fill an `OpenPr`.
const OPEN_PR_FIELDS: &str = "number,url,baseRefName,headRefName,isDraft,title";

/// A PR with its body (and state, when `gh pr view` reports it).
#[derive(Debug, Deserialize)]
pub struct PrWithBody {
    #[serde(flatten)]
    pub pr: OpenPr,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub state: String,
    /// Opened from a fork: its head branch isn't on origin.
    #[serde(rename = "isCrossRepository", default)]
    pub cross_repo: bool,
}

/// JSON fields `gh pr list/view` must return to fill a `PrWithBody`.
fn pr_with_body_fields() -> String {
    format!("{OPEN_PR_FIELDS},body,isCrossRepository")
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    #[serde(rename = "headRefName", default)]
    pub head_ref_name: Option<String>,
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IssueComment {
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct PlanIssue {
    pub number: u64,
    pub title: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

/// An issue `gh issue create` just opened.
#[derive(Debug)]
pub struct CreatedIssue {
    pub number: u64,
    pub url: String,
}

fn gh(args: &[&str]) -> Result<String> {
    process::run_checked(config::gh_command()?.args(args))
        .context("running gh CLI (is it installed and authenticated?)")
}

/// `owner` and `name` of an `owner/name` repo.
fn split_repo(repo: &str) -> Result<(&str, &str)> {
    repo.split_once('/')
        .ok_or_else(|| anyhow::anyhow!("repo must be owner/name, got: {repo}"))
}

/// The issue number at the end of an issue URL.
fn issue_number_from_url(url: &str) -> Result<u64> {
    url.trim()
        .rsplit('/')
        .next()
        .and_then(|s| s.parse().ok())
        .with_context(|| format!("could not parse issue number from URL: {url}"))
}

pub fn ensure_label(repo: &str, name: &str, color: &str, description: &str) -> Result<()> {
    let mut cmd = config::gh_command()?;
    cmd.args([
        "label",
        "create",
        name,
        "--repo",
        repo,
        "--color",
        color,
        "--description",
        description,
    ]);
    process::run_tolerating(&mut cmd, "already exists")
        .with_context(|| format!("creating label {name}"))
}

pub fn create_issue(repo: &str, title: &str, body: &str, label: &str) -> Result<CreatedIssue> {
    let out = gh(&[
        "issue", "create", "--repo", repo, "--title", title, "--body", body, "--label", label,
    ])?;
    let url = out
        .lines()
        .last()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    Ok(CreatedIssue {
        number: issue_number_from_url(&url)?,
        url,
    })
}

pub fn get_issue(repo: &str, number: u64) -> Result<Issue> {
    let out = gh(&[
        "issue",
        "view",
        &number.to_string(),
        "--repo",
        repo,
        "--json",
        "title,body,state,labels,url",
    ])?;
    serde_json::from_str(&out).context("parsing issue JSON")
}

/// Find the merged PR that delivered an issue.
///
/// Prefers the PR that closed the issue. When the issue was closed by hand — e.g. a
/// stacked PR merged into a non-default base, where closing keywords don't fire —
/// falls back to the most recent merged PR in the same repo that references the
/// issue, preferring ones whose body carries a closing keyword.
pub fn find_linked_pr(repo: &str, issue_number: u64) -> Result<Option<PullRequest>> {
    #[derive(Deserialize)]
    struct Response {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        repository: Repo,
    }
    #[derive(Deserialize)]
    struct Repo {
        issue: GhIssue,
    }
    #[derive(Deserialize)]
    struct GhIssue {
        #[serde(rename = "timelineItems")]
        timeline_items: TimelineItems,
    }
    #[derive(Deserialize)]
    struct TimelineItems {
        nodes: Vec<TimelineNode>,
    }

    let (owner, name) = split_repo(repo)?;

    let query = r"
        fragment Pr on PullRequest {
          number
          title
          body
          headRefName
          state
          mergeCommit { oid }
          repository { nameWithOwner }
        }
        query($owner: String!, $repo: String!, $number: Int!) {
          repository(owner: $owner, name: $repo) {
            issue(number: $number) {
              timelineItems(itemTypes: [CLOSED_EVENT, CROSS_REFERENCED_EVENT], last: 100) {
                nodes {
                  __typename
                  ... on ClosedEvent {
                    closer { __typename ...Pr }
                  }
                  ... on CrossReferencedEvent {
                    willCloseTarget
                    source { __typename ...Pr }
                  }
                }
              }
            }
          }
        }
    ";

    let out = gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={query}"),
        "-f",
        &format!("owner={owner}"),
        "-f",
        &format!("repo={name}"),
        "-F",
        &format!("number={issue_number}"),
    ])?;

    let resp: Response = serde_json::from_str(&out).context("parsing GraphQL response")?;
    Ok(pick_linked_pr(
        resp.data.repository.issue.timeline_items.nodes,
        repo,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum TimelineNode {
    ClosedEvent {
        closer: Option<Subject>,
    },
    CrossReferencedEvent {
        #[serde(rename = "willCloseTarget", default)]
        will_close_target: bool,
        source: Subject,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum Subject {
    PullRequest(PrFields),
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct PrFields {
    number: u64,
    title: String,
    body: Option<String>,
    #[serde(rename = "headRefName", default)]
    head_ref_name: Option<String>,
    state: String,
    #[serde(rename = "mergeCommit", default)]
    merge_commit: Option<MergeCommit>,
    #[serde(default)]
    repository: Option<PrRepo>,
}

#[derive(Debug, Deserialize)]
struct MergeCommit {
    oid: String,
}

#[derive(Debug, Deserialize)]
struct PrRepo {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

/// Timeline nodes arrive oldest first.
fn pick_linked_pr(nodes: Vec<TimelineNode>, repo: &str) -> Option<PullRequest> {
    let mut closer = None;
    let mut referencing = Vec::new();
    for node in nodes {
        match node {
            TimelineNode::ClosedEvent { closer: c } => {
                closer = match c {
                    Some(Subject::PullRequest(pr)) => Some(pr),
                    _ => None,
                }
            }
            TimelineNode::CrossReferencedEvent {
                will_close_target,
                source: Subject::PullRequest(pr),
            } => referencing.push((will_close_target, pr)),
            _ => {}
        }
    }

    let merged = |pr: &PrFields| pr.state.eq_ignore_ascii_case("merged");
    let same_repo = |pr: &PrFields| {
        pr.repository
            .as_ref()
            .is_some_and(|r| r.name_with_owner.eq_ignore_ascii_case(repo))
    };

    let pr = if let Some(pr) = closer.filter(merged) {
        pr
    } else {
        referencing.retain(|(_, pr)| merged(pr) && same_repo(pr));
        // max_by_key keeps the last maximum, i.e. the most recent reference
        referencing
            .into_iter()
            .max_by_key(|(will_close, _)| *will_close)
            .map(|(_, pr)| pr)?
    };

    Some(PullRequest {
        number: pr.number,
        title: pr.title,
        body: pr.body,
        head_ref_name: pr.head_ref_name,
        merge_commit_sha: pr.merge_commit.map(|m| m.oid),
    })
}

pub fn get_pr_diff(repo: &str, pr_number: u64) -> Result<String> {
    gh(&["pr", "diff", &pr_number.to_string(), "--repo", repo])
}

/// Open issues carrying `label`, newest first.
pub fn list_open_issues(repo: &str, label: &str) -> Result<Vec<PlanIssue>> {
    let out = gh(&[
        "issue",
        "list",
        "--repo",
        repo,
        "--label",
        label,
        "--state",
        "open",
        "--json",
        "number,title,createdAt",
        "--limit",
        OPEN_ISSUE_LIMIT,
    ])?;
    serde_json::from_str(&out).context("parsing issue list JSON")
}

pub fn create_pr(repo: &str, title: &str, body: &str, label: &str) -> Result<String> {
    gh(&[
        "pr", "create", "--repo", repo, "--title", title, "--body", body, "--label", label,
    ])
}

/// The PR in `state` whose head is `branch`, if any.
pub fn pr_for_branch(repo: &str, branch: &str, state: PrState) -> Result<Option<PrWithBody>> {
    let out = gh(&[
        "pr",
        "list",
        "--repo",
        repo,
        "--head",
        branch,
        "--state",
        state.as_arg(),
        "--json",
        &pr_with_body_fields(),
        "--limit",
        "1",
    ])?;
    let prs: Vec<PrWithBody> = serde_json::from_str(&out).context("parsing PR list JSON")?;
    Ok(prs.into_iter().next())
}

/// Numbers of the PRs in `repo` that GitHub links as closing an issue, from
/// `gh issue view --json closedByPullRequestsReferences`.
pub fn parse_closing_pr_numbers(json: &str, repo: &str) -> Result<Vec<u64>> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("parsing closedByPullRequestsReferences JSON")?;
    let refs = v["closedByPullRequestsReferences"].as_array();
    Ok(refs
        .into_iter()
        .flatten()
        .filter(|r| {
            match (
                r["repository"]["owner"]["login"].as_str(),
                r["repository"]["name"].as_str(),
            ) {
                (Some(owner), Some(name)) => format!("{owner}/{name}").eq_ignore_ascii_case(repo),
                _ => true,
            }
        })
        .filter_map(|r| r["number"].as_u64())
        .collect())
}

/// The open PR GitHub links as closing issue `number`, whatever its branch is
/// called. Stacked PRs (into a non-default base) are never linked: search PR
/// bodies with `select_body_closing_pr` for those.
pub fn open_closing_pr(repo: &str, number: u64) -> Result<Option<OpenPr>> {
    let n = number.to_string();
    let out = gh(&[
        "issue",
        "view",
        &n,
        "--repo",
        repo,
        "--json",
        "closedByPullRequestsReferences",
    ])?;
    for pr in parse_closing_pr_numbers(&out, repo)? {
        let fields = format!("{OPEN_PR_FIELDS},state");
        let out = gh(&[
            "pr",
            "view",
            &pr.to_string(),
            "--repo",
            repo,
            "--json",
            &fields,
        ])?;
        let view: PrWithBody =
            serde_json::from_str(&out).with_context(|| format!("parsing PR #{pr} JSON"))?;
        if view.state.eq_ignore_ascii_case(PrState::Open.as_arg()) {
            return Ok(Some(view.pr));
        }
    }
    Ok(None)
}

/// The issues of `repo` that `body` says it closes: a closing keyword (any
/// case, optional colon) followed by `#N` or the issue's URL, in body order.
pub fn closed_issues(body: &str, repo: &str) -> Vec<u64> {
    let url_prefix = issues_url_prefix(&repo.to_ascii_lowercase());
    let words: Vec<String> = body
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    words
        .windows(2)
        .filter(|w| {
            let keyword = w[0].trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
            CLOSING_KEYWORDS.contains(&keyword.trim_end_matches(':'))
        })
        .filter_map(|w| {
            let target = w[1]
                .strip_prefix('#')
                .or_else(|| w[1].strip_prefix(&url_prefix))?;
            target
                .split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()
        })
        .collect()
}

/// Whether `body` says it closes issue `number` of `repo` (see [`closed_issues`]).
pub fn body_closes(body: &str, repo: &str, number: u64) -> bool {
    closed_issues(body, repo).contains(&number)
}

/// The PR among `prs` whose body closes issue `number` of `repo`, skipping
/// fork PRs and preferring one whose head is `head`.
pub fn select_body_closing_pr<'a>(
    prs: &'a [PrWithBody],
    repo: &str,
    number: u64,
    head: Option<&str>,
) -> Option<&'a OpenPr> {
    let matching: Vec<&OpenPr> = prs
        .iter()
        .filter(|p| !p.cross_repo && body_closes(&p.body, repo, number))
        .map(|p| &p.pr)
        .collect();
    let preferred = matching
        .iter()
        .find(|pr| head.is_some_and(|h| pr.head == h));
    preferred.or(matching.first()).copied()
}

/// PRs in `state` with their bodies, newest first, up to `PR_SCAN_LIMIT`:
/// what `select_body_closing_pr` searches.
pub fn list_prs_with_body(repo: &str, state: PrState) -> Result<Vec<PrWithBody>> {
    let out = gh(&[
        "pr",
        "list",
        "--repo",
        repo,
        "--state",
        state.as_arg(),
        "--json",
        &pr_with_body_fields(),
        "--limit",
        PR_SCAN_LIMIT,
    ])?;
    serde_json::from_str(&out).context("parsing PR list JSON")
}

/// `gh stack <args>` for `repo`. gh-stack can't map an SSH host-alias remote
/// (`git@github-personal:...`) to GitHub, so the repo is named via `GH_REPO`.
pub fn gh_stack(repo_root: &std::path::Path, repo: &str, args: &[&str]) -> Result<()> {
    let mut cmd = config::gh_command()?;
    cmd.arg("stack")
        .args(args)
        .env("GH_REPO", repo)
        .current_dir(repo_root);
    process::run_checked(&mut cmd)
        .context("running gh stack (is the gh-stack extension installed?)")?;
    Ok(())
}

/// Open a draft PR from `head` into `base` and return its URL.
pub fn create_draft_pr(
    repo: &str,
    head: &str,
    base: &str,
    title: &str,
    body: &str,
) -> Result<String> {
    let out = gh(&[
        "pr", "create", "--repo", repo, "--draft", "--head", head, "--base", base, "--title",
        title, "--body", body,
    ])?;
    out.lines()
        .last()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .context("gh pr create printed no PR URL")
}

/// Name of the repo's default branch on GitHub.
pub fn default_branch(repo: &str) -> Result<String> {
    let q = ".defaultBranchRef.name";
    let out = gh(&["repo", "view", repo, "--json", "defaultBranchRef", "-q", q])?;
    if out.is_empty() {
        anyhow::bail!("gh reported no default branch for {repo}");
    }
    Ok(out)
}

pub fn add_label_to_issue(repo: &str, issue_number: u64, label: &str) -> Result<()> {
    gh(&[
        "issue",
        "edit",
        &issue_number.to_string(),
        "--repo",
        repo,
        "--add-label",
        label,
    ])?;
    Ok(())
}

pub fn update_issue_body(repo: &str, number: u64, body: &str) -> Result<()> {
    update_issue(repo, number, None, Some(body))
}

/// Update an issue's title and/or body in a single `gh issue edit` call.
/// Passing `None` for a field leaves it untouched.
pub fn update_issue(
    repo: &str,
    number: u64,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<()> {
    if title.is_none() && body.is_none() {
        return Ok(());
    }
    let number_str = number.to_string();
    let mut args = vec!["issue", "edit", &number_str, "--repo", repo];
    if let Some(t) = title {
        args.push("--title");
        args.push(t);
    }
    if let Some(b) = body {
        args.push("--body");
        args.push(b);
    }
    gh(&args)?;
    Ok(())
}

pub fn reopen_issue(repo: &str, number: u64) -> Result<()> {
    let mut cmd = config::gh_command()?;
    cmd.args(["issue", "reopen", &number.to_string(), "--repo", repo]);
    process::run_tolerating(&mut cmd, "already open")
        .with_context(|| format!("reopening issue #{number}"))
}

pub fn close_issue(repo: &str, number: u64) -> Result<()> {
    let mut cmd = config::gh_command()?;
    cmd.args(["issue", "close", &number.to_string(), "--repo", repo]);
    process::run_tolerating(&mut cmd, "already closed")
        .with_context(|| format!("closing issue #{number}"))
}

pub fn add_sub_issue(repo: &str, parent_number: u64, child_number: u64) -> Result<()> {
    #[derive(Deserialize)]
    struct IdResponse {
        data: IdData,
    }
    #[derive(Deserialize)]
    struct IdData {
        repository: IdRepo,
    }
    #[derive(Deserialize)]
    struct IdRepo {
        #[serde(rename = "parentIssue")]
        parent_issue: IdNode,
        #[serde(rename = "childIssue")]
        child_issue: IdNode,
    }
    #[derive(Deserialize)]
    struct IdNode {
        id: String,
    }

    let (owner, name) = split_repo(repo)?;

    let id_query = "query($owner: String!, $repo: String!, $parent: Int!, $child: Int!) { \
        repository(owner: $owner, name: $repo) { \
          parentIssue: issue(number: $parent) { id } \
          childIssue: issue(number: $child) { id } \
        } \
    }";

    let out = gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={id_query}"),
        "-f",
        &format!("owner={owner}"),
        "-f",
        &format!("repo={name}"),
        "-F",
        &format!("parent={parent_number}"),
        "-F",
        &format!("child={child_number}"),
    ])?;

    let resp: IdResponse = serde_json::from_str(&out).context("parsing issue node IDs")?;
    let parent_id = resp.data.repository.parent_issue.id;
    let child_id = resp.data.repository.child_issue.id;

    let mutation = "mutation($parentId: ID!, $childId: ID!) { \
        addSubIssue(input: {issueId: $parentId, subIssueId: $childId}) { \
          issue { id } \
        } \
    }";

    gh(&[
        "api",
        "graphql",
        "-f",
        &format!("query={mutation}"),
        "-f",
        &format!("parentId={parent_id}"),
        "-f",
        &format!("childId={child_id}"),
    ])?;

    Ok(())
}

pub fn add_issue_comment(repo: &str, number: u64, body: &str) -> Result<()> {
    gh(&[
        "issue",
        "comment",
        &number.to_string(),
        "--repo",
        repo,
        "--body",
        body,
    ])?;
    Ok(())
}

pub fn get_issue_comments(repo: &str, number: u64) -> Result<Vec<IssueComment>> {
    #[derive(Deserialize)]
    struct Response {
        comments: Vec<IssueComment>,
    }

    let out = gh(&[
        "issue",
        "view",
        &number.to_string(),
        "--repo",
        repo,
        "--json",
        "comments",
    ])?;

    let resp: Response = serde_json::from_str(&out).context("parsing issue comments JSON")?;
    Ok(resp.comments)
}

pub fn list_unlearned_plans(repo: &str) -> Result<Vec<PlanIssue>> {
    #[derive(Deserialize)]
    struct IssueWithLabels {
        number: u64,
        title: String,
        #[serde(rename = "createdAt")]
        created_at: String,
        labels: Vec<Label>,
    }

    let out = gh(&[
        "issue",
        "list",
        "--repo",
        repo,
        "--label",
        PLAN_LABEL,
        "--state",
        "closed",
        "--json",
        "number,title,createdAt,labels",
        "--limit",
        CLOSED_PLAN_LIMIT,
    ])?;
    let all: Vec<IssueWithLabels> =
        serde_json::from_str(&out).context("parsing issue list JSON")?;
    Ok(all
        .into_iter()
        .filter(|i| !i.labels.iter().any(|l| l.name == LEARNED_LABEL))
        .map(|i| PlanIssue {
            number: i.number,
            title: i.title,
            created_at: i.created_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const REPO: &str = "owner/repo";

    fn pick(nodes_json: &str) -> Option<u64> {
        let nodes: Vec<TimelineNode> = serde_json::from_str(nodes_json).unwrap();
        pick_linked_pr(nodes, REPO).map(|pr| pr.number)
    }

    fn pr(number: u64, state: &str, repo: &str) -> String {
        format!(
            r#"{{"__typename":"PullRequest","number":{number},"title":"t","body":"b","headRefName":"h","state":"{state}","mergeCommit":null,"repository":{{"nameWithOwner":"{repo}"}}}}"#
        )
    }

    fn closed(closer: &str) -> String {
        format!(r#"{{"__typename":"ClosedEvent","closer":{closer}}}"#)
    }

    fn xref(will_close: bool, source: &str) -> String {
        format!(
            r#"{{"__typename":"CrossReferencedEvent","willCloseTarget":{will_close},"source":{source}}}"#
        )
    }

    const ISSUE_SOURCE: &str = r#"{"__typename":"Issue"}"#;

    #[test]
    fn prefers_merged_closer() {
        let nodes = format!(
            "[{},{}]",
            xref(true, &pr(2, "MERGED", REPO)),
            closed(&pr(1, "MERGED", REPO))
        );
        assert_eq!(pick(&nodes), Some(1));
    }

    #[test]
    fn manual_close_falls_back_to_referencing_pr() {
        let nodes = format!(
            "[{},{},{}]",
            xref(false, ISSUE_SOURCE),
            xref(true, &pr(7, "MERGED", REPO)),
            closed("null")
        );
        assert_eq!(pick(&nodes), Some(7));
    }

    #[test]
    fn commit_closer_falls_back_to_referencing_pr() {
        let nodes = format!(
            "[{},{}]",
            xref(false, &pr(7, "MERGED", REPO)),
            closed(r#"{"__typename":"Commit"}"#)
        );
        assert_eq!(pick(&nodes), Some(7));
    }

    #[test]
    fn fallback_prefers_closing_keyword_then_most_recent() {
        let nodes = format!(
            "[{},{},{},{}]",
            xref(true, &pr(3, "MERGED", REPO)),
            xref(true, &pr(4, "MERGED", REPO)),
            xref(false, &pr(5, "MERGED", REPO)),
            closed("null")
        );
        assert_eq!(pick(&nodes), Some(4));
    }

    #[test]
    fn fallback_ignores_unmerged_and_other_repos() {
        let nodes = format!(
            "[{},{},{}]",
            xref(true, &pr(3, "OPEN", REPO)),
            xref(true, &pr(4, "MERGED", "someone/else")),
            closed("null")
        );
        assert_eq!(pick(&nodes), None);
    }

    #[test]
    fn unmerged_closer_falls_back() {
        let nodes = format!(
            "[{},{}]",
            xref(false, &pr(9, "MERGED", "Owner/Repo")),
            closed(&pr(1, "CLOSED", REPO))
        );
        assert_eq!(pick(&nodes), Some(9));
    }

    #[test]
    fn uses_latest_close_event() {
        let nodes = format!("[{},{}]", closed(&pr(1, "MERGED", REPO)), closed("null"));
        assert_eq!(pick(&nodes), None);
    }

    #[test]
    fn closing_pr_numbers_keep_this_repo_only() {
        let json = r#"{"closedByPullRequestsReferences":[
            {"number":147,"repository":{"name":"repo","owner":{"login":"Owner"}},"url":"u"},
            {"number":9,"repository":{"name":"fork","owner":{"login":"someone"}},"url":"u"}
        ]}"#;
        assert_eq!(parse_closing_pr_numbers(json, REPO).unwrap(), vec![147]);
    }

    #[rstest]
    #[case::closes("Work in progress.\n\nCloses #145", true)]
    #[case::lowercase_and_period("fixes #145.", true)]
    #[case::colon("Resolves: #145", true)]
    #[case::issue_url_any_case("Closes https://github.com/owner/Repo/issues/145", true)]
    #[case::longer_number_url("Closes https://github.com/owner/repo/issues/1450", false)]
    #[case::other_repo_url("Closes https://github.com/other/repo/issues/145", false)]
    #[case::longer_number("Closes #1450", false)]
    #[case::number_prefix("Closes #14", false)]
    #[case::no_keyword("See #145", false)]
    #[case::other_repo_ref("Closes other/repo#145", false)]
    fn body_closes_matches_keyword_and_exact_number(#[case] body: &str, #[case] closes: bool) {
        assert_eq!(body_closes(body, REPO, 145), closes);
    }

    #[rstest]
    #[case::first_match(None, "u2")]
    #[case::the_branch_preferred(Some("c"), "u3")]
    #[case::unknown_branch(Some("zz"), "u2")]
    fn select_body_closing_pr_skips_forks_and_prefers_the_branch(
        #[case] head: Option<&str>,
        #[case] want: &str,
    ) {
        let prs: Vec<PrWithBody> = serde_json::from_str(
            r#"[
            {"url":"u1","baseRefName":"main","headRefName":"a","isDraft":true,"title":"t","body":"Closes #1450"},
            {"url":"u0","baseRefName":"main","headRefName":"b","isDraft":true,"title":"t","body":"Closes #145","isCrossRepository":true},
            {"url":"u2","baseRefName":"a","headRefName":"b","isDraft":true,"title":"t","body":"WIP\n\nCloses #145"},
            {"url":"u3","baseRefName":"main","headRefName":"c","isDraft":true,"title":"t","body":"Closes #145"}
        ]"#,
        )
        .unwrap();
        let got = select_body_closing_pr(&prs, REPO, 145, head).map(|p| p.url.as_str());
        assert_eq!(got, Some(want));
    }

    #[rstest]
    #[case::closes("closes #42", &[42])]
    #[case::fixes_any_case("Fixes #7", &[7])]
    #[case::resolves("Resolves #100", &[100])]
    #[case::multiline("## Summary\nDoes some stuff.\n\nCloses #55\n", &[55])]
    #[case::several("Fixes #1, closes #2", &[1, 2])]
    #[case::none("no issue reference here", &[])]
    #[case::keyword_inside_a_word("prefixes #12", &[])]
    #[case::after_punctuation("Done (closes #12)", &[12])]
    // Lower-casing changes this body's byte length; a byte offset taken from
    // the lower-cased copy must never be used to slice the original.
    #[case::non_ascii("İstanbul: closes #5", &[5])]
    fn closed_issues_finds_closing_references(#[case] body: &str, #[case] want: &[u64]) {
        assert_eq!(closed_issues(body, REPO), want);
    }

    #[rstest]
    #[case::plain("https://github.com/o/r/issues/42", Ok(42))]
    #[case::trailing_newline("https://github.com/o/r/issues/7\n", Ok(7))]
    #[case::no_number("https://github.com/o/r/issues/", Err(()))]
    fn issue_number_from_url_cases(#[case] url: &str, #[case] want: std::result::Result<u64, ()>) {
        assert_eq!(issue_number_from_url(url).map_err(|_| ()), want);
    }

    #[test]
    fn issue_urls_share_one_prefix() {
        assert_eq!(issue_url(REPO, 9), "https://github.com/owner/repo/issues/9");
        assert!(issue_url(REPO, 9).starts_with(&issues_url_prefix(REPO)));
    }
}
