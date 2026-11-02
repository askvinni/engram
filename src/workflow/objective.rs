use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::ops::Range;

use crate::integrations::{claude, github, kata};
use crate::workflow::learn;

const NODES_MARKER_START: &str = "<!-- engram:nodes ";
const NODES_MARKER_END: &str = " -->";
const ROADMAP_HEADING: &str = "Roadmap";
/// A node plan's marker is `Objective: #<number> (node <id>)`.
const OBJECTIVE_MARKER_PREFIX: &str = "Objective: #";
const OBJECTIVE_MARKER_NODE: &str = " (node ";
/// Branch prefix of the PR `objective land` commits learnings on.
const LAND_BRANCH_PREFIX: &str = "engram/objective-land-";

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Pending,
    InProgress,
    Done,
}

impl NodeStatus {
    /// The status as the roadmap table shows it, which is also its serialized form.
    fn as_str(self) -> &'static str {
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

impl ObjectiveNode {
    fn pending(id: &str, description: &str, depends_on: Vec<String>) -> Self {
        ObjectiveNode {
            id: id.to_string(),
            description: description.to_string(),
            status: NodeStatus::Pending,
            plan_issue: None,
            pr_url: None,
            depends_on,
        }
    }
}

/// Comma-separated node IDs, blanks dropped.
fn parse_id_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse bullet-list node definitions from a Roadmap section body.
/// Accepts lines like:
///   - 1.1: Description
///   - 1.2: Description (depends: 1.1)
///   - 1.3: Description (depends: 1.1, 1.2)
pub fn parse_nodes_from_roadmap_input(text: &str) -> Vec<ObjectiveNode> {
    const DEPENDS: &str = "(depends:";
    text.lines()
        .filter_map(|line| {
            let (id, desc_and_deps) = line.trim().strip_prefix("- ")?.split_once(": ")?;
            let (description, depends_on) = match desc_and_deps.rfind(DEPENDS) {
                Some(pos) => (
                    &desc_and_deps[..pos],
                    parse_id_list(desc_and_deps[pos + DEPENDS.len()..].trim_end_matches(')')),
                ),
                None => (desc_and_deps, Vec::new()),
            };
            Some(ObjectiveNode::pending(
                id.trim(),
                description.trim(),
                depends_on,
            ))
        })
        .collect()
}

/// Extract node state stored in the hidden `<!-- engram:nodes [...] -->` comment.
pub fn parse_nodes_from_comment(body: &str) -> Option<Vec<ObjectiveNode>> {
    let start = body.find(NODES_MARKER_START)?;
    let after_start = &body[start + NODES_MARKER_START.len()..];
    let end = after_start.find(NODES_MARKER_END)?;
    serde_json::from_str(&after_start[..end]).ok()
}

pub fn write_nodes_to_comment(nodes: &[ObjectiveNode]) -> Result<String> {
    let json = serde_json::to_string(nodes).context("serializing objective nodes")?;
    Ok(format!("{NODES_MARKER_START}{json}{NODES_MARKER_END}"))
}

/// Returns the IDs of deps that block `node` (i.e. not yet Done).
/// Single-pass filter — does not recurse to avoid circular dep issues.
pub fn blocked_by<'a>(node: &'a ObjectiveNode, all_nodes: &'a [ObjectiveNode]) -> Vec<&'a str> {
    node.depends_on
        .iter()
        .filter(|dep_id| {
            all_nodes
                .iter()
                .any(|n| &n.id == *dep_id && n.status != NodeStatus::Done)
        })
        .map(String::as_str)
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
        let pr = node.pr_url.as_deref().unwrap_or_default();
        lines.push(format!(
            "| {} | {} | {} | {plan} | {pr} | {blocked} |",
            node.id,
            node.description,
            node.status.as_str(),
        ));
    }
    lines.join("\n")
}

/// Byte range of the content under `## <heading>`: from the line after the
/// heading up to the next `## ` heading, or the end of `body`.
fn section_range(body: &str, heading: &str) -> Option<Range<usize>> {
    let heading_pos = body.find(&format!("## {heading}"))?;
    let start = body[heading_pos..]
        .find('\n')
        .map_or(body.len(), |i| heading_pos + i + 1);
    let end = body[start..]
        .find("\n## ")
        .map_or(body.len(), |i| start + i + 1);
    Some(start..end)
}

fn extract_section<'a>(body: &'a str, heading: &str) -> Option<&'a str> {
    section_range(body, heading).map(|r| &body[r])
}

/// Replace the content of the `## Roadmap` section in `body` with a fresh
/// rendered table plus the hidden nodes comment. Preserves all other sections.
pub fn build_objective_body(body: &str, nodes: &[ObjectiveNode]) -> Result<String> {
    let table = render_roadmap_table(nodes);
    let comment = write_nodes_to_comment(nodes)?;
    let new_content = format!("\n{table}\n\n{comment}");
    Ok(match section_range(body, ROADMAP_HEADING) {
        Some(range) if range.end == body.len() => format!("{}{new_content}", &body[..range.start]),
        Some(range) => format!(
            "{}{new_content}\n\n{}",
            &body[..range.start],
            &body[range.end..]
        ),
        None => format!("{}\n\n## {ROADMAP_HEADING}{new_content}", body.trim_end()),
    })
}

/// The `Objective: #N (node ID)` line that ties a plan to its objective node.
fn objective_marker(objective_number: u64, node_id: &str) -> String {
    format!("{OBJECTIVE_MARKER_PREFIX}{objective_number}{OBJECTIVE_MARKER_NODE}{node_id})")
}

/// Look for `Objective: #N (node ID)` on any line of `body`.
pub(crate) fn parse_objective_marker(body: &str) -> Option<(u64, String)> {
    body.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(OBJECTIVE_MARKER_PREFIX)?;
        let (num, node) = rest.split_once(OBJECTIVE_MARKER_NODE)?;
        Some((num.parse().ok()?, node.trim_end_matches(')').to_string()))
    })
}

/// The objective issue and its node list; fails for an issue without one.
fn fetch_objective(repo: &str, number: u64) -> Result<(github::Issue, Vec<ObjectiveNode>)> {
    let issue = github::get_issue(repo, number).context("fetching objective issue")?;
    let nodes = parse_nodes_from_comment(issue.body_text()).ok_or_else(|| {
        anyhow::anyhow!(
            "could not parse nodes from objective #{number} — \
             was it created with `engram objective new`?"
        )
    })?;
    Ok((issue, nodes))
}

/// Write `nodes` back into objective `number`'s body.
fn save_nodes(repo: &str, number: u64, body: &str, nodes: &[ObjectiveNode]) -> Result<()> {
    github::update_issue_body(repo, number, &build_objective_body(body, nodes)?)
        .context("updating objective issue body")
}

/// Once every node is done, comment and close objective `number` (warnings only).
fn close_if_all_done(repo: &str, number: u64, nodes: &[ObjectiveNode]) {
    if !all_nodes_done(nodes) {
        return;
    }
    if let Err(e) = github::add_issue_comment(repo, number, &build_close_comment(nodes)) {
        eprintln!("warning: could not post closing comment on #{number}: {e:#}");
    }
    match github::close_issue(repo, number) {
        Ok(()) => println!("Closed objective #{number} — all nodes done."),
        Err(e) => eprintln!("warning: could not close objective #{number}: {e:#}"),
    }
}

pub fn new(repo: &str, title: &str, body: &str, tracker: &kata::TrackerMeta) -> Result<()> {
    let roadmap_text = extract_section(body, ROADMAP_HEADING)
        .ok_or_else(|| anyhow::anyhow!("body must contain a ## {ROADMAP_HEADING} section"))?;

    let nodes = parse_nodes_from_roadmap_input(roadmap_text);
    if nodes.is_empty() {
        anyhow::bail!(
            "## {ROADMAP_HEADING} section contains no nodes — expected lines like `- 1.1: Description`"
        );
    }

    let issue_body = build_objective_body(body, &nodes)?;
    let issue = github::create_issue(repo, title, &issue_body, github::OBJECTIVE_LABEL)?;
    kata::mirror_new_issue(
        repo,
        &issue,
        &kata::MirrorSpec {
            title,
            body: &issue_body,
            idempotency_key: kata::objective_idempotency_key(issue.number),
            parent: None,
            meta: tracker.resolve_for(title, body),
        },
    );

    println!("{}", issue.url);
    Ok(())
}

pub fn list_open(repo: &str) -> Result<()> {
    let objectives = github::list_open_issues(repo, github::OBJECTIVE_LABEL)?;
    if objectives.is_empty() {
        println!("No open objectives.");
        return Ok(());
    }
    for obj in &objectives {
        let age = crate::date::days_ago(&obj.created_at);
        println!("#{:<4} {} ({})", obj.number, obj.title, age);
    }
    Ok(())
}

pub fn view(repo: &str, number: u64) -> Result<()> {
    let issue = github::get_issue(repo, number)?;

    println!("Objective #{}: {}", number, issue.title);
    println!();

    // Print body but omit the hidden nodes comment
    for line in issue.body_text().lines() {
        if !line.starts_with(NODES_MARKER_START) {
            println!("{line}");
        }
    }
    Ok(())
}

/// IDs of the unfinished nodes `node_id` depends on, directly or through
/// other unfinished nodes. A done node's work is already on the default
/// branch, so the walk stops there: it doesn't put its own dependencies on
/// any branch. Unknown IDs are skipped; cycles are safe.
pub fn unfinished_ancestors(
    node_id: &str,
    all_nodes: &[ObjectiveNode],
) -> std::collections::BTreeSet<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec![node_id];
    while let Some(id) = stack.pop() {
        let Some(node) = all_nodes.iter().find(|n| n.id == id) else {
            continue;
        };
        for dep in &node.depends_on {
            let unfinished = all_nodes
                .iter()
                .any(|n| &n.id == dep && n.status != NodeStatus::Done);
            if unfinished && seen.insert(dep.clone()) {
                stack.push(dep);
            }
        }
    }
    seen
}

/// Returns true only when the list is non-empty and every node has status Done.
pub fn all_nodes_done(nodes: &[ObjectiveNode]) -> bool {
    !nodes.is_empty() && nodes.iter().all(|n| n.status == NodeStatus::Done)
}

pub(crate) fn build_close_comment(nodes: &[ObjectiveNode]) -> String {
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

/// The objective a node plan is created for.
#[derive(Debug)]
struct ObjectiveContext<'a> {
    repo: &'a str,
    number: u64,
    title: &'a str,
    body: &'a str,
    tracker: &'a kata::TrackerMeta,
}

/// Create a plan issue for the node at `node_idx`, mutate `nodes` to reflect
/// `InProgress` status, and return the plan issue URL. Does not update the
/// objective issue body — callers are responsible for that.
fn create_plan_for_node(
    obj: &ObjectiveContext,
    nodes: &mut [ObjectiveNode],
    node_idx: usize,
    body: Option<&str>,
) -> Result<String> {
    let node = &nodes[node_idx];
    if let Some(existing) = node.plan_issue {
        anyhow::bail!(
            "node {} already has plan issue #{existing} — cannot create another",
            node.id
        );
    }

    let plan_body_text = match body {
        Some(b) => {
            crate::workflow::plan::warn_missing_sections(b);
            b.to_string()
        }
        None => generate_plan_body_for_node(obj.title, obj.body, &node.id, &node.description)?,
    };
    let plan_body = format!(
        "{}\n\n{plan_body_text}",
        objective_marker(obj.number, &node.id)
    );
    let plan_title = format!("[{}] {}", node.id, node.description);
    let issue = github::create_issue(obj.repo, &plan_title, &plan_body, github::PLAN_LABEL)?;

    nodes[node_idx].status = NodeStatus::InProgress;
    nodes[node_idx].plan_issue = Some(issue.number);

    if let Err(e) = github::add_sub_issue(obj.repo, obj.number, issue.number) {
        eprintln!(
            "warning: could not link #{} as sub-issue: {e:#}",
            issue.number
        );
    }

    if kata::kata_available() {
        let kata_parent = kata::parse_ref(obj.body);
        let meta = node_tracker_meta(
            obj.tracker,
            &plan_title,
            &plan_body,
            obj.title,
            obj.body,
            kata_parent,
        );
        kata::mirror_new_issue(
            obj.repo,
            &issue,
            &kata::MirrorSpec {
                title: &plan_title,
                body: &plan_body,
                idempotency_key: kata::plan_idempotency_key(issue.number),
                parent: kata_parent,
                meta,
            },
        );
    }

    Ok(issue.url)
}

/// A node plan's own Jira key/type (flags, then its body/title) win; anything
/// missing is inherited from the objective's GitHub issue, then its kata mirror.
pub(crate) fn node_tracker_meta(
    tracker: &kata::TrackerMeta,
    plan_title: &str,
    plan_body: &str,
    obj_title: &str,
    obj_body: &str,
    kata_parent: Option<&str>,
) -> kata::TrackerMeta {
    let meta = tracker
        .resolve_for(plan_title, plan_body)
        .or(kata::TrackerMeta::resolve(None, None, obj_title, obj_body));
    match kata_parent {
        Some(parent) if !meta.is_complete() => match kata::tracker_meta_of(parent) {
            Ok(parent_meta) => meta.or(parent_meta),
            Err(e) => {
                eprintln!("warning: could not read kata metadata of {parent}: {e:#}");
                meta
            }
        },
        _ => meta,
    }
}

/// Which nodes `objective plan` creates plan issues for.
#[derive(Debug, Clone, Copy)]
pub enum PlanTarget<'a> {
    /// One node, by ID.
    Node(&'a str),
    /// Every pending node whose dependencies are all done.
    AllUnblocked,
}

pub fn plan(
    repo: &str,
    objective_number: u64,
    target: PlanTarget,
    body: Option<&str>,
    tracker: &kata::TrackerMeta,
) -> Result<()> {
    let (obj_issue, mut nodes) = fetch_objective(repo, objective_number)?;
    let obj = ObjectiveContext {
        repo,
        number: objective_number,
        title: &obj_issue.title,
        body: obj_issue.body_text(),
        tracker,
    };

    match target {
        PlanTarget::AllUnblocked => {
            let unblocked = unblocked_nodes(&nodes);
            if unblocked.is_empty() {
                println!("No unblocked pending nodes in objective #{objective_number}.");
                return Ok(());
            }

            let mut failed = 0usize;
            let mut created = 0usize;
            for idx in unblocked {
                let nid = nodes[idx].id.clone();
                match create_plan_for_node(&obj, &mut nodes, idx, None) {
                    Ok(url) => {
                        println!("{url} (node {nid})");
                        created += 1;
                    }
                    Err(e) => {
                        eprintln!("error: node {nid}: {e:#}");
                        failed += 1;
                    }
                }
            }
            save_nodes(repo, objective_number, obj.body, &nodes)?;
            if created > 0 {
                println!("Created {created} plan issue(s) for objective #{objective_number}.");
            }
            if failed > 0 {
                anyhow::bail!("{failed} node(s) failed — see errors above");
            }
        }
        PlanTarget::Node(nid) => {
            let node_idx = nodes.iter().position(|n| n.id == nid).ok_or_else(|| {
                anyhow::anyhow!("node {nid} not found in objective #{objective_number}")
            })?;
            let plan_url = create_plan_for_node(&obj, &mut nodes, node_idx, body)?;
            save_nodes(repo, objective_number, obj.body, &nodes)?;
            println!("{plan_url}");
            println!("Marked node {nid} as in_progress in objective #{objective_number}.");
        }
    }
    Ok(())
}

/// Check `plan_body` for an `Objective: #N (node ID)` marker and, if found,
/// mark that node as Done in the objective issue, closing the objective once
/// every node is done. A missing node list or node is only a warning; failing
/// to fetch or update the objective is an error.
pub fn maybe_mark_node_done(repo: &str, plan_body: &str) -> Result<()> {
    let Some((obj_number, node_id)) = parse_objective_marker(plan_body) else {
        return Ok(());
    };

    let obj_issue = github::get_issue(repo, obj_number).context("fetching objective issue")?;
    let Some(mut nodes) = parse_nodes_from_comment(obj_issue.body_text()) else {
        eprintln!("warning: could not parse nodes from objective #{obj_number}");
        return Ok(());
    };
    let Some(node) = nodes.iter_mut().find(|n| n.id == node_id) else {
        eprintln!("warning: node {node_id} not found in objective #{obj_number}");
        return Ok(());
    };
    node.status = NodeStatus::Done;

    save_nodes(repo, obj_number, obj_issue.body_text(), &nodes)?;
    println!("Marked node {node_id} as done in objective #{obj_number}.");
    close_if_all_done(repo, obj_number, &nodes);
    Ok(())
}

/// Batch-land all plans in an objective: synthesise learnings from every node
/// that has a linked plan issue and isn't yet done, commit everything in one
/// branch+PR, and close the objective when all nodes are done.
pub fn land(repo_root: &std::path::Path, repo: &str, objective_number: u64) -> Result<()> {
    let (obj_issue, mut nodes) = fetch_objective(repo, objective_number)?;
    let obj_body = obj_issue.body_text();

    let landable: Vec<(usize, u64)> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.status != NodeStatus::Done)
        .filter_map(|(i, n)| Some((i, n.plan_issue?)))
        .collect();

    if landable.is_empty() {
        println!("No plans to land in objective #{objective_number}.");
        if !obj_issue.is_closed() {
            close_if_all_done(repo, objective_number, &nodes);
        }
        return Ok(());
    }

    let mut learned: Vec<u64> = vec![];
    let mut failed = 0usize;

    for (idx, plan_num) in landable {
        let node_id = nodes[idx].id.clone();
        println!("\nLearning from plan #{plan_num} (node {node_id})...");
        match learn::write_memory(repo_root, plan_num, repo) {
            Ok(true) => learned.push(plan_num),
            Ok(false) => {
                learn::mark_learned_or_warn(repo, plan_num, learn::LearnOutcome::NoLearnings);
            }
            Err(e) => {
                eprintln!("  skipping #{plan_num}: {e:#}");
                failed += 1;
                continue;
            }
        }
        nodes[idx].status = NodeStatus::Done;
        if let Err(e) = save_nodes(repo, objective_number, obj_body, &nodes) {
            eprintln!("warning: could not update objective body after node {node_id}: {e:#}");
        }
    }

    if !learned.is_empty() {
        let title = format!("engram: objective #{objective_number} learnings");
        let pr = learn::MemoryPr {
            branch: &format!("{LAND_BRANCH_PREFIX}{objective_number}"),
            commit_message: &format!("engram: learn from {}", learn::issue_list(&learned)),
            title: &title,
            body: &format!(
                "Learnings from objective #{objective_number}: {}.{}",
                learn::issue_list(&learned),
                learn::PR_FOOTER
            ),
        };
        if let Some(pr_url) = learn::commit_learned(repo_root, repo, &learned, &pr)? {
            println!("\nPR created: {pr_url}");
        }
    }

    close_if_all_done(repo, objective_number, &nodes);

    if failed > 0 {
        anyhow::bail!("{failed} plan(s) failed — see errors above");
    }
    Ok(())
}

/// Add a pending node to `nodes`, refusing a duplicate ID or an unknown dependency.
fn append_node(
    nodes: &mut Vec<ObjectiveNode>,
    id: &str,
    description: &str,
    depends: Option<&str>,
) -> Result<()> {
    if nodes.iter().any(|n| n.id == id) {
        anyhow::bail!("node {id} already exists");
    }
    let depends_on = depends.map(parse_id_list).unwrap_or_default();
    if let Some(dep) = depends_on
        .iter()
        .find(|d| !nodes.iter().any(|n| &n.id == *d))
    {
        anyhow::bail!("dependency {dep} not found");
    }
    nodes.push(ObjectiveNode::pending(id, description, depends_on));
    Ok(())
}

pub fn append(
    repo: &str,
    number: u64,
    id: &str,
    description: &str,
    depends: Option<&str>,
) -> Result<()> {
    let (obj_issue, mut nodes) = fetch_objective(repo, number)?;
    append_node(&mut nodes, id, description, depends)
        .with_context(|| format!("appending to objective #{number}"))?;
    save_nodes(repo, number, obj_issue.body_text(), &nodes)?;
    println!("Appended node {id} to objective #{number}.");
    Ok(())
}

fn generate_plan_body_for_node(
    objective_title: &str,
    objective_body: &str,
    node_id: &str,
    node_description: &str,
) -> Result<String> {
    let prompt = format!(
        r"You are helping create an engram plan issue for a specific node in an objective.

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

Return ONLY the plan body text."
    );
    claude::run_prompt(&prompt).context("generating the plan body")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

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
        let comment = write_nodes_to_comment(&nodes).unwrap();
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

    #[rstest]
    // Unknown x and done d are skipped, and so is e behind d.
    #[case::stops_at_done_and_unknown("c", &["a", "b"])]
    #[case::root("a", &[])]
    #[case::cycle("y", &["y", "z"])]
    fn unfinished_ancestors_stops_at_done_nodes_and_survives_cycles(
        #[case] id: &str,
        #[case] want: &[&str],
    ) {
        use NodeStatus::*;
        let nodes = [
            node("a", Pending, &[]),
            node("b", InProgress, &["a"]),
            node("c", Pending, &["b", "x", "d"]),
            node("d", Done, &["e"]),
            node("e", Pending, &[]),
            node("y", Pending, &["z"]),
            node("z", Pending, &["y"]),
        ];
        let got: Vec<String> = unfinished_ancestors(id, &nodes).into_iter().collect();
        assert_eq!(got, want);
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
        let result = build_objective_body(body, &nodes).unwrap();
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
        let body_v1 =
            build_objective_body("## Goal\nX\n\n## Roadmap\n- 1.1: Alpha", &nodes_v1).unwrap();

        // Now simulate updating the node
        let mut nodes_v2 = nodes_v1.clone();
        nodes_v2[0].status = NodeStatus::Done;
        let body_v2 = build_objective_body(&body_v1, &nodes_v2).unwrap();

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
        let result = build_objective_body(body, &nodes).unwrap();
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
            depends_on: deps.iter().map(std::string::ToString::to_string).collect(),
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

    // --- append helpers ---

    fn body_with_nodes(nodes: &[ObjectiveNode]) -> String {
        let comment = write_nodes_to_comment(nodes).unwrap();
        format!("## Goal\nDo stuff\n\n## Roadmap\n{comment}")
    }

    #[test]
    fn append_node_round_trips_through_body() {
        let obj_body = body_with_nodes(&[node("1.1", NodeStatus::Pending, &[])]);
        let mut nodes = parse_nodes_from_comment(&obj_body).unwrap();
        nodes.push(ObjectiveNode {
            id: "1.2".to_string(),
            description: "Second".to_string(),
            status: NodeStatus::Pending,
            plan_issue: None,
            pr_url: None,
            depends_on: vec!["1.1".to_string()],
        });
        let reparsed =
            parse_nodes_from_comment(&build_objective_body(&obj_body, &nodes).unwrap()).unwrap();
        assert_eq!(reparsed.len(), 2);
        assert_eq!(reparsed[1].id, "1.2");
        assert_eq!(reparsed[1].depends_on, vec!["1.1"]);
    }

    #[test]
    fn append_node_adds_a_pending_node() {
        let mut nodes = vec![node("1.1", NodeStatus::Done, &[])];
        append_node(&mut nodes, "1.2", "Second", Some("1.1, ")).unwrap();
        assert_eq!(nodes[1].id, "1.2");
        assert_eq!(nodes[1].status, NodeStatus::Pending);
        assert_eq!(nodes[1].depends_on, vec!["1.1"]);
    }

    #[test]
    fn append_node_rejects_duplicate_id_and_unknown_dep() {
        let mut nodes = vec![node("1.1", NodeStatus::Pending, &[])];
        let dup = append_node(&mut nodes, "1.1", "Again", None).unwrap_err();
        assert!(dup.to_string().contains("already exists"), "{dup}");
        let unknown = append_node(&mut nodes, "1.2", "B", Some("9.9")).unwrap_err();
        assert!(unknown.to_string().contains("dependency 9.9"), "{unknown}");
        assert_eq!(nodes.len(), 1);
    }

    #[test]
    fn status_text_matches_its_serialized_form() {
        for status in [
            NodeStatus::Pending,
            NodeStatus::InProgress,
            NodeStatus::Done,
        ] {
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                serde_json::Value::from(status.as_str())
            );
        }
    }

    #[test]
    fn objective_marker_round_trips() {
        let body = format!("{}\n\n**Why**\nStuff", objective_marker(42, "1.2"));
        assert_eq!(parse_objective_marker(&body), Some((42, "1.2".to_string())));
    }

    #[test]
    fn extract_section_stops_at_the_next_heading() {
        let body = "## Goal\nG\n\n## Roadmap\n- 1.1: A\n\n## Notes\nN";
        assert_eq!(extract_section(body, "Roadmap"), Some("- 1.1: A\n\n"));
        assert_eq!(extract_section(body, "Notes"), Some("N"));
        assert_eq!(extract_section(body, "Missing"), None);
    }
}
