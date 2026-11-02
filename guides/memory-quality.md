# Memory quality reference

This is a reference for extraction quality decisions. Read it when you're unsure whether a learning is worth keeping, or when reviewing synthesized files from `engram learn`.

---

## The extraction bar

A learning passes if **all three** are true:

1. **Cross-cutting** — useful in ≥2 distinct future situations, in different parts of the codebase or different commands. If the insight only applies to one function in one module, it belongs in a source comment, not in memory.

2. **Non-obvious** — an agent reading the relevant source file for 10 minutes should not be able to derive it. If it's just describing what the code does, it adds no value.

3. **Captures failure, hidden constraint, or WHY** — it either records something that went wrong (so the next agent avoids it), explains an external constraint that the code can't express (GitHub's auto-close behaviour, `claude -p`'s CLAUDE.md loading), or explains a design decision whose motivation would otherwise be invisible.

---

## What NOT to extract

Skip these, even if they're accurate and true:

- **Import paths, function signatures, struct field lists** — agents use the compiler and the source
- **Single-module implementation details** — e.g. exactly how `slugify()` works inside `src/memory/mod.rs`; no other module cares
- **Things derivable from `cargo doc` or `--help`** — API docs belong in `///` comments; flags are in `engram --help`

---

## Common over-extraction (delete these)

Most synthesized learnings that reviewers cut fall into these patterns. Drop a candidate that matches one, however accurate it is.

- **Restates the always-loaded instructions.** CLAUDE.md and AGENTS.md load in every session; memory files load only when `read_when` matches. Anything those files already say, or that follows directly from them, is pure duplication — even if this issue is where the rule was learned. Smell: you can point at the paragraph in the always-loaded instructions that says it (e.g. "add `.context()` at every call boundary" when the error-handling section already does).
- **Spec drift / history narration.** "X shipped differently than issue #N / the plan / the old tool did." The issue is closed and the code is the source of truth; a future agent works from the code, not the issue. Smell: an issue or node number in the title or slug (`objective-141-…`, `…-differs-from-5-4-spec`), or "differs from", "instead of what the issue said". Keep it only if the divergence is itself a trap that will recur — and then state the trap, not the history.
- **One-off process advice.** Coordination tied to a single event (e.g. "merge the PR retiring tool X after the stack") won't recur in the same form. Smell: it names specific PRs, branches or a one-time migration.
- **Several angles on one insight.** When candidates from the same change share a root cause, return the strongest one — or none.

---

## Before you return

Check every candidate; drop it at the first "yes":

1. Do the always-loaded instructions already say it?
2. Does its title or slug carry an issue/node number, or does it narrate how the result differs from an issue, spec or older tool?
3. Is it tied to one event, PR or migration?
4. Would a future agent working from the current code get it from 10 minutes with the source?
5. Does another candidate share its root insight and say it better?

What survives must record a failure, an external constraint, or a non-obvious WHY that applies beyond this feature. **Most issues yield 0–1 learnings; an empty result is normal and good.** A file that restates what agents already know costs tokens in every matching session and dilutes the files that matter.

---

## What TO extract (with engram examples)

**Cross-cutting CLI wiring decisions:**
> `claude -p` must always run from `std::env::temp_dir()`, not from the repo root. This affects every module that shells out to Claude. If run from the repo, Claude Code loads CLAUDE.md and begins acting as an agent instead of returning JSON.
>
> Applies to: `src/integrations/claude.rs:synthesize_learnings`, `src/integrations/claude.rs:compact_learnings`, any future function that shells out to Claude.

**Hidden external constraints:**
> GitHub auto-closes an issue when a PR with `closes #N` in its body merges. If `cmd_land` calls `gh issue close N` after this has already happened, the gh CLI returns a non-zero exit code. Check issue state before closing.
>
> Applies to: `src/workflow/plan.rs:land`, any future command that mutates issue state.

**Anti-patterns found the hard way:**
> `write_claude_md_section` used `find()` to locate `<!-- engram:end -->`. If a memory file's body contained that string literally, the marker was found inside the content rather than after it, truncating the section. Use `rfind()` to always match the last occurrence.
>
> Applies to: `src/memory/mod.rs:write_claude_md_section`, any future function that searches for HTML comment delimiters in file content.

---

## Writing for agents, not humans

Agents read memory files at decision time — they're about to do something and need to know if there's a gotcha. Write accordingly:

- **Use the imperative**: "Always use `current_dir(std::env::temp_dir())` for `claude -p` invocations." Not: "The code uses temp_dir for Claude."
- **Name the consequence of getting it wrong**: "...otherwise Claude Code loads CLAUDE.md and acts as an agent instead of returning JSON."
- **Use source pointers instead of copying code**: `see src/integrations/claude.rs:synthesize_learnings`. Copied code rots; pointers stay accurate as long as the symbol exists.
- **One paragraph maximum** per body. If it needs more, split into two files.

---

## `read_when` quality

Each condition must be a concrete phrase that completes: *"I should read this when I am..."*

| Bad | Good |
|-----|------|
| `"working with Claude"` | `"calling claude -p programmatically from engram"` |
| `"GitHub issues"` | `"closing a GitHub issue from cmd_land"` |
| `"memory files"` | `"writing memory content into CLAUDE.md"` |
| `"error handling"` | `"debugging a JSON parse error from claude -p output"` |

A condition should be specific enough that an agent in a different context would NOT load the file. Over-broad `read_when` conditions waste tokens on every irrelevant task.

---

## Tripwire format

Tripwires live in `tripwires/` files and are also embedded in the `tripwires:` YAML block in the frontmatter.

```yaml
tripwires:
  - action: "Invoking claude -p with current_dir set to a repo directory"
    warning: "Always use current_dir(std::env::temp_dir()) — repo CLAUDE.md will cause Claude to act as an agent instead of returning JSON"
```

Rules:
- `action` starts with a **gerund** ("Invoking", "Calling", "Passing") or the word "Before": describes the thing the agent is about to do
- `warning` is **imperative**: tells the agent what to do instead, and names the consequence if ignored
- Both fields fit on one line — no multi-sentence explanations in the YAML block (use the body for depth)
