---
name: engram-plan
description: Use this skill when the user asks to "create a plan", "open a plan issue", "write a plan for X", "use engram plan", "start a new feature", or is about to begin work that will be tracked as an engram-plan GitHub issue. Provides guidance on writing effective engram plan issues that produce useful learnings later.
version: 0.3.0
allowed-tools: Bash(engram plan*)
---

# engram-plan skill

## Input

Title or description of the work: **$ARGUMENTS**

If `$ARGUMENTS` is empty, ask the user what they want to plan before continuing.

An **engram plan** is a GitHub issue labeled `engram-plan`. It represents exactly one unit of work — one PR from start to merge. After the PR merges, `engram learn <N>` (or `engram land <N>`) synthesizes cross-cutting learnings from the issue and PR into `.engram/memory/`.

The quality of the plan body directly determines the quality of the memory files `engram learn` produces. A bare plan (Why/What/Scope only) produces shallow or no learnings. A thorough plan (all seven sections) produces specific, well-routed pattern and tripwire files that help future work.

## The planning process — six steps, always in order

### Step 0 — Intercept vague requests

If the developer has not provided a detailed body (or has provided only a title), do **not** ask for permission to explore. Say: *"Before I draft the plan, let me look at the relevant code."* Then proceed to Step 1 immediately. Never skip exploration.

### Step 1 — Explore the codebase (mandatory, before writing anything)

Before drafting a single word of the plan body:

1. Read the source files most obviously relevant to the stated change
2. Check `.engram/memory/index.md` for existing memory that applies to this area; load any matching topic files
3. Scan recent closed issues for similar prior work: `gh issue list --repo <repo> --state closed --label engram-plan --limit 20`
4. If the scope is unclear, read adjacent files to understand where the natural seam is

**Rule: never draft a plan body without first reading the relevant source files.** Plans written blind miss the real problem and produce learnings that don't map to the code.

### Step 2 — Draft the full seven-section body silently

After exploration, draft the complete body internally. Do not present it section by section. Produce the whole thing at once, filling all seven sections with specific content grounded in what you found in Step 1.

### Step 3 — Present the draft and surface weak sections

Show the complete draft. Then explicitly flag any section that is thin or vague — this is the bumper. Examples:

- *"Approach only has one bullet — I found the relevant function in `src/compact.rs` but there's no clear pattern to follow. Should we look at how `learn.rs` handles the similar case?"*
- *"Edge cases section is empty — does concurrent invocation or an empty memory directory need handling here?"*
- *"Key files names `src/github.rs` but the change actually happens in `cmd_plan()` in `src/main.rs` — did you mean to list that instead?"*

Do not proceed until the developer has confirmed the draft or made corrections.

### Step 4 — Validate the title

Check the proposed title against these rules:

- Must start with a verb (`Add`, `Fix`, `Refactor`, `Remove`, `Extract`, `Support`…)
- Must describe the *change*, not the *work* — `"Add --dry-run flag"` not `"Investigate dry run"`
- Must be specific enough to route a memory file — test: *"Could you write a `read_when` condition for a memory file derived only from this title?"* If not, the title is too vague

If the title fails any check, propose a corrected version and confirm before continuing.

### Step 5 — Scope sanity check

Re-read Scope and Key files against what you found in Step 1. Surface mismatches:

- If Scope names only one file but Approach implies changes across three, flag it
- If Key files lists a file that doesn't exist or isn't the real entry point, flag it
- Confirm the work fits in **one PR** — if not, the plan must be split into two issues

### Step 6 — Create the issue

Only after steps 1–5 pass. Run:

```
engram plan "<title>" --body "<seven-section body>"
```

Print the issue URL. Then add one sentence on what would make this plan produce strong learnings when `engram learn` runs — for example: *"Make sure the PR diff shows the new parsing logic clearly — that's what synthesis will use to generate the pattern file."*

---

## The seven required sections

Every plan body must contain all seven sections. `engram plan` will warn on stderr if any are missing.

### **Why**
One sentence. What is broken or missing today? Must be falsifiable — *"X fails when Y"* or *"X is impossible today."* Not *"X could be better."*

### **Background**
2–4 sentences for a cold reader. Which files or functions are involved? What does the user experience look like before this change? Someone who has never seen the codebase should understand the gap from Why + Background alone.

### **Approach**
3–6 bullet points on the technical strategy. Cover: which module handles the change, what data flows where, what the key tradeoff is. This is the section `engram learn` draws on most heavily when synthesising pattern and architecture learnings — vague bullets produce vague memory files.

### **Acceptance criteria**
Checkable outcome checklist. Each item must be verifiable by running a command or reading a diff. *"works correctly"* fails. *"`engram plan --body ''` prints a warning to stderr and still creates the issue"* passes.

### **Scope**
Which source files change, and which plausibly-adjacent files do **not** change (call them out explicitly if someone might wonder). One line per module is fine.

### **Edge cases and risks**
What can go wrong? What inputs or states need special handling? What is explicitly excluded because it would make this too large? Even two bullets here produce tripwire learnings that prevent future regressions.

### **Key files**
3–6 `src/foo.rs:FunctionName` entry points. A cold reader should be able to open these and orient immediately without reading the whole codebase.

---

## Validation warnings (bumpers, not hard blocks)

Emit a visible warning — but do not refuse to create the issue — when:

- Approach has fewer than 3 bullets
- Edge cases section is absent
- Scope does not name at least one specific file
- Title does not start with a verb
- Body is under 200 words (proxy for "not thought through")
- Plan would require more than one PR — split it instead

---

## Title conventions

The title becomes the GitHub issue title **and** is fed to Claude as context during `engram learn`. Write it as a concise verb phrase describing the change:

- Good: `"Add --dry-run flag to compact command"`
- Good: `"Fix branch-not-found error in cmd_land when PR already merged"`
- Bad: `"Dry run"` (too vague — Claude can't derive `read_when` conditions from this)
- Bad: `"Investigate the compact issue"` (describes the work, not the change)

The title is used by `synthesize_learnings` in `src/claude.rs` as part of the synthesis prompt. A precise title produces more specific `read_when` routing conditions in the memory files.

---

## Scope discipline — one plan, one PR

`engram learn <N>` finds the merged PR that closed issue `N` via GitHub's `CLOSED_EVENT` graph. It expects **exactly one** such PR. If the work splits into two PRs, create two plan issues — one closes each PR.

The PR body must contain `closes #N` (or `fixes #N`, `resolves #N`). Without that marker, GitHub does not generate the close event and `engram learn` will report "no linked PR found."

---

## Branch naming

`engram land <N>` tries three branch name patterns in order:
1. `fix/issue-N`
2. `feat/issue-N`
3. `issue-N`

Name your working branch to match one of these so `land` can delete it automatically after close. Any other name requires manual branch cleanup.

---

## When NOT to create an engram plan

Skip the plan if the work is:
- A single-file typo/rename with no cross-cutting insight worth capturing
- A documentation-only change
- A revert of a recent commit

Plans generate learnings. If the work won't teach Claude anything new about the codebase, a plain branch + PR is sufficient.

---

## Full workflow

```
# 1. Create the plan issue (use this skill to draft the body)
engram plan "Verb-phrase title" --body "..."

# 2. Create a branch
git checkout -b feat/issue-<N>

# 3. Implement; open PR with "closes #N" in the PR body

# 4. After PR merges:
engram land <N>
# land = engram learn <N> + close issue + delete local branch
```

If you want to inspect the synthesized memory files before merging the learning PR, use `engram learn <N>` instead of `land`, review the branch, then run `engram land <N>` to close and clean up.
