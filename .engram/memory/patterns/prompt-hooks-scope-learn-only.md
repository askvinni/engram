---
title: "Prompt hooks in .engram/prompt-hooks/ are injected only during engram learn, not other commands"
read_when:
  - "writing a prompt hook rule and expecting it to affect engram plan or other Claude-invocation commands"
  - "adding a new Claude synthesis function in claude.rs and deciding whether to inject prompt hooks"
  - "debugging why a prompt hook has no visible effect on a particular engram command"
tripwires:
  - action: "Adding a rule to .engram/prompt-hooks/ expecting it to affect all engram commands"
    warning: "Hooks are loaded only in learn.rs before synthesize_learnings() — no other command reads the hooks directory; to cover a new synthesis function, explicitly call load_prompt_hooks() and pass the result through"
last_updated: "2026-05-18"
source_issues: [13]
---

load_prompt_hooks() is called only from src/learn.rs, immediately before synthesize_learnings(). No other engram command — plan, status, land, or list — reads the .engram/prompt-hooks/ directory. This means classification rules, labeling conventions, or testing-framework hints placed in prompt hooks have no effect on plan generation or any other Claude-invoked step. The scope is invisible at hook-creation time: a user who writes 'always suggest linked tests' will see no effect unless they are running engram learn. Any new synthesis function added to claude.rs in the future must explicitly call load_prompt_hooks() and thread the result through to be covered. See src/learn.rs for the only call site and src/claude.rs:synthesize_learnings for the injection point.
