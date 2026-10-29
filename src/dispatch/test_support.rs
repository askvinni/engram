//! Fixtures shared by the dispatch tests.

use super::stack::{Base, StackedNode};
use super::*;

pub(super) fn plan(kata_ref: Option<&str>) -> PlanFields {
    PlanFields {
        number: 142,
        kata_ref: kata_ref.map(Into::into),
        title: "Add dispatch preflight!".into(),
        meta: BTreeMap::from([
            ("jira".into(), "ENG-7".into()),
            ("type".into(), "feat".into()),
        ]),
    }
}

/// The expected value, or text the error's full message contains.
pub(super) type Want<T> = std::result::Result<T, &'static str>;

pub(super) fn assert_result<T: PartialEq + std::fmt::Debug>(got: Result<T>, want: Want<T>) {
    match (got, want) {
        (Ok(got), Ok(want)) => assert_eq!(got, want),
        (Err(e), Err(part)) => assert!(format!("{e:#}").contains(part), "{e:#}"),
        (got, want) => panic!("got {got:?}, want {want:?}"),
    }
}

pub(super) fn assert_mentions(text: &str, has: &[&str], lacks: &[&str]) {
    for part in has {
        assert!(text.contains(part), "{part:?} missing from\n{text}");
    }
    for part in lacks {
        assert!(!text.contains(part), "{part:?} unexpected in\n{text}");
    }
}

pub(super) fn target() -> DispatchPlan {
    DispatchPlan {
        number: 143,
        title: "[5.2] Add engram dispatch".into(),
        body: "Objective: #141 (node 5.2)\n\n## Why\nUse `$HOME` and \"quotes\".\n".into(),
        kata_ref: Some("mfbq".into()),
        claimed_by_me: false,
        meta: kata::TrackerMeta::default(),
        parent: None,
        branch: "143-add-engram-dispatch".into(),
        branch_state: BranchState::Missing,
        on_origin: false,
        pr_title: "[5.2] Add engram dispatch".into(),
        base: "main".into(),
        pr: None,
        pr_link: None,
        worktree: None,
        objective_node: Some((141, "5.2".into())),
        stacked_on: None,
    }
}

pub(super) fn with_pr(mut t: DispatchPlan, base: &str, draft: bool) -> DispatchPlan {
    t.pr = Some(OpenPr {
        number: 9,
        url: "https://github.com/o/r/pull/9".into(),
        base: base.into(),
        head: t.branch.clone(),
        is_draft: draft,
        title: t.pr_title.clone(),
    });
    t
}

/// Found by its body, ready for review, its branch also local; `base` is
/// the computed one, the PR's own is `main`.
pub(super) fn reused(base: &str) -> DispatchPlan {
    let mut t = with_pr(target(), "main", false);
    t.pr_link = Some(PrLink::BodyKeyword);
    t.branch_state = BranchState::Local;
    t.base = base.into();
    t
}

pub(super) fn node(id: &str, status: NodeStatus, deps: &[&str]) -> ObjectiveNode {
    ObjectiveNode {
        id: id.into(),
        description: id.into(),
        status,
        plan_issue: None,
        pr_url: None,
        depends_on: deps.iter().map(std::string::ToString::to_string).collect(),
    }
}

pub(super) fn planned(id: &str, status: NodeStatus, deps: &[&str], plan: u64) -> ObjectiveNode {
    ObjectiveNode {
        plan_issue: Some(plan),
        ..node(id, status, deps)
    }
}

pub(super) fn on(id: &str, base: Option<&str>) -> StackedNode {
    StackedNode {
        node_id: id.into(),
        base: base.map_or(Base::Main, |b| Base::OnNode(b.into())),
    }
}
