---
title: "When init creates a new subdirectory, write a README explaining the contract"
read_when:
  - "adding a new directory that engram init creates"
  - "implementing or changing cmd_init()"
  - "designing a new engram extension point that users interact with directly"
tripwires: []
last_updated: "2026-05-18"
source_issues: [13]
---

Any directory that users are expected to populate (like `.engram/prompt-hooks/`) should have a README written into it by init. The README should explain: what files belong here, the expected format, how they are loaded (alphabetical order, which file types), and what they affect. Writing the README at creation time means the directory self-documents at the point of discovery — a user who finds it in their repo via ls or git status immediately understands its purpose without having to read external docs. The README should be skipped when loading operational files. See src/main.rs:PROMPT_HOOKS_README and load_prompt_hooks's README.md filter.
