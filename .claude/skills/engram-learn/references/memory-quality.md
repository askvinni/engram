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

- **Import paths and use declarations** — agents use the compiler to find these
- **Function signatures and struct field lists** — read the source
- **Single-module implementation details** — e.g. exactly how `slugify()` works inside `src/memory.rs`; no other module cares
- **Patterns already universal in the codebase** — e.g. "use `anyhow::Result`"; it's in every file and in CLAUDE.md
- **Things derivable from `cargo doc`** — public API docs belong in `///` comments, not memory files
- **Descriptions of what a CLI flag does** — agents can run `engram --help`

---

## What TO extract (with engram examples)

**Cross-cutting CLI wiring decisions:**
> `claude -p` must always run from `std::env::temp_dir()`, not from the repo root. This affects every module that shells out to Claude. If run from the repo, Claude Code loads CLAUDE.md and begins acting as an agent instead of returning JSON.
>
> Applies to: `src/claude.rs:synthesize_learnings`, `src/claude.rs:compact_learnings`, any future function that shells out to Claude.

**Hidden external constraints:**
> GitHub auto-closes an issue when a PR with `closes #N` in its body merges. If `cmd_land` calls `gh issue close N` after this has already happened, the gh CLI returns a non-zero exit code. Check issue state before closing.
>
> Applies to: `src/main.rs:cmd_land`, any future command that mutates issue state.

**Anti-patterns found the hard way:**
> `write_claude_md_section` used `find()` to locate `<!-- engram:end -->`. If a memory file's body contained that string literally, the marker was found inside the content rather than after it, truncating the section. Use `rfind()` to always match the last occurrence.
>
> Applies to: `src/memory.rs:write_claude_md_section`, any future function that searches for HTML comment delimiters in file content.

---

## Writing for agents, not humans

Agents read memory files at decision time — they're about to do something and need to know if there's a gotcha. Write accordingly:

- **Use the imperative**: "Always use `current_dir(std::env::temp_dir())` for `claude -p` invocations." Not: "The code uses temp_dir for Claude."
- **Name the consequence of getting it wrong**: "...otherwise Claude Code loads CLAUDE.md and acts as an agent instead of returning JSON."
- **Use source pointers instead of copying code**: `see src/claude.rs:synthesize_learnings`. Copied code rots; pointers stay accurate as long as the symbol exists.
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
