---
name: engram-objective
description: Use this skill when the user asks to "create an objective", "open an objective issue", "write an objective for X", "use engram objective", "start a new multi-node goal", or is about to begin work that will be tracked as an engram-objective GitHub issue. Provides a guided six-step flow for creating well-formed objective issues that work with --all-unblocked dispatch and auto-close mechanics.
version: 0.2.0
allowed-tools: Bash(engram objective*)
---

# engram-objective skill

## Input

Goal description: **$ARGUMENTS**

If `$ARGUMENTS` is empty, ask the user to describe the goal before continuing.

An **engram objective** is a GitHub issue labeled `engram-objective`. It represents a multi-node goal broken into a DAG of plan nodes. Each node maps to one plan issue (one PR). The `engram objective plan <issue> --all-unblocked` command dispatches all nodes whose dependencies are complete; the auto-close hook closes the objective when every node reaches `done`.

The quality of the objective body determines whether `--all-unblocked` and auto-close work correctly. A malformed node list silently prevents dispatch. A thorough body with all sections and a valid bullet-list roadmap produces a self-managing objective.

## The objective creation process — six steps, always in order

### Step 0 — Intercept vague requests

If the user has not provided a clear, scoped goal, do **not** ask for permission to explore. Say: *"Before I draft the objective, let me look at the codebase and open issues."* Then proceed to Step 1 immediately. Never skip exploration.

### Step 1 — Explore the codebase (mandatory, before writing anything)

Before drafting anything:

1. Read the source files most obviously relevant to the stated goal
2. Check `.engram/memory/index.md` for existing memory that applies
3. List open objectives to avoid duplication: `engram objective list`
4. Scan recent closed issues for prior related work: `gh issue list --state closed --label engram-plan --limit 20`
5. If the scope is unclear, read adjacent files to find the natural seam

**Rule: never draft an objective without first reading the relevant source files.** Objectives drafted blind produce node lists that don't map to the real work.

### Step 2 — Draft the full objective body silently

After exploration, draft the complete body internally. Produce the whole thing at once. The body must contain exactly these sections:

- `## Goal` — one paragraph stating what the completed objective achieves
- `## Background` — 2–4 sentences for a cold reader; which files or subsystems are involved
- `## Roadmap` — bullet list, one node per line, using the format below
- `## Acceptance criteria` — checkable outcomes for the whole objective
- `## Scope` — what changes, what explicitly does not

**Node bullet format for `## Roadmap`:**
```
- 1: Description of first node
- 2: Description of second node (depends: 1)
- 3: Description of third node (depends: 1, 2)
```

Rules for nodes:
- IDs are local labels used only for `depends` — `1`, `2`, `3` is simplest. `engram objective new` renumbers every node to `<issue-number>.<n>` (in list order) once the issue exists, rewriting `depends` to match
- Labels must be unique and `depends` must reference labels in the same list, or `objective new` refuses before creating anything
- Every node must complete in one PR — split large nodes further
- Start simple: 3–6 nodes covers most goals. More nodes are fine if each is genuinely independent.
- Prefer a linear chain over a wide fan-out unless parallelism is real

### Step 3 — Present the draft and surface weak sections

Show the complete draft body. Render a preview roadmap table as Markdown so the user can scan it easily:

```
| ID | Description           | Depends On |
|----|-----------------------|------------|
| 1  | First node            |            |
| 2  | Second node           | 1          |
```

Then explicitly flag any weak section:

- *"Roadmap only has 1 node — is this really a multi-node objective, or should this be a plan issue instead?"*
- *"Node 3 depends on both 1 and 2 — is that intentional or should it only depend on 2?"*
- *"Acceptance criteria is empty — how will we know the objective is complete?"*

Do not proceed until the user has confirmed the draft or made corrections.

### Step 4 — Validate the title and node graph

**Title checks:**
- Must start with a verb (`Add`, `Build`, `Refactor`, `Migrate`, `Replace`, `Extract`…)
- Must describe the *outcome*, not the work — `"Add streaming support to the API"` not `"Streaming investigation"`
- Must be specific enough that a cold reader could guess the node list

**Cycle detection:**
Perform a topological sort on the `depends_on` graph before creating the issue. A cycle exists when a node (transitively) depends on itself. If a cycle is detected:
1. Show which nodes form the cycle
2. Explain why it is a problem (dispatch would loop forever; auto-close would never fire)
3. Re-enter the confirm step — do **not** create the issue

If the title fails any check, propose a corrected version and confirm before continuing.

### Step 5 — Scope sanity check

Re-read Scope against what you found in Step 1. Surface mismatches:

- If Scope names only one file but the node list spans five subsystems, flag it
- If a node is so large it would require multiple PRs, flag it and suggest splitting
- Confirm the overall goal fits the `engram objective` model — if all nodes are in a single file and could be done in one PR, a plain `engram plan new` is better

### Step 6 — Create the issue

Only after steps 1–5 pass. Run:

```
engram objective new "<title>" --body "<full body with ## Goal, ## Background, ## Roadmap (bullets), ## Acceptance criteria, ## Scope>"
```

Print the new issue URL. Then add one sentence on next steps:

*"Run `engram objective plan <issue> --all-unblocked` to create plan issues for all nodes that are ready to start."*

---

## Objective body template

Every objective body must contain these sections, in this order:

### **Goal**
One paragraph. What does the completed objective achieve? Must be falsifiable — a reader should be able to tell whether it's done.

### **Background**
2–4 sentences. Which files or subsystems are involved? What is the user experience before this objective is complete? A cold reader should understand the gap from Goal + Background alone.

### **Roadmap**
Bullet list of nodes, one per line. Format:
```
- 1: Description
- 2: Description (depends: 1)
```
This section is parsed by `parse_nodes_from_roadmap_input` in `src/objective.rs`. The `engram objective new` command converts it to a rendered Markdown table plus the hidden `<!-- engram:nodes [...] -->` state comment. Do **not** write the table or the JSON manually — the CLI generates them.

### **Acceptance criteria**
Checkable outcome checklist for the whole objective. Each item must be verifiable. *"works correctly"* fails; *"`engram objective list` no longer shows this issue after all nodes reach done"* passes.

### **Scope**
Which source files or subsystems change. Call out explicitly what does **not** change if someone might wonder.

---

## Node ID conventions

engram assigns node IDs: once `engram objective new` has created the issue, it renumbers every node to `<issue-number>.<n>` (n = 1-based position in the roadmap list) and rewrites each `depends` to the new IDs. The IDs you write in `## Roadmap` are only local labels for expressing `depends` — any unique token works (`1`, `a`, or the older `1.1` style). Do not try to guess the issue number. Use the assigned IDs (e.g. `42.1`) with `engram objective plan --node`.

`engram objective append <issue>` assigns the next ID (`<issue>.<max + 1>`) when `--id` is omitted.

The `plan_issue` and `pr_url` fields in the hidden JSON are `null` until `engram objective plan` populates them. The skill does not set these — the CLI handles it.

---

## Cycle detection algorithm

Given the node bullet list, build an adjacency map from `depends_on` edges and run a DFS with three-color marking (white/gray/black). Report a cycle if a gray node is encountered during traversal. Example error to surface:

```
Cycle detected: 1.2 → 1.3 → 1.2
Fix: remove the dependency from 1.3 back to 1.2, or introduce a new node that both can depend on.
```

Do not create the issue until the graph is acyclic.

---

## When to use engram objective vs. engram plan

Use **engram objective** when:
- The goal requires more than one PR
- Different nodes can be worked on independently (or in sequence)
- You want `--all-unblocked` batch dispatch and auto-close

Use **engram plan** when:
- The work fits in exactly one PR
- There is no natural decomposition into independent sub-tasks

If a stated "objective" turns out to have only one node, suggest using `engram plan new` instead and do not create an objective.

---

## Full workflow

```
# 1. Create the objective issue (use this skill to draft the body)
engram objective new "Verb-phrase title" --body "..."

# 2. Dispatch all ready nodes
engram objective plan <issue> --all-unblocked

# 3. Work on each plan issue; open PRs with "closes #<plan-issue>" in the PR body

# 4. After each PR merges, run engram plan land <plan-issue>
#    The auto-close hook updates the objective node to done and closes the
#    objective when all nodes are complete.
```
