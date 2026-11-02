use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use crate::memory::{self, index};

/// Separator between the action and warning halves of a `--tripwire` value.
const TRIPWIRE_SEP: &str = "::";

/// A memory entry supplied directly by the caller — no issue, PR, or Claude synthesis.
#[derive(Debug, Default)]
pub struct Capture {
    pub category: String,
    pub slug: String,
    pub title: String,
    pub read_when: Vec<String>,
    pub tripwires: Vec<memory::Tripwire>,
    pub body: String,
    /// Extra frontmatter keys (e.g. the `origin` marker) written alongside the OKF fields.
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
}

impl Capture {
    /// The entry `engram capture` describes: the slug defaults to the title's,
    /// and the body to stdin when `--body` is absent.
    pub fn from_args(args: crate::cli::CaptureArgs) -> Result<Self> {
        let slug = args.slug.unwrap_or_else(|| slugify(&args.title));
        let mut entry = Capture {
            category: args.category,
            slug,
            title: args.title,
            read_when: args.read_when,
            tripwires: args.tripwires,
            body: resolve_body(args.body)?,
            extra: BTreeMap::new(),
        };
        if args.session {
            entry.mark_session();
        }
        Ok(entry)
    }

    /// Tags this entry as a mid-session observation (`origin: session`) so
    /// `compact` can tell it apart from `learn`-synthesized memory.
    pub fn mark_session(&mut self) {
        self.extra.insert(
            memory::ORIGIN_KEY.to_string(),
            serde_yaml_ng::Value::String(memory::ORIGIN_SESSION.to_string()),
        );
    }

    /// The `origin` marker, if one was set.
    pub fn origin(&self) -> Option<&str> {
        self.extra.get(memory::ORIGIN_KEY).and_then(|v| v.as_str())
    }
}

/// Lowercase, ASCII-alphanumeric words joined by `-`.
pub fn slugify(s: &str) -> String {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

fn validate_slug(slug: &str) -> Result<()> {
    let valid = !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        anyhow::bail!("invalid slug {slug:?} (use lowercase letters, digits, and '-')");
    }
    Ok(())
}

/// Parses `ACTION::WARNING` into a tripwire.
pub fn parse_tripwire(s: &str) -> Result<memory::Tripwire> {
    let (action, warning) = s.split_once(TRIPWIRE_SEP).with_context(|| {
        format!("tripwire {s:?} must be formatted as ACTION{TRIPWIRE_SEP}WARNING")
    })?;
    let (action, warning) = (action.trim(), warning.trim());
    if action.is_empty() || warning.is_empty() {
        anyhow::bail!("tripwire {s:?} needs both an action and a warning");
    }
    Ok(memory::Tripwire {
        action: action.to_string(),
        warning: warning.to_string(),
    })
}

/// Writes `capture` to `.engram/memory/<category>/<slug>.md` with the same
/// OKF frontmatter `learn` produces, then refreshes the index and CLAUDE.md
/// section. Refuses to overwrite an existing file. Makes no GitHub calls.
pub fn write(repo_root: &Path, capture: &Capture) -> Result<PathBuf> {
    memory::validate_category(&capture.category)?;
    validate_slug(&capture.slug)?;
    if capture.title.trim().is_empty() {
        anyhow::bail!("title must not be empty");
    }
    if capture.body.trim().is_empty() {
        anyhow::bail!("body must not be empty");
    }

    let cat_dir = memory::memory_dir(repo_root).join(&capture.category);
    let path = cat_dir.join(format!("{}.md", capture.slug));
    if path.exists() {
        anyhow::bail!(
            "{} already exists — pick a different --slug",
            path.display()
        );
    }

    let mut fm = memory::Frontmatter::new(&capture.category, &capture.title);
    fm.read_when.clone_from(&capture.read_when);
    fm.tripwires.clone_from(&capture.tripwires);
    fm.extra = capture.extra.clone();
    fm.touch();
    let content = memory::render(&fm, &capture.body)?;

    std::fs::create_dir_all(&cat_dir).with_context(|| format!("creating {}", cat_dir.display()))?;
    std::fs::write(&path, &content).with_context(|| format!("writing {}", path.display()))?;

    memory::rebuild_index(repo_root).context("rebuilding memory index")?;
    memory::write_claude_md_section(repo_root).context("updating CLAUDE.md")?;
    // `search` only rebuilds the FTS index when it's missing, so keep an existing one current.
    if index::exists(repo_root) {
        index::upsert_file(repo_root, &capture.category, &capture.slug, &content)
            .context("updating search index")?;
    }

    Ok(path)
}

/// Returns `body` if given, otherwise reads it from piped stdin.
fn resolve_body(body: Option<String>) -> Result<String> {
    if let Some(body) = body {
        return Ok(body);
    }
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        anyhow::bail!("no --body given and stdin is a terminal — pass --body or pipe the body in");
    }
    let mut buf = String::new();
    stdin
        .read_to_string(&mut buf)
        .context("reading body from stdin")?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(category: &str, slug: &str) -> Capture {
        Capture {
            category: category.to_string(),
            slug: slug.to_string(),
            title: "Use bar".to_string(),
            read_when: vec!["when calling foo".to_string()],
            tripwires: vec![parse_tripwire("Calling foo() :: Use bar()").unwrap()],
            body: "Prefer bar over foo.".to_string(),
            extra: BTreeMap::new(),
        }
    }

    fn origin_of(fm: &memory::Frontmatter) -> Option<&str> {
        fm.extra.get(memory::ORIGIN_KEY).and_then(|v| v.as_str())
    }

    #[test]
    fn slugify_normalizes_titles() {
        assert_eq!(slugify("Don't mock the DB!"), "don-t-mock-the-db");
        assert_eq!(slugify("  a  b  "), "a-b");
    }

    #[test]
    fn parse_tripwire_requires_both_halves() {
        let t = parse_tripwire("Calling foo() :: Use bar()").unwrap();
        assert_eq!(
            (t.action.as_str(), t.warning.as_str()),
            ("Calling foo()", "Use bar()")
        );
        assert!(parse_tripwire("no separator").is_err());
        assert!(parse_tripwire(":: warning only").is_err());
    }

    #[test]
    fn write_produces_learn_compatible_file_and_index_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut c = capture("tripwires", "use-bar");
        c.extra.insert(
            "source".into(),
            serde_yaml_ng::Value::String("session".into()),
        );

        let path = write(root, &c).unwrap();
        assert_eq!(path, root.join(".engram/memory/tripwires/use-bar.md"));

        let (fm, body) =
            memory::split_frontmatter(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(fm.kind, "tripwire");
        assert_eq!(fm.title, "Use bar");
        assert_eq!(fm.read_when, ["when calling foo"]);
        assert_eq!(fm.tripwires[0].warning, "Use bar()");
        assert_eq!(fm.generated.unwrap().at, crate::date::today_iso());
        assert!(fm.sources.is_empty());
        assert_eq!(fm.extra.get("source"), c.extra.get("source"));
        assert_eq!(body.trim(), "Prefer bar over foo.");

        let index = std::fs::read_to_string(root.join(".engram/memory/index.md")).unwrap();
        assert!(index.contains("[tripwires/use-bar.md](./tripwires/use-bar.md)"));
        assert!(std::fs::read_to_string(root.join("CLAUDE.md"))
            .unwrap()
            .contains("@.engram/memory/index.md"));
    }

    #[test]
    fn session_capture_is_marked_unlike_plain_capture() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let plain = write(root, &capture("patterns", "plain")).unwrap();
        let mut c = capture("patterns", "observed");
        c.mark_session();
        let session = write(root, &c).unwrap();

        let raw = std::fs::read_to_string(&session).unwrap();
        assert!(raw.contains("origin: session"), "{raw}");
        let fm_of = |p: &Path| {
            memory::split_frontmatter(&std::fs::read_to_string(p).unwrap())
                .unwrap()
                .0
        };
        assert_eq!(origin_of(&fm_of(&session)), Some(memory::ORIGIN_SESSION));
        assert_eq!(origin_of(&fm_of(&plain)), None);
    }

    #[test]
    fn learn_rewrite_clears_session_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut c = capture("patterns", "observed");
        c.mark_session();
        let path = write(root, &c).unwrap();

        let item = crate::integrations::claude::LearningItem {
            category: "patterns".to_string(),
            slug: "observed".to_string(),
            title: "Synthesized".to_string(),
            read_when: vec![],
            tripwires: vec![],
            body: "New body.".to_string(),
        };
        memory::write_topic_file(root, "acme/widgets", &item, 7).unwrap();
        let (fm, _) = memory::split_frontmatter(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(origin_of(&fm), None);
        assert_eq!(fm.sources.len(), 1);
    }

    #[test]
    fn write_updates_existing_search_index() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, &capture("patterns", "first")).unwrap();
        index::rebuild_index(root).unwrap();

        let mut second = capture("patterns", "second");
        second.body = "Zanzibar quokka.".to_string();
        write(root, &second).unwrap();

        let hits = index::search(root, "quokka").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, ".engram/memory/patterns/second.md");
    }

    #[test]
    fn write_rejects_bad_input_and_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(write(root, &capture("notes", "x")).is_err());
        assert!(write(root, &capture("patterns", "../escape")).is_err());
        let mut empty = capture("patterns", "empty");
        empty.body = "  \n".to_string();
        assert!(write(root, &empty).is_err());

        write(root, &capture("patterns", "dup")).unwrap();
        assert!(write(root, &capture("patterns", "dup")).is_err());
        assert!(!root.join(".engram/memory/notes").exists());
    }
}
