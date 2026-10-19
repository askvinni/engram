---
type: tripwire
title: 'learn''s write_topic_file drops Frontmatter.extra markers such as origin: session when it rewrites a file'
read_when:
- adding a new provenance or status marker to memory frontmatter via Frontmatter.extra
- 'implementing compact or learn logic that depends on the origin: session marker surviving'
- debugging why a captured memory file lost its origin marker after engram plan learn
tripwires:
- action: Relying on a Frontmatter.extra key to persist after memory::write_topic_file rewrites the same slug
  warning: 'write_topic_file rebuilds the frontmatter from the LearningItem, so every extra marker is lost, including origin: session. If the marker must survive, explicitly carry it over in write_topic_file and add a test like capture.rs:learn_rewrite_clears_session_marker.'
generated:
  by: engram/0.1.0
  at: 2026-10-19
sources:
- id: '90'
  resource: https://github.com/askvinni/engram/issues/90
---

Session-captured files are tagged by putting origin: session in Frontmatter.extra (see src/capture.rs:Capture::mark_session and memory::ORIGIN_KEY/ORIGIN_SESSION). This avoids adding a new top-level OKF field. When learn synthesizes an item with the same category and slug, memory::write_topic_file writes fresh frontmatter, so the marker is cleared. This is intended: a learn-reviewed file is no longer an unreviewed observation, and the test learn_rewrite_clears_session_marker locks the behavior in. The catch is that any other marker stored in extra is wiped the same way. A future marker that should outlive a learn rewrite, such as a pin, a manual-edit flag, or an import source, will vanish silently unless write_topic_file is changed to preserve it. Any compact logic that treats session files differently should also expect the set of session files to shrink after each learn cycle.
