---
title: "Cycles in an objective node DAG silently break dispatch and auto-close at runtime, not at creation time"
read_when:
  - "creating an engram-objective issue whose nodes have depends_on relationships"
  - "editing or adding dependency edges to an existing objective's node graph"
  - "implementing code that modifies the engram:nodes JSON on an objective issue"
tripwires:
  - action: "Creating an engram-objective issue without validating the depends_on graph for cycles"
    warning: "A cycle (e.g. 1.2 depends on 1.3, 1.3 depends on 1.2) causes --all-unblocked to find no dispatchable nodes forever and prevents maybe_close_objective from ever firing — run a topological sort and reject creation if a cycle is detected"
last_updated: "2026-05-19"
source_issues: [61]
---

The `--all-unblocked` dispatcher and `maybe_close_objective` both assume the objective's `depends_on` graph is a DAG. A cycle means every node in the cycle is blocked by another node in the same cycle, so no node ever reaches `done`, the auto-close hook never fires, and repeated `engram objective plan --all-unblocked` calls produce zero dispatches with no error. The failure is entirely silent at issue-creation time: the JSON comment is stored without validation and the `gh issue create` call succeeds — the broken state only surfaces when the dispatcher runs. A topological sort must be performed on the node list before calling `engram objective new`, the specific cycle must be surfaced to the user, and creation must be blocked until it is resolved. See `src/objective.rs:dispatch_unblocked` and `src/objective.rs:maybe_close_objective`.
