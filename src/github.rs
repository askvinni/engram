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
            "label", "create", name,
            "--repo", repo,
            "--color", color,
            "--description", description,
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
        "issue", "create",
        "--repo", repo,
        "--title", title,
        "--body", body,
        "--label", label,
    ])
}

pub fn get_issue(repo: &str, number: u64) -> Result<Issue> {
    let out = gh(&[
        "issue", "view", &number.to_string(),
        "--repo", repo,
        "--json", "title,body,state",
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
        "api", "graphql",
        "-f", &format!("query={query}"),
        "-f", &format!("owner={owner}"),
        "-f", &format!("repo={name}"),
        "-F", &format!("number={issue_number}"),
    ])?;

    #[derive(Deserialize)]
    struct Response { data: Data }
    #[derive(Deserialize)]
    struct Data { repository: Repo }
    #[derive(Deserialize)]
    struct Repo { issue: GhIssue }
    #[derive(Deserialize)]
    struct GhIssue {
        #[serde(rename = "timelineItems")]
        timeline_items: TimelineItems,
    }
    #[derive(Deserialize)]
    struct TimelineItems { nodes: Vec<ClosedEventNode> }
    #[derive(Deserialize)]
    struct ClosedEventNode { closer: Option<PrFields> }
    #[derive(Deserialize)]
    struct PrFields { number: u64, title: String, body: Option<String>, state: String }

    let resp: Response = serde_json::from_str(&out).context("parsing GraphQL response")?;
    let closer = resp.data.repository.issue.timeline_items.nodes
        .into_iter().next()
        .and_then(|n| n.closer)
        .filter(|pr| pr.state == "MERGED");

    Ok(closer.map(|pr| PullRequest { number: pr.number, title: pr.title, body: pr.body }))
}

pub fn get_pr_diff(repo: &str, pr_number: u64) -> Result<String> {
    gh(&["pr", "diff", &pr_number.to_string(), "--repo", repo])
}

pub fn list_open_plans(repo: &str) -> Result<Vec<PlanIssue>> {
    let out = gh(&[
        "issue", "list",
        "--repo", repo,
        "--label", "engram-plan",
        "--state", "open",
        "--json", "number,title,createdAt",
        "--limit", "50",
    ])?;
    serde_json::from_str(&out).context("parsing issue list JSON")
}

pub fn create_pr(repo: &str, title: &str, body: &str, label: &str) -> Result<String> {
    gh(&[
        "pr", "create",
        "--repo", repo,
        "--title", title,
        "--body", body,
        "--label", label,
    ])
}
