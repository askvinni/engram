use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct Issue {
    pub title: String,
    pub body: Option<String>,
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    #[serde(rename = "headRefName", default)]
    pub head_ref_name: Option<String>,
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

pub fn find_linked_pr(repo: &str, issue_number: u64) -> Result<Option<PullRequest>> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("repo must be owner/name, got: {repo}"))?;

    let query = r#"
        query($owner: String!, $repo: String!, $number: Int!) {
          repository(owner: $owner, name: $repo) {
            issue(number: $number) {
              timelineItems(itemTypes: [CLOSED_EVENT], last: 1) {
                nodes {
                  ... on ClosedEvent {
                    closer {
                      ... on PullRequest {
                        number
                        title
                        body
                        headRefName
                        state
                      }
                    }
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
        nodes: Vec<ClosedEventNode>,
    }
    #[derive(Deserialize)]
    struct ClosedEventNode {
        closer: Option<PrFields>,
    }
    #[derive(Deserialize)]
    struct PrFields {
        number: u64,
        title: String,
        body: Option<String>,
        #[serde(rename = "headRefName", default)]
        head_ref_name: Option<String>,
        state: String,
    }

    let resp: Response = serde_json::from_str(&out).context("parsing GraphQL response")?;
    let closer = resp
        .data
        .repository
        .issue
        .timeline_items
        .nodes
        .into_iter()
        .next()
        .and_then(|n| n.closer)
        .filter(|pr| pr.state == "MERGED");

    Ok(closer.map(|pr| PullRequest {
        number: pr.number,
        title: pr.title,
        body: pr.body,
        head_ref_name: pr.head_ref_name,
    }))
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
