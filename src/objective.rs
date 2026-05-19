use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::github;

const NODES_MARKER_START: &str = "<!-- engram:nodes ";
const NODES_MARKER_END: &str = " -->";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Pending,
    InProgress,
    Done,
}

impl NodeStatus {
    fn display(&self) -> &'static str {
        match self {
            NodeStatus::Pending => "pending",
            NodeStatus::InProgress => "in_progress",
            NodeStatus::Done => "done",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ObjectiveNode {
    pub id: String,
    pub description: String,
    pub status: NodeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_issue: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// Parse bullet-list node definitions from a Roadmap section body.
/// Accepts lines like:
///   - 1.1: Description
///   - 1.2: Description (depends: 1.1)
///   - 1.3: Description (depends: 1.1, 1.2)
pub fn parse_nodes_from_roadmap_input(text: &str) -> Vec<ObjectiveNode> {
    let mut nodes = Vec::new();
    for line in text.lines() {
        let rest = match line.trim().strip_prefix("- ") {
            Some(r) => r,
            None => continue,
        };
        let (id, desc_and_deps) = match rest.split_once(": ") {
            Some(p) => p,
            None => continue,
        };
        let (description, depends_on) = if let Some(deps_pos) = desc_and_deps.rfind("(depends:") {
            let desc = desc_and_deps[..deps_pos].trim().to_string();
            let deps_part = desc_and_deps[deps_pos + "(depends:".len()..]
                .trim_start()
                .trim_end_matches(')');
            let depends_on = deps_part
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            (desc, depends_on)
        } else {
            (desc_and_deps.trim().to_string(), Vec::new())
        };
        nodes.push(ObjectiveNode {
            id: id.trim().to_string(),
            description,
            status: NodeStatus::Pending,
            plan_issue: None,
            pr_url: None,
            depends_on,
        });
    }
    nodes
}

/// Extract node state stored in the hidden `<!-- engram:nodes [...] -->` comment.
pub fn parse_nodes_from_comment(body: &str) -> Option<Vec<ObjectiveNode>> {
    let start = body.find(NODES_MARKER_START)?;
    let after_start = &body[start + NODES_MARKER_START.len()..];
    let end = after_start.find(NODES_MARKER_END)?;
    serde_json::from_str(&after_start[..end]).ok()
}

pub fn write_nodes_to_comment(nodes: &[ObjectiveNode]) -> String {
    let json = serde_json::to_string(nodes).unwrap_or_default();
    format!("{NODES_MARKER_START}{json}{NODES_MARKER_END}")
}

/// Returns the IDs of deps that block `node` (i.e. not yet Done).
/// Single-pass filter — does not recurse to avoid circular dep issues.
pub fn blocked_by<'a>(node: &'a ObjectiveNode, all_nodes: &'a [ObjectiveNode]) -> Vec<&'a str> {
    node.depends_on
        .iter()
        .filter(|dep_id| {
            all_nodes
                .iter()
                .find(|n| &n.id == *dep_id)
                .map(|n| n.status != NodeStatus::Done)
                .unwrap_or(false)
        })
        .map(|s| s.as_str())
        .collect()
}

pub fn render_roadmap_table(nodes: &[ObjectiveNode]) -> String {
    let mut lines = vec![
        "| ID | Description | Status | Plan | PR | Blocked By |".to_string(),
        "|----|-------------|--------|------|----|------------|".to_string(),
    ];
    for node in nodes {
        let blocked = blocked_by(node, nodes).join(", ");
        let plan = node.plan_issue.map(|n| format!("#{n}")).unwrap_or_default();
        let pr = node.pr_url.clone().unwrap_or_default();
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            node.id,
            node.description,
            node.status.display(),
            plan,
            pr,
            blocked,
        ));
    }
    lines.join("\n")
}

/// Replace the content of the `## Roadmap` section in `body` with a fresh
/// rendered table plus the hidden nodes comment. Preserves all other sections.
pub fn build_objective_body(body: &str, nodes: &[ObjectiveNode]) -> String {
    let table = render_roadmap_table(nodes);
    let comment = write_nodes_to_comment(nodes);
    let new_content = format!("\n{table}\n\n{comment}");

    if let Some(heading_pos) = body.find("## Roadmap") {
        let after_heading_pos = body[heading_pos..]
            .find('\n')
            .map(|i| heading_pos + i + 1)
            .unwrap_or(body.len());
        let before = &body[..after_heading_pos];
        let rest = &body[after_heading_pos..];
        // Find start of next ## section (the '\n' before '## ' marks the boundary)
        let next_section_pos = rest.find("\n## ").map(|i| i + 1).unwrap_or(rest.len());
        let after_section = &rest[next_section_pos..];
        if after_section.is_empty() {
            format!("{before}{new_content}")
        } else {
            format!("{before}{new_content}\n\n{after_section}")
        }
    } else {
        format!("{}\n\n## Roadmap{}", body.trim_end(), new_content)
    }
}

/// Look for `Objective: #N (node ID)` on any line of `body`.
fn parse_objective_marker(body: &str) -> Option<(u64, String)> {
    for line in body.lines() {
        if let Some(rest) = line.trim().strip_prefix("Objective: #") {
            if let Some((num_str, rest2)) = rest.split_once(" (node ") {
                if let Ok(num) = num_str.parse::<u64>() {
                    let node_id = rest2.trim_end_matches(')').to_string();
                    return Some((num, node_id));
                }
            }
        }
    }
    None
}

fn extract_section<'a>(body: &'a str, heading: &str) -> Option<&'a str> {
    let needle = format!("## {heading}");
    let pos = body.find(needle.as_str())?;
    let after_heading = &body[pos + needle.len()..];
    let content_start = after_heading.find('\n').map(|i| i + 1).unwrap_or(0);
    let content = &after_heading[content_start..];
    let section_end = content
        .find("\n## ")
        .map(|i| i + 1)
        .unwrap_or(content.len());
    Some(&content[..section_end])
}

pub fn new(repo: &str, title: &str, body: &str) -> Result<()> {
    let roadmap_text = extract_section(body, "Roadmap")
        .ok_or_else(|| anyhow::anyhow!("body must contain a ## Roadmap section"))?;

    let nodes = parse_nodes_from_roadmap_input(roadmap_text);
    if nodes.is_empty() {
        anyhow::bail!(
            "## Roadmap section contains no nodes — expected lines like `- 1.1: Description`"
        );
    }

    let issue_body = build_objective_body(body, &nodes);
    let url = github::create_issue(repo, title, &issue_body, "engram-objective")?;
    println!("{}", url.trim());
    Ok(())
}

pub fn list_open(repo: &str) -> Result<()> {
    let objectives = github::list_open_objectives(repo)?;
    if objectives.is_empty() {
        println!("No open objectives.");
        return Ok(());
    }
    for obj in &objectives {
        let age = crate::days_ago(&obj.created_at);
        println!("#{:<4} {} ({})", obj.number, obj.title, age);
    }
    Ok(())
}

pub fn view(repo: &str, number: u64) -> Result<()> {
    let issue = github::get_issue(repo, number)?;
    let body = issue.body.as_deref().unwrap_or("");

    println!("Objective #{}: {}", number, issue.title);
    println!();

    // Print body but omit the hidden nodes comment
    for line in body.lines() {
        if line.starts_with(NODES_MARKER_START) {
            continue;
        }
        println!("{line}");
    }
    Ok(())
}

/// Returns true only when the list is non-empty and every node has status Done.
pub fn all_nodes_done(nodes: &[ObjectiveNode]) -> bool {
    !nodes.is_empty() && nodes.iter().all(|n| n.status == NodeStatus::Done)
}

fn build_close_comment(nodes: &[ObjectiveNode]) -> String {
    let mut lines = vec![
        "All nodes completed — closing objective.".to_string(),
        String::new(),
    ];
    for node in nodes {
        lines.push(format!("- {} {}", node.id, node.description));
    }
    lines.join("\n")
}

/// Returns indices of nodes that are Pending with all dependencies Done.
/// Single-pass — safe against circular deps.
pub fn unblocked_nodes(nodes: &[ObjectiveNode]) -> Vec<usize> {
    nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.status == NodeStatus::Pending && blocked_by(n, nodes).is_empty())
        .map(|(i, _)| i)
        .collect()
}

/// Create a plan issue for the node at `node_idx`, mutate `nodes` to reflect
/// InProgress status, and return the plan issue URL. Does not update the
/// objective issue body — callers are responsible for that.
fn create_plan_for_node(
    repo: &str,
    objective_number: u64,
    obj_title: &str,
    obj_body: &str,
    nodes: &mut [ObjectiveNode],
    node_idx: usize,
    body: Option<&str>,
) -> Result<String> {
    let node_id = nodes[node_idx].id.clone();
    let node_description = nodes[node_idx].description.clone();

    if let Some(existing) = nodes[node_idx].plan_issue {
        anyhow::bail!("node {node_id} already has plan issue #{existing} — cannot create another");
    }

    let marker = format!("Objective: #{objective_number} (node {node_id})");
    let plan_body = match body {
        Some(b) => format!("{marker}\n\n{b}"),
        None => {
            let generated =
                generate_plan_body_for_node(obj_title, obj_body, &node_id, &node_description)?;
            format!("{marker}\n\n{generated}")
        }
    };

    let plan_title = format!("[{node_id}] {node_description}");
    let plan_url = github::create_issue(repo, &plan_title, &plan_body, "engram-plan")?;
    let plan_url = plan_url.trim().to_string();

    let plan_issue_number: u64 = plan_url
        .rsplit('/')
        .next()
        .and_then(|s| s.parse().ok())
        .context("parsing plan issue number from URL")?;

    nodes[node_idx].status = NodeStatus::InProgress;
    nodes[node_idx].plan_issue = Some(plan_issue_number);

    if let Err(e) = github::add_sub_issue(repo, objective_number, plan_issue_number) {
        eprintln!("warning: could not link #{plan_issue_number} as sub-issue: {e:#}");
    }

    Ok(plan_url)
}

pub fn plan(
    repo: &str,
    objective_number: u64,
    node_id: Option<&str>,
    all_unblocked: bool,
    body: Option<&str>,
) -> Result<()> {
    let obj_issue =
        github::get_issue(repo, objective_number).context("fetching objective issue")?;
    let obj_body = obj_issue.body.as_deref().unwrap_or("").to_string();

    let mut nodes = parse_nodes_from_comment(&obj_body).ok_or_else(|| {
        anyhow::anyhow!(
            "could not parse nodes from objective #{objective_number} — \
             was it created with `engram objective new`?"
        )
    })?;

    if all_unblocked {
        let unblocked = unblocked_nodes(&nodes);
        if unblocked.is_empty() {
            println!("No unblocked pending nodes in objective #{objective_number}.");
            return Ok(());
        }

        let mut any_failure = false;
        let mut created_count = 0usize;
        for &idx in &unblocked {
            let nid = nodes[idx].id.clone();
            match create_plan_for_node(
                repo,
                objective_number,
                &obj_issue.title,
                &obj_body,
                &mut nodes,
                idx,
                None,
            ) {
                Ok(url) => {
                    println!("{url} (node {nid})");
                    created_count += 1;
                }
                Err(e) => {
                    eprintln!("error: node {nid}: {e:#}");
                    any_failure = true;
                }
            }
        }

        let new_body = build_objective_body(&obj_body, &nodes);
        github::update_issue_body(repo, objective_number, &new_body)
            .context("updating objective issue body")?;

        if created_count > 0 {
            println!("Created {created_count} plan issue(s) for objective #{objective_number}.");
        }

        if any_failure {
            anyhow::bail!("one or more nodes failed — see errors above");
        }
    } else {
        let nid = node_id.expect("node_id is Some when all_unblocked is false");

        let node_idx = nodes.iter().position(|n| n.id == nid).ok_or_else(|| {
            anyhow::anyhow!("node {nid} not found in objective #{objective_number}")
        })?;

        let plan_url = create_plan_for_node(
            repo,
            objective_number,
            &obj_issue.title,
            &obj_body,
            &mut nodes,
            node_idx,
            body,
        )?;

        let new_body = build_objective_body(&obj_body, &nodes);
        github::update_issue_body(repo, objective_number, &new_body)
            .context("updating objective issue body")?;

        println!("{plan_url}");
        println!("Marked node {nid} as in_progress in objective #{objective_number}.");
    }

    Ok(())
}

/// Check `plan_body` for an `Objective: #N (node ID)` marker and, if found,
/// mark that node as Done in the objective issue. Non-fatal — call site should
/// log warnings rather than propagating errors.
pub fn maybe_mark_node_done(repo: &str, plan_body: &str) -> Result<()> {
    let (obj_number, node_id) = match parse_objective_marker(plan_body) {
        Some(v) => v,
        None => return Ok(()),
    };

    let obj_issue = github::get_issue(repo, obj_number).context("fetching objective issue")?;
    let obj_body = obj_issue.body.as_deref().unwrap_or("");

    let mut nodes = match parse_nodes_from_comment(obj_body) {
        Some(n) => n,
        None => {
            eprintln!("warning: could not parse nodes from objective #{obj_number}");
            return Ok(());
        }
    };

    let node = match nodes.iter_mut().find(|n| n.id == node_id) {
        Some(n) => n,
        None => {
            eprintln!("warning: node {node_id} not found in objective #{obj_number}");
            return Ok(());
        }
    };

    node.status = NodeStatus::Done;

    let new_body = build_objective_body(obj_body, &nodes);
    github::update_issue_body(repo, obj_number, &new_body)?;
    println!("Marked node {node_id} as done in objective #{obj_number}.");

    if all_nodes_done(&nodes) {
        let comment = build_close_comment(&nodes);
        if let Err(e) = github::add_issue_comment(repo, obj_number, &comment) {
            eprintln!("warning: could not post closing comment on #{obj_number}: {e:#}");
        }
        match github::close_issue(repo, obj_number) {
            Ok(()) => println!("Closed objective #{obj_number} — all nodes done."),
            Err(e) => eprintln!("warning: could not close objective #{obj_number}: {e:#}"),
        }
    }

    Ok(())
}

/// Batch-land all plans in an objective: synthesise learnings from every node
/// that has a linked plan issue and isn't yet done, commit everything in one
/// branch+PR, and close the objective when all nodes are done.
pub fn land(repo_root: &std::path::Path, repo: &str, objective_number: u64) -> Result<()> {
    let obj_issue =
        github::get_issue(repo, objective_number).context("fetching objective issue")?;
    let obj_body = obj_issue.body.as_deref().unwrap_or("").to_string();

    let mut nodes = parse_nodes_from_comment(&obj_body).ok_or_else(|| {
        anyhow::anyhow!(
            "could not parse nodes from objective #{objective_number} — \
             was it created with `engram objective new`?"
        )
    })?;

    let landable: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.plan_issue.is_some() && n.status != NodeStatus::Done)
        .map(|(i, _)| i)
        .collect();

    if landable.is_empty() {
        println!("No plans to land in objective #{objective_number}.");
        if all_nodes_done(&nodes) && obj_issue.state != "CLOSED" {
            let comment = build_close_comment(&nodes);
            if let Err(e) = github::add_issue_comment(repo, objective_number, &comment) {
                eprintln!("warning: could not post closing comment: {e:#}");
            }
            match github::close_issue(repo, objective_number) {
                Ok(()) => println!("Closed objective #{objective_number} — all nodes done."),
                Err(e) => eprintln!("warning: could not close objective: {e:#}"),
            }
        }
        return Ok(());
    }

    let mut learned: Vec<u64> = vec![];
    let mut failed = 0usize;

    for &idx in &landable {
        let plan_num = nodes[idx].plan_issue.unwrap();
        let node_id = nodes[idx].id.clone();
        println!("\nLearning from plan #{plan_num} (node {node_id})...");
        let wrote = match crate::learn::write_memory(repo_root, plan_num, repo) {
            Ok(wrote) => wrote,
            Err(e) => {
                eprintln!("  skipping #{plan_num}: {e:#}");
                failed += 1;
                continue;
            }
        };
        if wrote {
            learned.push(plan_num);
        }
        nodes[idx].status = NodeStatus::Done;
        let updated = build_objective_body(&obj_body, &nodes);
        if let Err(e) = github::update_issue_body(repo, objective_number, &updated) {
            eprintln!("warning: could not update objective body after node {node_id}: {e:#}");
        }
    }

    // Commit all memory changes in one branch + PR
    if !learned.is_empty() {
        let branch = format!("engram/objective-land-{objective_number}");
        let ok = Command::new("git")
            .args(["checkout", "-b", &branch])
            .current_dir(repo_root)
            .status()?
            .success();
        if !ok {
            anyhow::bail!("git checkout -b {branch} failed");
        }
        Command::new("git")
            .args(["add", ".engram/memory", "CLAUDE.md"])
            .current_dir(repo_root)
            .status()?;
        let nothing_staged = Command::new("git")
            .args(["diff", "--cached", "--quiet"])
            .current_dir(repo_root)
            .status()?
            .success();
        if !nothing_staged {
            let issue_list = learned
                .iter()
                .map(|n| format!("#{n}"))
                .collect::<Vec<_>>()
                .join(", ");
            Command::new("git")
                .args(["commit", "-m", &format!("engram: learn from {issue_list}")])
                .current_dir(repo_root)
                .status()?;
            Command::new("git")
                .args(["push", "-u", "origin", &branch])
                .current_dir(repo_root)
                .status()?;
            let pr_body = format!(
                "Learnings from objective #{objective_number}: {issue_list}.\n\n---\n*Created by engram*"
            );
            let pr_url = github::create_pr(
                repo,
                &format!("engram: objective #{objective_number} learnings"),
                &pr_body,
                "engram-learned",
            )?;
            println!("\nPR created: {}", pr_url.trim());
            for n in &learned {
                if let Err(e) = github::add_label_to_issue(repo, *n, "engram-learned") {
                    eprintln!("warning: could not label #{n}: {e:#}");
                }
            }
        }
    }

    // Close objective if all nodes are now done
    if all_nodes_done(&nodes) {
        let comment = build_close_comment(&nodes);
        if let Err(e) = github::add_issue_comment(repo, objective_number, &comment) {
            eprintln!("warning: could not post closing comment: {e:#}");
        }
        match github::close_issue(repo, objective_number) {
            Ok(()) => println!("Closed objective #{objective_number} — all nodes done."),
            Err(e) => eprintln!("warning: could not close objective: {e:#}"),
        }
    }

    if failed > 0 {
        anyhow::bail!("{failed} plan(s) failed — see errors above");
    }
    Ok(())
}

fn generate_plan_body_for_node(
    objective_title: &str,
    objective_body: &str,
    node_id: &str,
    node_description: &str,
) -> Result<String> {
    let prompt = format!(
        r#"You are helping create an engram plan issue for a specific node in an objective.

## Objective: {objective_title}

{objective_body}

## Node to plan
ID: {node_id}
Description: {node_description}

Generate a plan body for an engram-plan GitHub issue for this node. Include all seven sections:

**Why**: What is broken or missing today (one falsifiable sentence)
**Background**: Context for a cold reader (2-4 sentences)
**Approach**: Technical strategy (3-6 bullet points)
**Acceptance criteria**: Checkable outcome checklist
**Scope**: Which files change; which don't
**Edge cases and risks**: What can go wrong
**Key files**: 3-6 src/foo.rs:FunctionName entry points

Return ONLY the plan body text."#
    );

    let output = Command::new("claude")
        .args(["-p", &prompt, "--output-format", "text"])
        .current_dir(std::env::temp_dir())
        .output()
        .context("running claude CLI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude -p failed: {}", stderr.trim());
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nodes_basic() {
        let input = "- 1.1: First node\n- 1.2: Second node";
        let nodes = parse_nodes_from_roadmap_input(input);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].id, "1.1");
        assert_eq!(nodes[0].description, "First node");
        assert!(nodes[0].depends_on.is_empty());
        assert_eq!(nodes[1].id, "1.2");
    }

    #[test]
    fn parse_nodes_with_deps() {
        let input = "- 1.1: Alpha\n- 1.2: Beta (depends: 1.1)\n- 1.3: Gamma (depends: 1.1, 1.2)";
        let nodes = parse_nodes_from_roadmap_input(input);
        assert_eq!(nodes.len(), 3);
        assert!(nodes[0].depends_on.is_empty());
        assert_eq!(nodes[1].depends_on, vec!["1.1"]);
        assert_eq!(nodes[2].depends_on, vec!["1.1", "1.2"]);
    }

    #[test]
    fn parse_nodes_skips_non_bullet_lines() {
        let input = "Some intro text\n- 1.1: Node\nMore text\n- 1.2: Other";
        let nodes = parse_nodes_from_roadmap_input(input);
        assert_eq!(nodes.len(), 2);
    }

    #[test]
    fn parse_nodes_empty_input() {
        assert!(parse_nodes_from_roadmap_input("").is_empty());
    }

    #[test]
    fn nodes_comment_round_trip() {
        let nodes = vec![ObjectiveNode {
            id: "1.1".to_string(),
            description: "Test".to_string(),
            status: NodeStatus::InProgress,
            plan_issue: Some(42),
            pr_url: None,
            depends_on: vec!["1.0".to_string()],
        }];
        let comment = write_nodes_to_comment(&nodes);
        let parsed = parse_nodes_from_comment(&comment).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "1.1");
        assert_eq!(parsed[0].plan_issue, Some(42));
    }

    #[test]
    fn parse_nodes_from_comment_missing_returns_none() {
        assert!(parse_nodes_from_comment("no comment here").is_none());
    }

    #[test]
    fn blocked_by_pending_dep() {
        let nodes = vec![
            ObjectiveNode {
                id: "1.1".to_string(),
                description: "A".to_string(),
                status: NodeStatus::Pending,
                plan_issue: None,
                pr_url: None,
                depends_on: vec![],
            },
            ObjectiveNode {
                id: "1.2".to_string(),
                description: "B".to_string(),
                status: NodeStatus::Pending,
                plan_issue: None,
                pr_url: None,
                depends_on: vec!["1.1".to_string()],
            },
        ];
        let blocked = blocked_by(&nodes[1], &nodes);
        assert_eq!(blocked, vec!["1.1"]);
    }

    #[test]
    fn blocked_by_done_dep_is_clear() {
        let nodes = vec![
            ObjectiveNode {
                id: "1.1".to_string(),
                description: "A".to_string(),
                status: NodeStatus::Done,
                plan_issue: None,
                pr_url: None,
                depends_on: vec![],
            },
            ObjectiveNode {
                id: "1.2".to_string(),
                description: "B".to_string(),
                status: NodeStatus::Pending,
                plan_issue: None,
                pr_url: None,
                depends_on: vec!["1.1".to_string()],
            },
        ];
        assert!(blocked_by(&nodes[1], &nodes).is_empty());
    }

    #[test]
    fn render_roadmap_table_columns() {
        let nodes = vec![ObjectiveNode {
            id: "1.1".to_string(),
            description: "Alpha".to_string(),
            status: NodeStatus::Pending,
            plan_issue: None,
            pr_url: None,
            depends_on: vec![],
        }];
        let table = render_roadmap_table(&nodes);
        assert!(table.contains("| ID |"));
        assert!(table.contains("| 1.1 |"));
        assert!(table.contains("pending"));
    }

    #[test]
    fn build_objective_body_initial_replaces_bullets() {
        let nodes = vec![ObjectiveNode {
            id: "1.1".to_string(),
            description: "Alpha".to_string(),
            status: NodeStatus::Pending,
            plan_issue: None,
            pr_url: None,
            depends_on: vec![],
        }];
        let body = "## Goal\nDo stuff\n\n## Roadmap\n- 1.1: Alpha";
        let result = build_objective_body(body, &nodes);
        assert!(result.contains("## Goal"));
        assert!(result.contains("## Roadmap"));
        assert!(result.contains("| 1.1 |"));
        assert!(result.contains(NODES_MARKER_START));
        assert!(!result.contains("- 1.1: Alpha"));
    }

    #[test]
    fn build_objective_body_update_replaces_old_comment() {
        let nodes_v1 = vec![ObjectiveNode {
            id: "1.1".to_string(),
            description: "Alpha".to_string(),
            status: NodeStatus::Pending,
            plan_issue: None,
            pr_url: None,
            depends_on: vec![],
        }];
        let body_v1 = build_objective_body("## Goal\nX\n\n## Roadmap\n- 1.1: Alpha", &nodes_v1);

        // Now simulate updating the node
        let mut nodes_v2 = nodes_v1.clone();
        nodes_v2[0].status = NodeStatus::Done;
        let body_v2 = build_objective_body(&body_v1, &nodes_v2);

        assert!(body_v2.contains("done"));
        assert!(!body_v2.contains("pending"));
        // Only one nodes comment in the result
        assert_eq!(body_v2.matches(NODES_MARKER_START).count(), 1);
    }

    #[test]
    fn build_objective_body_preserves_section_after_roadmap() {
        let nodes = vec![ObjectiveNode {
            id: "1.1".to_string(),
            description: "A".to_string(),
            status: NodeStatus::Pending,
            plan_issue: None,
            pr_url: None,
            depends_on: vec![],
        }];
        let body = "## Goal\nFoo\n\n## Roadmap\n- 1.1: A\n\n## Notes\nExtra";
        let result = build_objective_body(body, &nodes);
        assert!(result.contains("## Notes\nExtra"));
    }

    #[test]
    fn parse_objective_marker_found() {
        let body = "Objective: #42 (node 1.2)\n\n**Why**\nStuff";
        let (num, id) = parse_objective_marker(body).unwrap();
        assert_eq!(num, 42);
        assert_eq!(id, "1.2");
    }

    #[test]
    fn parse_objective_marker_absent() {
        assert!(parse_objective_marker("**Why**\nStuff").is_none());
    }

    fn node(id: &str, status: NodeStatus, deps: &[&str]) -> ObjectiveNode {
        ObjectiveNode {
            id: id.to_string(),
            description: id.to_string(),
            status,
            plan_issue: None,
            pr_url: None,
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn unblocked_nodes_no_deps() {
        let nodes = vec![
            node("1.1", NodeStatus::Pending, &[]),
            node("1.2", NodeStatus::Pending, &[]),
        ];
        let unblocked = unblocked_nodes(&nodes);
        assert_eq!(unblocked, vec![0, 1]);
    }

    #[test]
    fn unblocked_nodes_skips_non_pending() {
        let nodes = vec![
            node("1.1", NodeStatus::Done, &[]),
            node("1.2", NodeStatus::InProgress, &[]),
            node("1.3", NodeStatus::Pending, &[]),
        ];
        let unblocked = unblocked_nodes(&nodes);
        assert_eq!(unblocked, vec![2]);
    }

    #[test]
    fn unblocked_nodes_blocked_by_pending_dep() {
        let nodes = vec![
            node("1.1", NodeStatus::Pending, &[]),
            node("1.2", NodeStatus::Pending, &["1.1"]),
        ];
        let unblocked = unblocked_nodes(&nodes);
        assert_eq!(unblocked, vec![0]);
    }

    #[test]
    fn unblocked_nodes_unblocked_when_dep_done() {
        let nodes = vec![
            node("1.1", NodeStatus::Done, &[]),
            node("1.2", NodeStatus::Pending, &["1.1"]),
        ];
        let unblocked = unblocked_nodes(&nodes);
        assert_eq!(unblocked, vec![1]);
    }

    #[test]
    fn unblocked_nodes_empty_when_all_blocked() {
        let nodes = vec![
            node("1.1", NodeStatus::Pending, &[]),
            node("1.2", NodeStatus::Pending, &["1.1"]),
            node("1.3", NodeStatus::Pending, &["1.2"]),
        ];
        // Only 1.1 is unblocked
        let unblocked = unblocked_nodes(&nodes);
        assert_eq!(unblocked, vec![0]);
    }

    #[test]
    fn unblocked_nodes_empty_input() {
        assert!(unblocked_nodes(&[]).is_empty());
    }

    #[test]
    fn all_nodes_done_empty_list() {
        assert!(!all_nodes_done(&[]));
    }

    #[test]
    fn all_nodes_done_all_done() {
        let nodes = vec![
            node("1.1", NodeStatus::Done, &[]),
            node("1.2", NodeStatus::Done, &[]),
        ];
        assert!(all_nodes_done(&nodes));
    }

    #[test]
    fn all_nodes_done_one_pending() {
        let nodes = vec![
            node("1.1", NodeStatus::Done, &[]),
            node("1.2", NodeStatus::Pending, &[]),
        ];
        assert!(!all_nodes_done(&nodes));
    }

    #[test]
    fn all_nodes_done_one_in_progress() {
        let nodes = vec![
            node("1.1", NodeStatus::Done, &[]),
            node("1.2", NodeStatus::InProgress, &[]),
        ];
        assert!(!all_nodes_done(&nodes));
    }

    #[test]
    fn all_nodes_done_single_done() {
        let nodes = vec![node("1.1", NodeStatus::Done, &[])];
        assert!(all_nodes_done(&nodes));
    }
}
