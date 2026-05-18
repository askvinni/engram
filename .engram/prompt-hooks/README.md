# Prompt Hooks

Markdown files in this directory are injected into the Claude prompt during
`engram learn` under a "Project-Specific Rules" section.

Use hooks to customize how learnings are classified for this repo. Examples:

- "Always classify Rust lifetime errors as tripwires."
- "This repo uses pytest — testing learnings should reference pytest patterns."
- "Prefer architecture entries for any change to the public API surface."

Files are loaded in alphabetical order. Only `.md` files are included.
This directory is committed to the repo so rules are shared across the team.
