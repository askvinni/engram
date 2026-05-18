---
name: engram plan
about: Plan a single PR worth of work using engram's structured format
labels: engram-plan
---

**Why:** [One sentence. What is broken or missing today? Must be falsifiable — "X breaks" or "X is impossible today."]

**Background:** [2–4 sentences. What does the affected code do today? Which files or functions are involved? What does the user experience look like before this change? A cold reader should understand the gap from Why + Background alone.]

**Approach:**
- [How will this be implemented? Give 3–6 bullets covering key decisions: which module handles the change, what data flows where, what the main tradeoff is. This section feeds pattern and architecture learnings in engram learn.]

**Acceptance criteria:**
- [ ] [Observable outcome — must be verifiable by running a command or reading a diff]
- [ ] [Add more items as needed]

**Scope:**
- Changes: `src/foo.rs:FunctionName`, `src/bar.rs`
- Does NOT touch: `src/baz.rs` [call out adjacent modules that are explicitly out of scope]

**Edge cases and risks:**
- [What can go wrong? What inputs or states need special handling?]
- [What is explicitly excluded because it would make this too large?]

**Key files:**
- `src/foo.rs:FunctionName` — [one line on why this is the entry point]
- `src/bar.rs` — [one line]
