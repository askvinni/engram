---
title: "Strip markdown code fences from LLM output before JSON parsing"
read_when:
  - "parsing structured output from claude -p"
  - "adding a new synthesis function in claude.rs that expects JSON back"
  - "debugging JSON parse errors from claude output"
tripwires: []
last_updated: "2026-05-18"
source_issues: [26]
---

Even when a prompt instructs Claude to return only JSON, it often wraps the response in a ` ```json ... ``` ` block. Always apply a strip_code_fence() step before JSON parsing: check if the first line starts with ` ``` `, and if so, extract only the inner lines up to the closing fence. Without this, `serde_json::from_str` will fail on otherwise valid JSON. Apply the strip before the `find('[')` / `rfind(']')` extraction step. See src/claude.rs:strip_code_fence for the implementation.
