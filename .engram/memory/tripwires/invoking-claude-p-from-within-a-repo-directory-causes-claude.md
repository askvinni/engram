---
title: "Invoking claude -p from inside a repo directory loads CLAUDE.md as agent context"
read_when:
  - "calling claude -p programmatically from engram"
  - "implementing or changing synthesize_learnings() or any function that shells out to Claude"
  - "debugging unexpected Claude behavior where it acts as an agent instead of returning JSON"
tripwires:
  - action: "Invoking claude -p with current_dir set to a repo that has a CLAUDE.md"
    warning: "CLAUDE.md is loaded as context, turning a JSON synthesis call into an action-taking agent session; always use current_dir(std::env::temp_dir()) for non-interactive Claude invocations"
last_updated: "2026-05-18"
source_issues: [26]
---

Claude Code loads CLAUDE.md from the working directory (and parent directories) when invoked. When engram calls `claude -p` to synthesize learnings, if the working directory is the user's repo, Claude reads CLAUDE.md — which contains engram memory — and interprets the call as an agentic session rather than a simple prompt/response. The symptom is Claude trying to write files or take actions instead of returning a JSON array. The fix is to set `current_dir(std::env::temp_dir())` on the Command builder, which has no CLAUDE.md. See src/claude.rs:synthesize_learnings.
