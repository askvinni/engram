use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct Issue {
    pub title: String,
    pub body: Option<String>,
    pub state: String,
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

fn gh(args: &[&str]) -> Result<String> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .context("running gh CLI (is it installed and authenticated?)")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8(output.stdout)?)
}

pub fn ensure_label(repo: &str, name: &str, color: &str, description: &str) -> Result<()> {
    let output = Command::new("gh")
        .args([
            "label",
            "create",
            name,
            "--repo",
            repo,
            "--color",
            color,
            "--description",
            description,
        ])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("already exists") {
            anyhow::bail!("failed to create label {}: {}", name, stderr.trim());
        }
    }
    Ok(())
}

pub fn create_issue(repo: &str, title: &str, body: &str, label: &str) -> Result<String> {
    gh(&[
        "issue", "create", "--repo", repo, "--title", title, "--body", body, "--label", label,
    ])
}

pub fn get_issue(repo: &str, number: u64) -> Result<Issue> {
    let out = gh(&[
        "issue",
        "view",
        &number.to_string(),
        "--repo",
        repo,
        "--json",
        "title,body,state",
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
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("repo must be owner/name, got: {repo}"))?;

    let query = r#"
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
    "#;

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

    let merged = |pr: &PrFields| pr.state == "MERGED";
    let same_repo = |pr: &PrFields| {
        pr.repository
            .as_ref()
            .is_some_and(|r| r.name_with_owner.eq_ignore_ascii_case(repo))
    };

    let pr = match closer.filter(merged) {
        Some(pr) => pr,
        None => {
            referencing.retain(|(_, pr)| merged(pr) && same_repo(pr));
            // max_by_key keeps the last maximum, i.e. the most recent reference
            referencing
                .into_iter()
                .max_by_key(|(will_close, _)| *will_close)
                .map(|(_, pr)| pr)?
        }
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

pub fn find_pr_for_branch(repo: &str, branch: &str) -> Result<Option<PullRequest>> {
    let out = gh(&[
        "pr",
        "list",
        "--repo",
        repo,
        "--head",
        branch,
        "--json",
        "number,title,body,state,headRefName",
        "--limit",
        "1",
    ])?;
    let mut prs: Vec<PullRequest> = serde_json::from_str(&out).context("parsing PR list JSON")?;
    Ok(if prs.is_empty() {
        None
    } else {
        Some(prs.remove(0))
    })
}

pub fn list_open_plans(repo: &str) -> Result<Vec<PlanIssue>> {
    let out = gh(&[
        "issue",
        "list",
        "--repo",
        repo,
        "--label",
        "engram-plan",
        "--state",
        "open",
        "--json",
        "number,title,createdAt",
        "--limit",
        "50",
    ])?;
    serde_json::from_str(&out).context("parsing issue list JSON")
}

pub fn create_pr(repo: &str, title: &str, body: &str, label: &str) -> Result<String> {
    gh(&[
        "pr", "create", "--repo", repo, "--title", title, "--body", body, "--label", label,
    ])
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
    gh(&[
        "issue",
        "edit",
        &number.to_string(),
        "--repo",
        repo,
        "--body",
        body,
    ])?;
    Ok(())
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
    let output = Command::new("gh")
        .args(["issue", "reopen", &number.to_string(), "--repo", repo])
        .output()
        .context("running gh CLI")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already open") {
            return Ok(());
        }
        anyhow::bail!("gh issue reopen #{number} failed: {}", stderr.trim());
    }
    Ok(())
}

pub fn list_open_objectives(repo: &str) -> Result<Vec<PlanIssue>> {
    let out = gh(&[
        "issue",
        "list",
        "--repo",
        repo,
        "--label",
        "engram-objective",
        "--state",
        "open",
        "--json",
        "number,title,createdAt",
        "--limit",
        "50",
    ])?;
    serde_json::from_str(&out).context("parsing issue list JSON")
}

pub fn add_sub_issue(repo: &str, parent_number: u64, child_number: u64) -> Result<()> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("repo must be owner/name, got: {repo}"))?;

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
    let out = gh(&[
        "issue",
        "view",
        &number.to_string(),
        "--repo",
        repo,
        "--json",
        "comments",
    ])?;

    #[derive(Deserialize)]
    struct Response {
        comments: Vec<IssueComment>,
    }

    let resp: Response = serde_json::from_str(&out).context("parsing issue comments JSON")?;
    Ok(resp.comments)
}

pub fn close_issue(repo: &str, number: u64) -> Result<()> {
    let output = Command::new("gh")
        .args(["issue", "close", &number.to_string(), "--repo", repo])
        .output()
        .context("running gh CLI")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already closed") {
            return Ok(());
        }
        anyhow::bail!("gh issue close #{number} failed: {}", stderr.trim());
    }
    Ok(())
}

pub fn list_unlearned_plans(repo: &str) -> Result<Vec<PlanIssue>> {
    #[derive(Deserialize)]
    struct LabelEntry {
        name: String,
    }
    #[derive(Deserialize)]
    struct IssueWithLabels {
        number: u64,
        title: String,
        #[serde(rename = "createdAt")]
        created_at: String,
        labels: Vec<LabelEntry>,
    }

    let out = gh(&[
        "issue",
        "list",
        "--repo",
        repo,
        "--label",
        "engram-plan",
        "--state",
        "closed",
        "--json",
        "number,title,createdAt,labels",
        "--limit",
        "100",
    ])?;
    let all: Vec<IssueWithLabels> =
        serde_json::from_str(&out).context("parsing issue list JSON")?;
    Ok(all
        .into_iter()
        .filter(|i| !i.labels.iter().any(|l| l.name == "engram-learned"))
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
}
