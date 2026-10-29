//! `--stack`: one objective's dependency chains as stacked PRs, each node
//! based on its dependency's branch and linear chains linked with `gh stack`.

use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

use crate::integrations::github;
use crate::workflow::objective::{self, NodeStatus, ObjectiveNode};

use super::{
    ensure_none_failed, format_plan, format_ready_base, plans_failed, queue_node_progress,
    DispatchPlan, NodeProgress, Outcome, PlanCounts, Run, Tally, READY_NO_RELAUNCH,
};

/// What a stack node's branch starts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Base {
    /// The default branch.
    Main,
    /// The branch of the node with this ID.
    OnNode(String),
}

/// An objective node in stack order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StackedNode {
    pub(super) node_id: String,
    pub(super) base: Base,
}

/// An objective's unfinished nodes in stack order, and the ones left out.
#[derive(Debug, PartialEq)]
pub(super) struct StackOrder {
    pub(super) stacked: Vec<StackedNode>,
    /// (node ID, why it is skipped)
    pub(super) skipped: Vec<(String, String)>,
}

/// A stack node's branch and PR URL (while planning, the branch stands in
/// for a PR not opened yet): what dependents start from and what is linked.
#[derive(Debug, Clone)]
pub(super) struct PlacedNode {
    pub(super) branch: String,
    pub(super) pr: String,
}

impl PlacedNode {
    pub(super) fn of(plan: &DispatchPlan) -> Self {
        let pr = plan.pr.as_ref().map_or(&plan.branch, |pr| &pr.url);
        PlacedNode {
            branch: plan.branch.clone(),
            pr: pr.clone(),
        }
    }
}

/// Order an objective's unfinished nodes for a stack: dependencies first, ties
/// in objective order. No unfinished dependency: based on the default branch;
/// one: on its branch; several: on the one that transitively depends on all
/// the others, else skipped as a diamond (and so are its dependents). Returns
/// the nodes in order and the skipped ones; fails on a cycle.
pub(super) fn order_for_stack(nodes: &[ObjectiveNode]) -> Result<StackOrder> {
    let unfinished: Vec<&ObjectiveNode> = nodes
        .iter()
        .filter(|n| n.status != NodeStatus::Done)
        .collect();
    // Each node's unfinished dependencies, deduplicated, in declaration order.
    let deps: BTreeMap<&str, Vec<&str>> = unfinished
        .iter()
        .map(|n| {
            let mut seen = BTreeSet::new();
            let mut d = objective::blocked_by(n, nodes);
            d.retain(|id| seen.insert(*id));
            (n.id.as_str(), d)
        })
        .collect();
    let deps_of = |n: &ObjectiveNode| deps[n.id.as_str()].as_slice();
    let mut ordered: Vec<&ObjectiveNode> = Vec::new();
    let mut placed: BTreeSet<&str> = BTreeSet::new();
    while let Some(next) = unfinished
        .iter()
        .find(|n| !placed.contains(n.id.as_str()) && deps_of(n).iter().all(|d| placed.contains(d)))
    {
        placed.insert(&next.id);
        ordered.push(next);
    }
    if ordered.len() < unfinished.len() {
        let stuck: Vec<&str> = unfinished
            .iter()
            .map(|n| n.id.as_str())
            .filter(|id| !placed.contains(id))
            .collect();
        anyhow::bail!(
            "dependency cycle among objective nodes {}; fix their depends_on",
            stuck.join(", ")
        );
    }

    let mut stacked = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    for n in ordered {
        let deps = deps_of(n);
        if let Some(dep) = deps.iter().find(|d| skipped.iter().any(|(id, _)| id == *d)) {
            skipped.push((n.id.clone(), format!("depends on skipped node {dep}")));
            continue;
        }
        let covering = deps.iter().find(|d| {
            let above = objective::unfinished_ancestors(d, nodes);
            deps.iter().all(|o| o == *d || above.contains(*o))
        });
        let base = match (deps, covering) {
            ([], _) => Base::Main,
            (_, Some(d)) => Base::OnNode(d.to_string()),
            (_, None) => {
                skipped.push((
                    n.id.clone(),
                    format!(
                        "diamond: waits on {} and none of them depends on the others; \
                         dispatch it once all but one are done",
                        deps.join(", ")
                    ),
                ));
                continue;
            }
        };
        stacked.push(StackedNode {
            node_id: n.id.clone(),
            base,
        });
    }
    Ok(StackOrder { stacked, skipped })
}

/// Split stacked nodes into maximal linear chains, bottom to top, each with
/// the base its bottom sits on. A chain starts on the default branch or below
/// a node with several stacked children, and follows single children.
pub(super) fn straight_chains(nodes: &[StackedNode]) -> Vec<(Base, Vec<String>)> {
    let children = |id: &str| -> Vec<&str> {
        nodes
            .iter()
            .filter(|t| t.base == Base::OnNode(id.to_string()))
            .map(|t| t.node_id.as_str())
            .collect()
    };
    nodes
        .iter()
        .filter(|t| match &t.base {
            Base::Main => true,
            Base::OnNode(parent) => children(parent).len() != 1,
        })
        .map(|t| {
            let mut chain = vec![t.node_id.clone()];
            while let [only] = children(&chain[chain.len() - 1])[..] {
                chain.push(only.to_string());
            }
            (t.base.clone(), chain)
        })
        .collect()
}

/// The chains worth `gh stack link`: two or more PRs, at least one of them
/// newly dispatched. Chains made only of reused PRs are left as they are.
pub(super) fn chains_to_link(
    nodes: &[StackedNode],
    fresh: &BTreeSet<String>,
) -> Vec<(Base, Vec<String>)> {
    let mut chains = straight_chains(nodes);
    chains.retain(|(_, chain)| chain.len() >= 2 && chain.iter().any(|id| fresh.contains(id)));
    chains
}

/// The branch a stack node (or chain) based on `base` starts from: `main`,
/// or its dependency's branch once that is placed (`None` until then).
pub(super) fn base_branch(
    base: &Base,
    main: &str,
    placed: &BTreeMap<String, PlacedNode>,
) -> Option<String> {
    match base {
        Base::Main => Some(main.to_string()),
        Base::OnNode(id) => placed.get(id).map(|p| p.branch.clone()),
    }
}

impl Run<'_> {
    /// `dispatch` for `--stack`: the same steps, with each node based on its
    /// dependency's branch and linear chains linked with `gh stack link`.
    pub(super) fn dispatch_stack(&self, issues: &[u64]) -> Result<()> {
        let nodes = self.collect_stack_work(issues)?;
        let main = self.default_branch()?.to_string();
        let (planned, plan_failed) = self.plan_stack(nodes, &main);
        if planned.is_empty() && plan_failed > 0 {
            anyhow::bail!("nothing to dispatch \u{2014} see errors above");
        }
        // Done or only reused as a base: nothing to confirm or link.
        if planned.iter().all(|(_, t)| t.ready()) {
            println!("Nothing to dispatch.");
            anyhow::ensure!(plan_failed == 0, plans_failed(plan_failed));
            return Ok(());
        }
        let mut chains = self.chains_to_link(&planned, &main);
        if !chains.is_empty() && !self.gh_stack_available() {
            chains.clear();
        }
        let counts = PlanCounts::of(planned.iter().map(|(_, t)| t), plan_failed);
        println!("{}", counts.line(self.opts.no_launch));
        if !self.confirmed(plan_failed)? {
            return Ok(());
        }
        ensure_none_failed(plan_failed + self.run_stack(planned, &chains, &main))
    }

    /// The objective's unfinished nodes in stack order, with their plan
    /// issues. Diamonds, nodes without a plan issue and (when planned) their
    /// dependents are listed and skipped.
    pub(super) fn collect_stack_work(&self, issues: &[u64]) -> Result<Vec<(StackedNode, u64)>> {
        let &[number] = issues else {
            anyhow::bail!(
                "--stack takes exactly one engram-objective issue, got {}",
                issues.len()
            );
        };
        let (issue, nodes) = self.fetch_argument_issue(number)?;
        let Some(nodes) = nodes else {
            anyhow::bail!(
                "--stack needs an engram-objective issue; #{number} is a plan \u{2014} \
                 dispatch it without --stack"
            );
        };
        let StackOrder { stacked, skipped } = order_for_stack(&nodes)?;
        println!(
            "Objective #{number}: stacked dispatch onto {}",
            self.default_branch()?
        );
        for (id, why) in &skipped {
            println!("  skip node {id}: {why}");
        }
        self.remember_objective(number, issue);
        let mut out = Vec::new();
        for s in stacked {
            let id = &s.node_id;
            match nodes
                .iter()
                .find(|n| &n.id == id)
                .and_then(|n| n.plan_issue)
            {
                Some(plan) => out.push((s, plan)),
                None => println!(
                    "  skip node {id}: no plan issue yet \u{2014} run \
                     `engram objective plan {number} --node {id}`"
                ),
            }
        }
        Ok(out)
    }

    /// Plan each stack node on its dependency's branch, changing nothing. A
    /// node whose dependency wasn't planned is skipped, since its base branch
    /// won't exist. Returns the plans and how many failed.
    pub(super) fn plan_stack(
        &self,
        nodes: Vec<(StackedNode, u64)>,
        main: &str,
    ) -> (Vec<(StackedNode, DispatchPlan)>, usize) {
        let mut planned = Vec::new();
        let mut failed = 0;
        let mut placed: BTreeMap<String, PlacedNode> = BTreeMap::new();
        for (s, number) in nodes {
            let id = &s.node_id;
            let Some(base) = base_branch(&s.base, main, &placed) else {
                println!("  skip node {id}: its dependency was not planned");
                continue;
            };
            let mut t = match self.plan_dispatch(number, true) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("error: node {id} (#{number}): {e:#}");
                    failed += 1;
                    continue;
                }
            };
            t.set_stack_base(base, main);
            let text = if t.ready() {
                format_ready_base(&t)
            } else {
                format_plan(&t, self.opts.no_launch, &self.claude())
            };
            print!("Node {id}: {text}");
            placed.insert(id.clone(), PlacedNode::of(&t));
            planned.push((s, t));
        }
        (planned, failed)
    }

    /// The chains the run will `gh stack link`, each printed as the command
    /// it will try.
    pub(super) fn chains_to_link(
        &self,
        planned: &[(StackedNode, DispatchPlan)],
        main: &str,
    ) -> Vec<(Base, Vec<String>)> {
        let nodes: Vec<StackedNode> = planned.iter().map(|(s, _)| s.clone()).collect();
        let placed: BTreeMap<String, PlacedNode> = planned
            .iter()
            .map(|(s, t)| (s.node_id.clone(), PlacedNode::of(t)))
            .collect();
        // Nodes that will be dispatched, not just reused as a base.
        let fresh: BTreeSet<String> = planned
            .iter()
            .filter(|(_, t)| !t.ready())
            .map(|(s, _)| s.node_id.clone())
            .collect();
        let chains = chains_to_link(&nodes, &fresh);
        for (base, chain) in &chains {
            let base = base_branch(base, main, &placed).unwrap_or_default();
            let prs: Vec<&str> = chain
                .iter()
                .filter_map(|id| placed.get(id).map(|p| p.pr.as_str()))
                .collect();
            println!(
                "Would link: GH_REPO={} gh stack link --base {base} {}",
                self.repo,
                prs.join(" ")
            );
        }
        chains
    }

    /// Whether `gh stack` works; warns that linking is skipped when it doesn't.
    pub(super) fn gh_stack_available(&self) -> bool {
        match github::gh_stack(self.root, self.repo, &["--help"]) {
            Ok(()) => true,
            Err(e) => {
                eprintln!(
                    "warning: gh-stack is unavailable ({e:#}), so `gh stack link` will be skipped; \
                     the PRs are still chained by their base branches"
                );
                false
            }
        }
    }

    /// Carry out each stack node, skipping nodes whose dependency got no
    /// branch and PR; a ready PR is only used as a base (no claim, worktree
    /// or session). Then link `chains`, mark nodes in progress and print the
    /// tally. Returns how many failed.
    pub(super) fn run_stack(
        &self,
        planned: Vec<(StackedNode, DispatchPlan)>,
        chains: &[(Base, Vec<String>)],
        main: &str,
    ) -> usize {
        let mut progress = NodeProgress::new();
        // Nodes whose branch and PR exist, even if their session then failed.
        let mut placed: BTreeMap<String, PlacedNode> = BTreeMap::new();
        let mut tally = Tally::default();
        for (s, plan) in planned {
            let id = s.node_id;
            if base_branch(&s.base, main, &placed).is_none() {
                eprintln!("skip: node {id}: its dependency was not dispatched");
                continue;
            }
            let outcome = if plan.ready() {
                use_as_base(&plan, &mut progress)
            } else {
                self.dispatch_one(&plan, &mut progress)
            };
            tally.record(&outcome.launched, &format!("node {id} (#{})", plan.number));
            if let Some(pr) = outcome.pr_url {
                if outcome.launched.is_err() {
                    eprintln!("  its branch and PR exist; dependents still stack on it");
                }
                placed.insert(
                    id,
                    PlacedNode {
                        branch: plan.branch,
                        pr,
                    },
                );
            }
        }
        for (base, chain) in chains {
            self.link_chain(base, chain, main, &placed);
        }
        self.finish(&progress, tally)
    }

    /// `gh stack link` the placed bottom of `chain` (warnings only).
    pub(super) fn link_chain(
        &self,
        base: &Base,
        chain: &[String],
        main: &str,
        placed: &BTreeMap<String, PlacedNode>,
    ) {
        // Only the dispatched bottom of a chain can be linked.
        let prs: Vec<&str> = chain
            .iter()
            .map_while(|id| placed.get(id).map(|p| p.pr.as_str()))
            .collect();
        let Some(base) = base_branch(base, main, placed).filter(|_| prs.len() >= 2) else {
            return;
        };
        let mut args = vec!["link", "--base", &base];
        args.extend(&prs);
        match github::gh_stack(self.root, self.repo, &args) {
            Ok(()) => println!("Linked stack on {base}: {}", prs.join(" ")),
            Err(e) => eprintln!(
                "warning: `gh stack link` failed: {e:#}\n  \
                 the PRs are still chained by their base branches"
            ),
        }
    }
}

/// A ready PR in a stack: nothing to dispatch, only its branch to stack on.
pub(super) fn use_as_base(plan: &DispatchPlan, progress: &mut NodeProgress) -> Outcome {
    if let Some(warning) = plan.base_mismatch_warning() {
        eprintln!("{warning}");
    }
    queue_node_progress(plan, progress);
    println!(
        "#{}  {READY_NO_RELAUNCH}; used as a base ({} \u{2190} {})",
        plan.number, plan.branch, plan.base
    );
    Outcome {
        pr_url: plan.pr.as_ref().map(|pr| pr.url.clone()),
        launched: Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::test_support::*;
    use rstest::rstest;
    use NodeStatus::*;

    #[rstest]
    #[case::empty(vec![], vec![], &[])]
    #[case::chain(
        vec![node("a", Pending, &[]), node("b", Pending, &["a"]), node("c", Pending, &["b"])],
        vec![on("a", None), on("b", Some("a")), on("c", Some("b"))], &[])]
    #[case::dependencies_first(
        vec![node("c", Pending, &["b"]), node("b", Pending, &["a"]),
             node("x", Pending, &[]), node("a", Pending, &[])],
        vec![on("x", None), on("a", None), on("b", Some("a")), on("c", Some("b"))], &[])]
    #[case::fan_out(
        vec![node("a", Pending, &[]), node("b", Pending, &["a"]),
             node("c", Pending, &["a"]), node("r", Pending, &[])],
        vec![on("a", None), on("b", Some("a")), on("c", Some("a")), on("r", None)], &[])]
    // Objective #141's shape: 5.5 waits on 5.3 and 5.4, and 5.4 on 5.3.
    #[case::transitive_dependency_is_no_diamond(
        vec![node("5.3", InProgress, &[]), node("5.4", Pending, &["5.3"]),
             node("5.5", Pending, &["5.3", "5.4"])],
        vec![on("5.3", None), on("5.4", Some("5.3")), on("5.5", Some("5.4"))], &[])]
    // Done dependencies base on the default branch; unknown ones are ignored.
    #[case::done_and_unknown_dependencies(
        vec![node("a", Done, &[]), node("b", Done, &[]),
             node("c", Pending, &["a", "b"]), node("d", Pending, &["a", "c", "zz"])],
        vec![on("c", None), on("d", Some("c"))], &[])]
    #[case::diamond_and_its_dependents(
        vec![node("a", Pending, &[]), node("b", Pending, &[]),
             node("d", Pending, &["a", "b"]), node("e", Pending, &["d"])],
        vec![on("a", None), on("b", None)],
        &[("d", "diamond: waits on a, b"), ("e", "depends on skipped node d")])]
    // b reaches c only through done x, whose work is on the default branch
    // without c's: b's branch doesn't contain c, so n is a diamond.
    #[case::path_through_a_done_node(
        vec![node("c", Pending, &[]), node("x", Done, &["c"]),
             node("b", Pending, &["x"]), node("n", Pending, &["b", "c"])],
        vec![on("c", None), on("b", None)], &[("n", "diamond: waits on b, c")])]
    #[case::cycle_among_done_nodes(
        vec![node("x", Done, &["y"]), node("y", Done, &["x"])], vec![], &[])]
    fn order_for_stack_bases_each_node_on_its_dependency(
        #[case] nodes: Vec<ObjectiveNode>,
        #[case] want: Vec<StackedNode>,
        #[case] want_skipped: &[(&str, &str)],
    ) {
        let StackOrder { stacked, skipped } = order_for_stack(&nodes).unwrap();
        assert_eq!(stacked, want);
        assert_eq!(skipped.len(), want_skipped.len(), "{skipped:?}");
        for ((id, why), (want_id, want_why)) in skipped.iter().zip(want_skipped) {
            assert!(id == want_id && why.starts_with(want_why), "{id}: {why}");
        }
    }

    #[test]
    fn order_for_stack_reports_cycles() {
        let nodes = [
            node("a", Pending, &[]),
            node("b", Pending, &["c"]),
            node("c", Pending, &["b"]),
        ];
        assert_result(
            order_for_stack(&nodes),
            Err("cycle among objective nodes b, c"),
        );
    }

    #[test]
    fn straight_chains_split_at_forks_and_roots() {
        // a -> b -> {c -> d, e}; r alone.
        let targets = [
            on("a", None),
            on("b", Some("a")),
            on("c", Some("b")),
            on("d", Some("c")),
            on("e", Some("b")),
            on("r", None),
        ];
        let chain = |base: Option<&str>, ids: &[&str]| {
            (
                on("", base).base,
                ids.iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            straight_chains(&targets),
            vec![
                chain(None, &["a", "b"]),
                chain(Some("b"), &["c", "d"]),
                chain(Some("b"), &["e"]),
                chain(None, &["r"])
            ]
        );
    }

    #[rstest]
    #[case::nothing_fresh(&[], &[])]
    #[case::fresh_top(&["d"], &[(Some("b"), &["c", "d"][..])])]
    #[case::fresh_bottom(&["a"], &[(None, &["a", "b"][..])])]
    #[case::fresh_single_never_links(&["e", "r"], &[])]
    fn chains_to_link_needs_a_fresh_node(
        #[case] fresh: &[&str],
        #[case] want: &[(Option<&str>, &[&str])],
    ) {
        // a -> b -> {c -> d, e}; r alone.
        let targets = [
            on("a", None),
            on("b", Some("a")),
            on("c", Some("b")),
            on("d", Some("c")),
            on("e", Some("b")),
            on("r", None),
        ];
        let fresh = fresh.iter().map(std::string::ToString::to_string).collect();
        let want: Vec<_> = want
            .iter()
            .map(|(base, ids)| {
                (
                    on("", *base).base,
                    ids.iter().map(std::string::ToString::to_string).collect(),
                )
            })
            .collect();
        assert_eq!(chains_to_link(&targets, &fresh), want);
    }
}
