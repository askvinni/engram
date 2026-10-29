use anyhow::{Context, Result};
use serde::Deserialize;
use std::process::Command;

use crate::process;

// The guides served by `engram guide` are included here so the same quality rules
// that guide interactive Claude Code sessions also govern the claude -p synthesis calls.
const LEARN_QUALITY_GUIDE: &str = crate::guide::MEMORY_QUALITY;
const COMPACT_QUALITY_GUIDE: &str = crate::guide::MEMORY;

/// Max chars of any one large prompt input (diff, conversation, instructions).
const PROMPT_SECTION_CAP: usize = 8000;
/// Repo-relative directory of the markdown rules injected into the learn prompt.
pub const PROMPT_HOOKS_DIR: &str = ".engram/prompt-hooks";

#[derive(Debug, Deserialize)]
pub struct Tripwire {
    pub action: String,
    pub warning: String,
}

#[derive(Debug, Deserialize)]
pub struct LearningItem {
    pub category: String,
    pub slug: String,
    pub title: String,
    pub read_when: Vec<String>,
    #[serde(default)]
    pub tripwires: Vec<Tripwire>,
    pub body: String,
}

/// What `compact` does with one memory file. An action Claude invents is a
/// parse error rather than silently ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactKind {
    Keep,
    Delete,
    MergeInto,
}

#[derive(Debug, Deserialize)]
pub struct CompactAction {
    pub action: CompactKind,
    pub category: String,
    pub slug: String,
    pub reason: Option<String>,
    pub target_category: Option<String>,
    pub target_slug: Option<String>,
    pub target_updated_body: Option<String>,
}

/// Everything the learn synthesis prompt is built from.
#[derive(Debug)]
pub struct SynthesisInput<'a> {
    pub issue_title: &'a str,
    pub issue_body: &'a str,
    pub pr_title: &'a str,
    pub pr_body: &'a str,
    pub pr_diff: &'a str,
    pub conversation: &'a str,
    pub current_memory: &'a str,
    pub prompt_hooks: &'a str,
    /// The repo's CLAUDE.md/AGENTS.md text (see `memory::read_always_loaded_instructions`).
    pub instructions: &'a str,
}

fn strip_code_fence(s: &str) -> String {
    let mut lines = s.lines();
    if let Some(first) = lines.next() {
        if first.starts_with("```") {
            let inner: Vec<&str> = lines.take_while(|l| !l.starts_with("```")).collect();
            return inner.join("\n");
        }
    }
    s.to_string()
}

/// Returns at most `max` chars of `s` (never splitting a UTF-8 char) and whether it was cut.
fn truncate_chars(s: &str, max: usize) -> (&str, bool) {
    match s.char_indices().nth(max) {
        Some((idx, _)) => (&s[..idx], true),
        None => (s, false),
    }
}

/// The note appended to a prompt input that `truncate_chars` cut, or "".
fn truncation_note(cut: bool) -> String {
    if cut {
        format!("\n*(truncated at {PROMPT_SECTION_CAP} chars)*\n")
    } else {
        String::new()
    }
}

/// Prompt section holding the always-loaded instructions, or "" when there are none.
fn always_loaded_section(instructions: &str) -> String {
    let instructions = instructions.trim();
    if instructions.is_empty() {
        return String::new();
    }
    let (text, cut) = truncate_chars(instructions, PROMPT_SECTION_CAP);
    let note = truncation_note(cut);
    format!(
        "\n## Always-loaded instructions (never restate)\nEvery agent session in this repo already loads the text below. Anything it says — or that follows directly from it — must not become a memory file.\n\n{text}\n{note}"
    )
}

pub fn load_prompt_hooks(repo_root: &std::path::Path) -> String {
    let hooks_dir = repo_root.join(PROMPT_HOOKS_DIR);
    if !hooks_dir.exists() {
        return String::new();
    }
    let mut entries: Vec<_> = std::fs::read_dir(&hooks_dir)
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .collect();
    entries.sort_by_key(std::fs::DirEntry::path);
    entries
        .iter()
        .filter(|e| e.file_name() != "README.md")
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn build_synthesis_prompt(input: &SynthesisInput) -> String {
    let (diff, _) = truncate_chars(input.pr_diff, PROMPT_SECTION_CAP);
    let (conversation_trimmed, conversation_cut) =
        truncate_chars(input.conversation, PROMPT_SECTION_CAP);

    let memory_section = if input.current_memory.is_empty() {
        "_none yet_".to_string()
    } else {
        input.current_memory.to_string()
    };

    let conversation_section = if conversation_trimmed.is_empty() {
        String::new()
    } else {
        format!(
            "\n## Planning Conversation\n{conversation_trimmed}\n{}",
            truncation_note(conversation_cut)
        )
    };

    let hooks_section = if input.prompt_hooks.is_empty() {
        String::new()
    } else {
        format!("\n## Project-Specific Rules\n{}\n", input.prompt_hooks)
    };

    let instructions_section = always_loaded_section(input.instructions);
    let SynthesisInput {
        issue_title,
        issue_body,
        pr_title,
        pr_body,
        ..
    } = input;

    format!(
        r#"You are analyzing a completed GitHub issue and its associated pull request to extract learnings for an AI agent memory system.

## Quality Guide
{LEARN_QUALITY_GUIDE}

---
{instructions_section}
## Closed Issue: {issue_title}

{issue_body}

## Merged PR: {pr_title}

{pr_body}

## PR Diff
{diff}
{conversation_section}
## Current Memory
{memory_section}
{hooks_section}
---

Extract 0–2 learnings. Only include a learning if it passes the extraction bar and the "Before you return" check in the quality guide above. Returning `[]` is expected and correct when nothing clears the bar.

Return ONLY a JSON array, no other text:
[
  {{
    "category": "tripwires",
    "slug": "claude-repo-context",
    "title": "Run claude -p from temp_dir to avoid repo CLAUDE.md",
    "read_when": [
      "about to invoke claude -p programmatically",
      "building a tool that shells out to Claude Code"
    ],
    "tripwires": [
      {{
        "action": "Invoking claude -p from inside a repo directory",
        "warning": "CLAUDE.md gets loaded as context; always use current_dir(temp_dir()) for non-interactive calls"
      }}
    ],
    "body": "One paragraph explaining WHY this matters and the cross-cutting context. No code blocks."
  }},
  {{
    "category": "patterns",
    "slug": "graphql-closed-event",
    "title": "Use GraphQL CLOSED_EVENT to find the PR that closed an issue",
    "read_when": [
      "implementing GitHub API integration",
      "looking up which PR closed a given issue"
    ],
    "tripwires": [],
    "body": "One paragraph explaining WHY this approach is better than alternatives."
  }}
]"#
    )
}

/// Runs `claude -p` and returns its trimmed output. It runs from a temp dir
/// so Claude Code doesn't pick up the repo's CLAUDE.md and act on it instead
/// of just answering.
pub fn run_prompt(prompt: &str) -> Result<String> {
    process::run_checked(
        Command::new("claude")
            .args(["-p", prompt, "--output-format", "text"])
            .current_dir(std::env::temp_dir()),
    )
    .context("running claude -p (is Claude Code installed and authenticated?)")
}

/// Runs `claude -p` (see [`run_prompt`]) and returns the JSON array in its output.
fn run_claude_for_json_array(prompt: &str) -> Result<String> {
    let text = run_prompt(prompt)?;

    // Strip markdown code fences (```json ... ``` or ``` ... ```) if present
    let stripped = strip_code_fence(&text);

    let json_start = stripped
        .find('[')
        .context("claude output contained no JSON array")?;
    let json_end = stripped
        .rfind(']')
        .context("claude output had no closing ]")?;
    Ok(stripped[json_start..=json_end].to_string())
}

pub fn synthesize_learnings(input: &SynthesisInput) -> Result<Vec<LearningItem>> {
    let json = run_claude_for_json_array(&build_synthesis_prompt(input))?;
    serde_json::from_str(&json).context("parsing learning items from claude output")
}

fn build_compact_prompt(topics: &[crate::memory::TopicFile], instructions: &str) -> String {
    let files_section: String = topics
        .iter()
        .map(|t| format!("### {}/{}\n{}\n", t.category, t.slug, t.content))
        .collect::<Vec<_>>()
        .join("\n");

    let instructions_section = always_loaded_section(instructions);

    format!(
        r#"You are aggressively pruning an AI agent memory system. The default action is DELETE. Only keep a file if it clears a high bar.

## Memory system reference
{COMPACT_QUALITY_GUIDE}

---
{instructions_section}
## The only reasons to KEEP a file

1. **It records a failure that already happened** — a bug, a wrong approach, a gotcha that was discovered the hard way. The file exists so the same mistake is not repeated.
2. **It captures non-obvious external behaviour** — something about a third-party tool, API, or environment that a developer could not infer from reading the source code (e.g. a CLI flag that silently does the wrong thing, an API with surprising semantics).
3. **It explains WHY an architectural decision was made** when the alternative was plausible and the reason is not in any source file or commit message — and getting it wrong would cause a real problem.

## DELETE everything else, including

- Any file that restates the always-loaded instructions (CLAUDE.md/AGENTS.md) — they are loaded every session already
- Any file that narrates how shipped code differs from an issue, spec, plan node or old tool — the code is the source of truth (titles or slugs with issue/node numbers are the usual smell)
- Any one-off process or coordination advice tied to a single event (e.g. the order to merge one particular set of PRs)
- Any file that describes HOW the code works (a developer can read the code)
- Any file that documents the current design or current command behaviour
- Any pattern that is the obvious/natural approach in Rust or in this codebase
- Any file whose insight is "use X" where X is already used everywhere in the code
- Any file that amounts to a reminder or note-to-self about a decision that is already baked in
- Any file describing a single-command or single-function implementation detail
- Any UX or output-formatting decision already visible in the code
- Anything where a competent developer reading the source for 10 minutes would say "yes, obviously"

## MERGE only when two files that both pass the KEEP bar cover the same root insight

Provide a `target_updated_body` paragraph that synthesises both. Merge targets must not themselves be merge_into sources.

## Files to audit

{files_section}

---

Return ONLY a JSON array. Every file must appear exactly once. Be aggressive — when in doubt, delete.

[
  {{"action": "keep", "category": "tripwires", "slug": "invoking-claude-p-from-within-a-repo-directory-causes-claude"}},
  {{"action": "delete", "category": "patterns", "slug": "compute-human-readable-issue-age-today-1-day-ago-n-days-ago-", "reason": "Describes a done implementation; self-evident from src/main.rs"}},
  {{"action": "merge_into", "category": "patterns", "slug": "source-slug",
    "target_category": "architecture", "target_slug": "target-slug",
    "target_updated_body": "Synthesised paragraph.",
    "reason": "Same root insight"}}
]"#
    )
}

/// `instructions` is the repo's CLAUDE.md/AGENTS.md text
/// (see `memory::read_always_loaded_instructions`); "" omits that section.
pub fn compact_learnings(
    topics: &[crate::memory::TopicFile],
    instructions: &str,
) -> Result<Vec<CompactAction>> {
    if topics.is_empty() {
        return Ok(Vec::new());
    }
    let json = run_claude_for_json_array(&build_compact_prompt(topics, instructions))?;
    serde_json::from_str(&json).context("parsing compact actions from claude output")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECTION_HEADING: &str = "## Always-loaded instructions (never restate)";

    fn input(instructions: &str) -> SynthesisInput<'_> {
        SynthesisInput {
            issue_title: "Issue title",
            issue_body: "Issue body",
            pr_title: "PR title",
            pr_body: "PR body",
            pr_diff: "diff --git a/x b/x",
            conversation: "",
            current_memory: "",
            prompt_hooks: "",
            instructions,
        }
    }

    #[test]
    fn strip_code_fence_bare_json() {
        assert_eq!(strip_code_fence("[1, 2]"), "[1, 2]");
    }

    #[test]
    fn strip_code_fence_json_fenced() {
        let input = "```json\n[1, 2]\n```";
        assert_eq!(strip_code_fence(input), "[1, 2]");
    }

    #[test]
    fn strip_code_fence_plain_fenced() {
        let input = "```\n{\"key\": \"val\"}\n```";
        assert_eq!(strip_code_fence(input), "{\"key\": \"val\"}");
    }

    #[test]
    fn strip_code_fence_multiline() {
        let input = "```json\nline1\nline2\n```";
        assert_eq!(strip_code_fence(input), "line1\nline2");
    }

    #[test]
    fn strip_code_fence_no_closing_fence() {
        let input = "```json\n[1, 2]";
        assert_eq!(strip_code_fence(input), "[1, 2]");
    }

    #[test]
    fn truncate_chars_respects_char_boundaries() {
        assert_eq!(truncate_chars("abc", 5), ("abc", false));
        assert_eq!(truncate_chars("abc", 3), ("abc", false));
        assert_eq!(truncate_chars("abcdef", 3), ("abc", true));
        // Multi-byte chars: a byte slice at 3 would panic mid-"é".
        assert_eq!(truncate_chars("ééé", 2), ("éé", true));
    }

    #[test]
    fn always_loaded_section_empty_when_no_instructions() {
        assert_eq!(always_loaded_section(""), "");
        assert_eq!(always_loaded_section("  \n\t"), "");
    }

    #[test]
    fn always_loaded_section_truncates_with_note() {
        let short = always_loaded_section("### CLAUDE.md\nUse anyhow.");
        assert!(short.contains(SECTION_HEADING));
        assert!(short.contains("Use anyhow."));
        assert!(!short.contains("truncated"));

        let long = "x".repeat(PROMPT_SECTION_CAP + 10);
        let section = always_loaded_section(&long);
        assert!(section.contains(&"x".repeat(PROMPT_SECTION_CAP)));
        assert!(!section.contains(&"x".repeat(PROMPT_SECTION_CAP + 1)));
        assert!(section.contains(&format!("truncated at {PROMPT_SECTION_CAP} chars")));
    }

    #[test]
    fn synthesis_prompt_includes_instructions_only_when_present() {
        let without = build_synthesis_prompt(&input(""));
        assert!(!without.contains(SECTION_HEADING));
        assert!(without.contains("Extract 0–2 learnings"));
        assert!(without.contains("Returning `[]` is expected"));

        let with = build_synthesis_prompt(&input("### CLAUDE.md\nNever use unwrap."));
        assert!(with.contains(SECTION_HEADING));
        assert!(with.contains("Never use unwrap."));
        // Instructions come before the issue so they frame the extraction.
        let section_at = with.find(SECTION_HEADING).unwrap();
        assert!(section_at < with.find("## Closed Issue: Issue title").unwrap());
    }

    #[test]
    fn compact_prompt_includes_instructions_only_when_present() {
        let topics = [crate::memory::TopicFile {
            category: "patterns".into(),
            slug: "foo".into(),
            content: "Foo body.".into(),
        }];
        let without = build_compact_prompt(&topics, "");
        assert!(!without.contains(SECTION_HEADING));
        assert!(without.contains("### patterns/foo\nFoo body."));
        assert!(without.contains("restates the always-loaded instructions"));
        assert!(without.contains("narrates how shipped code differs"));
        assert!(without.contains("one-off process or coordination advice"));

        let with = build_compact_prompt(&topics, "### AGENTS.md\nRun cargo test.");
        assert!(with.contains(SECTION_HEADING));
        assert!(with.contains("Run cargo test."));
    }

    #[test]
    fn synthesis_prompt_notes_truncation_only_when_cut() {
        let mut short = input("");
        short.conversation = "We agreed on X.";
        let prompt = build_synthesis_prompt(&short);
        assert!(prompt.contains("## Planning Conversation\nWe agreed on X."));
        assert!(!prompt.contains("truncated"));

        let long = "y".repeat(PROMPT_SECTION_CAP + 1);
        let mut cut = input("");
        cut.conversation = &long;
        assert!(build_synthesis_prompt(&cut).contains("truncated at"));
    }

    #[test]
    fn compact_kinds_parse_and_unknown_ones_fail() {
        let actions: Vec<CompactAction> = serde_json::from_str(
            r#"[{"action":"keep","category":"patterns","slug":"a"},
                {"action":"merge_into","category":"patterns","slug":"b","target_category":"patterns","target_slug":"a"}]"#,
        )
        .unwrap();
        assert_eq!(actions[0].action, CompactKind::Keep);
        assert_eq!(actions[1].action, CompactKind::MergeInto);
        let unknown = r#"[{"action":"archive","category":"patterns","slug":"a"}]"#;
        assert!(serde_json::from_str::<Vec<CompactAction>>(unknown).is_err());
    }

    #[test]
    fn prompts_embed_quality_guides() {
        let learn = build_synthesis_prompt(&input(""));
        assert!(learn.contains("## The extraction bar"));

        let compact = build_compact_prompt(&[], "");
        assert!(compact.contains("# engram memory guide"));
    }
}
