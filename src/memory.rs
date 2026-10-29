use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use crate::claude::{CompactAction, CompactKind};
use crate::date::today_iso;
use crate::github;

/// Repo-relative directory holding every memory topic file.
pub const MEMORY_DIR: &str = ".engram/memory";
/// The instruction file engram keeps its managed memory section in.
pub const CLAUDE_MD: &str = "CLAUDE.md";
const ENGRAM_START: &str = "<!-- engram:start -->";
const ENGRAM_END: &str = "<!-- engram:end -->";
/// Instruction files every agent session loads; learnings must never restate them.
const INSTRUCTION_FILES: &[&str] = &[CLAUDE_MD, "AGENTS.md"];

/// Memory categories and their OKF v0.2 `type` value.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("patterns", "pattern"),
    ("tripwires", "tripwire"),
    ("architecture", "architecture"),
    ("testing", "testing"),
];

/// Frontmatter key marking how a topic file was written. Absent on
/// `learn`-synthesized files; `compact` sees it in the raw file content.
pub const ORIGIN_KEY: &str = "origin";
/// `origin` value for observations an agent recorded mid-session.
pub const ORIGIN_SESSION: &str = "session";

/// `<repo_root>/.engram/memory`.
pub fn memory_dir(repo_root: &Path) -> PathBuf {
    repo_root.join(MEMORY_DIR)
}

/// Repo-relative path of a topic file: `.engram/memory/<category>/<slug>.md`.
pub fn topic_rel_path(category: &str, slug: &str) -> String {
    format!("{MEMORY_DIR}/{category}/{slug}.md")
}

fn okf_type_for(category: &str) -> &'static str {
    CATEGORIES
        .iter()
        .find(|(c, _)| *c == category)
        .map_or("pattern", |(_, t)| *t)
}

/// Rejects categories outside `CATEGORIES`; files written elsewhere are never indexed or read.
pub fn validate_category(category: &str) -> Result<()> {
    if CATEGORIES.iter().any(|(c, _)| *c == category) {
        return Ok(());
    }
    let allowed: Vec<_> = CATEGORIES.iter().map(|(c, _)| *c).collect();
    anyhow::bail!(
        "unknown memory category {category:?} (expected one of: {})",
        allowed.join(", ")
    )
}

fn generator() -> String {
    format!("engram/{}", env!("CARGO_PKG_VERSION"))
}

fn table_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq, Eq)]
pub struct Tripwire {
    pub action: String,
    pub warning: String,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq, Eq)]
pub struct Generated {
    pub by: String,
    pub at: String,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub resource: String,
}

/// OKF v0.2 frontmatter for a memory topic file. `last_updated` and
/// `source_issues` are accepted on read for one release (pre-migration
/// files) but are never written back — `normalize` folds them into
/// `generated`/`sources` and clears them.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Frontmatter {
    #[serde(rename = "type", default)]
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub read_when: Vec<String>,
    #[serde(default)]
    pub tripwires: Vec<Tripwire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<Generated>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,

    #[serde(default, skip_serializing)]
    pub last_updated: Option<String>,
    #[serde(default, skip_serializing, rename = "source_issues")]
    pub legacy_source_issues: Option<Vec<u64>>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
}

impl Frontmatter {
    pub fn new(category: &str, title: &str) -> Self {
        Self {
            kind: okf_type_for(category).to_string(),
            title: title.to_string(),
            read_when: Vec::new(),
            tripwires: Vec::new(),
            generated: None,
            sources: Vec::new(),
            last_updated: None,
            legacy_source_issues: None,
            extra: BTreeMap::new(),
        }
    }

    /// Fills in `type`/`generated`/`sources` from legacy fields or defaults,
    /// then clears the legacy fields so they are never re-serialized.
    pub fn normalize(&mut self, category: &str, repo: &str) {
        if self.kind.is_empty() {
            self.kind = okf_type_for(category).to_string();
        }
        if self.generated.is_none() {
            let at = self.last_updated.clone().unwrap_or_else(today_iso);
            self.generated = Some(Generated {
                by: generator(),
                at,
            });
        }
        if self.sources.is_empty() {
            if let Some(ids) = self.legacy_source_issues.take() {
                self.sources = ids
                    .into_iter()
                    .map(|n| Source {
                        id: n.to_string(),
                        resource: github::issue_url(repo, n),
                    })
                    .collect();
            }
        }
        self.last_updated = None;
        self.legacy_source_issues = None;
    }

    /// Stamps `generated` with this binary and today's date; call on every content rewrite.
    pub fn touch(&mut self) {
        self.generated = Some(Generated {
            by: generator(),
            at: today_iso(),
        });
    }

    /// Adds a source for `issue_number`, deduping on `id`.
    pub fn add_source(&mut self, repo: &str, issue_number: u64) {
        let id = issue_number.to_string();
        if !self.sources.iter().any(|s| s.id == id) {
            self.sources.push(Source {
                id,
                resource: github::issue_url(repo, issue_number),
            });
        }
    }

    /// Merges in sources from another file's frontmatter, deduping on `id`.
    pub fn merge_sources(&mut self, other: Vec<Source>) {
        for s in other {
            if !self.sources.iter().any(|existing| existing.id == s.id) {
                self.sources.push(s);
            }
        }
    }
}

/// Splits a memory file into (YAML frontmatter text, body). The returned
/// `yaml` text excludes the `---` fences; `body` excludes the blank line
/// that follows the closing fence.
fn split_frontmatter_text(content: &str) -> Result<(&str, &str)> {
    let first_line_end = content.find('\n').unwrap_or(content.len());
    if content[..first_line_end].trim() != "---" {
        anyhow::bail!("missing opening --- frontmatter delimiter");
    }
    let yaml_start = (first_line_end + 1).min(content.len());

    let mut offset = yaml_start;
    loop {
        let line_end = content[offset..]
            .find('\n')
            .map_or(content.len(), |p| offset + p);
        let line = &content[offset..line_end];
        if line.trim() == "---" {
            let yaml_text = &content[yaml_start..offset];
            let body_start = (line_end + 1).min(content.len());
            return Ok((yaml_text, &content[body_start..]));
        }
        if line_end == content.len() {
            anyhow::bail!("missing closing --- frontmatter delimiter");
        }
        offset = line_end + 1;
    }
}

/// Parses a memory file into its typed frontmatter and body.
pub fn split_frontmatter(content: &str) -> Result<(Frontmatter, String)> {
    let (yaml_text, body) = split_frontmatter_text(content)?;
    let fm: Frontmatter =
        serde_yaml_ng::from_str(yaml_text).context("parsing memory file frontmatter")?;
    Ok((fm, body.trim_start_matches('\n').to_string()))
}

/// Renders typed frontmatter and a body back into a memory file.
pub fn render(fm: &Frontmatter, body: &str) -> Result<String> {
    let yaml = serde_yaml_ng::to_string(fm).context("serializing memory file frontmatter")?;
    let yaml = yaml.strip_prefix("---\n").unwrap_or(&yaml);
    let body = body.trim_end_matches('\n');
    if body.is_empty() {
        Ok(format!("---\n{yaml}---\n"))
    } else {
        Ok(format!("---\n{yaml}---\n\n{body}\n"))
    }
}

pub struct TopicFile {
    pub category: String,
    pub slug: String,
    pub content: String,
}

fn category_dirs(
    memory_dir: &Path,
) -> impl Iterator<Item = (&'static str, std::path::PathBuf)> + use<'_> {
    CATEGORIES
        .iter()
        .map(move |(c, _)| (*c, memory_dir.join(c)))
}

fn sorted_md_files(cat_dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    if !cat_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<_> = std::fs::read_dir(cat_dir)?
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();
    files.sort();
    Ok(files)
}

fn slug_of(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

pub fn list_all_topics(repo_root: &Path) -> Result<Vec<TopicFile>> {
    let mut topics = Vec::new();
    for (category, cat_dir) in category_dirs(&memory_dir(repo_root)) {
        for path in sorted_md_files(&cat_dir)? {
            let content = std::fs::read_to_string(&path)?;
            topics.push(TopicFile {
                category: category.to_string(),
                slug: slug_of(&path),
                content,
            });
        }
    }
    Ok(topics)
}

/// Applies `compact`'s verdicts: merges first, then deletions. A merge source
/// is only deleted once its merge landed, so a missing or malformed target
/// never loses content. Returns (deleted, merged).
pub fn apply_compact_actions(
    repo_root: &Path,
    repo: &str,
    actions: &[CompactAction],
    topics: &[TopicFile],
) -> Result<(usize, usize)> {
    let memory_dir = memory_dir(repo_root);
    let mut merged_sources = std::collections::HashSet::new();
    for action in actions {
        if let CompactKind::MergeInto = action.action {
            if merge_into_target(&memory_dir, repo, action, topics)? {
                merged_sources.insert((action.category.as_str(), action.slug.as_str()));
            }
        }
    }

    let (mut deleted, mut merged) = (0, 0);
    for action in actions {
        let counter = match action.action {
            CompactKind::Delete => &mut deleted,
            CompactKind::MergeInto
                if merged_sources.contains(&(action.category.as_str(), action.slug.as_str())) =>
            {
                &mut merged
            }
            _ => continue,
        };
        let path = memory_dir
            .join(&action.category)
            .join(format!("{}.md", action.slug));
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            *counter += 1;
        }
    }
    Ok((deleted, merged))
}

/// Folds a `merge_into` source's sources (and the merged body) into its
/// target. Returns false, changing nothing, when the target is missing,
/// malformed or not named.
fn merge_into_target(
    memory_dir: &Path,
    repo: &str,
    action: &CompactAction,
    topics: &[TopicFile],
) -> Result<bool> {
    let (Some(target_cat), Some(target_slug)) = (&action.target_category, &action.target_slug)
    else {
        return Ok(false);
    };
    let target_path = memory_dir
        .join(target_cat)
        .join(format!("{target_slug}.md"));
    if !target_path.exists() {
        return Ok(false);
    }

    let existing = std::fs::read_to_string(&target_path)
        .with_context(|| format!("reading {}", target_path.display()))?;
    let (mut fm, existing_body) = match split_frontmatter(&existing) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!(
                "warning: not merging {}/{} — target {} is malformed: {e:#}",
                action.category,
                action.slug,
                target_path.display()
            );
            return Ok(false);
        }
    };
    let source_entries: Vec<Source> = topics
        .iter()
        .find(|t| t.category == action.category && t.slug == action.slug)
        .and_then(|f| split_frontmatter(&f.content).ok())
        .map(|(mut source_fm, _)| {
            source_fm.normalize(&action.category, repo);
            source_fm.sources
        })
        .unwrap_or_default();
    fm.normalize(target_cat, repo);
    fm.merge_sources(source_entries);
    fm.touch();
    let body = action
        .target_updated_body
        .as_deref()
        .unwrap_or(&existing_body);
    std::fs::write(&target_path, render(&fm, body)?)
        .with_context(|| format!("writing {}", target_path.display()))?;
    Ok(true)
}

/// Topic files matching `permalink`: the file at that path under the memory
/// directory if there is one, else every topic whose slug is the path's stem.
/// Paths that would leave the memory directory never match.
pub fn find_topics(repo_root: &Path, permalink: &str) -> Result<Vec<PathBuf>> {
    let memory_dir = memory_dir(repo_root);
    let requested = Path::new(permalink);
    let inside = requested
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if inside && memory_dir.join(requested).is_file() {
        return Ok(vec![memory_dir.join(requested)]);
    }
    let needle = requested.file_stem().map_or_else(
        || permalink.to_string(),
        |s| s.to_string_lossy().to_string(),
    );
    let mut matches = Vec::new();
    for (_, cat_dir) in category_dirs(&memory_dir) {
        for path in sorted_md_files(&cat_dir)? {
            if slug_of(&path) == needle {
                matches.push(path);
            }
        }
    }
    Ok(matches)
}

/// Read all memory files for injection into the synthesis prompt.
/// Returns a compact summary of existing topics so Claude avoids duplicates.
pub fn read_all(repo_root: &Path) -> Result<String> {
    let memory_dir = memory_dir(repo_root);
    if !memory_dir.exists() {
        return Ok(String::new());
    }

    let mut all = String::new();
    for (category, cat_dir) in category_dirs(&memory_dir) {
        for path in sorted_md_files(&cat_dir)? {
            let content = std::fs::read_to_string(&path)?;
            let slug = slug_of(&path);
            all.push_str(&format!("### [{category}/{slug}]\n{content}\n\n"));
        }
    }
    Ok(all)
}

/// Write a single learning item to .engram/memory/<category>/<slug>.md
pub fn write_topic_file(
    repo_root: &Path,
    repo: &str,
    item: &crate::claude::LearningItem,
    issue_number: u64,
) -> Result<()> {
    let category = &item.category;
    let slug = &item.slug;
    validate_category(category)?;
    let cat_dir = memory_dir(repo_root).join(category);
    std::fs::create_dir_all(&cat_dir).with_context(|| format!("creating {}", cat_dir.display()))?;

    let path = cat_dir.join(format!("{slug}.md"));

    let mut fm = if path.exists() {
        let existing = std::fs::read_to_string(&path)?;
        match split_frontmatter(&existing) {
            Ok((mut fm, _)) => {
                fm.normalize(category, repo);
                fm
            }
            Err(e) => {
                eprintln!(
                    "warning: {} has malformed frontmatter ({e:#}); rewriting it and dropping its previous sources",
                    path.display()
                );
                Frontmatter::new(category, &item.title)
            }
        }
    } else {
        Frontmatter::new(category, &item.title)
    };

    fm.title.clone_from(&item.title);
    fm.read_when.clone_from(&item.read_when);
    fm.tripwires = item
        .tripwires
        .iter()
        .map(|t| Tripwire {
            action: t.action.clone(),
            warning: t.warning.clone(),
        })
        .collect();
    fm.add_source(repo, issue_number);
    // The body is now learn-synthesized, so any capture-time marker no longer applies.
    fm.extra.remove(ORIGIN_KEY);
    fm.touch();

    let content = render(&fm, &item.body)?;
    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))
}

/// Rewrites every topic file's frontmatter into OKF v0.2 shape, accepting
/// legacy `last_updated`/`source_issues` fields. Files with malformed YAML
/// are reported and left untouched. Returns (processed, skipped).
pub fn migrate_all(repo_root: &Path, repo: &str) -> Result<(usize, usize)> {
    let mut processed = 0;
    let mut skipped = 0;

    for (category, cat_dir) in category_dirs(&memory_dir(repo_root)) {
        for path in sorted_md_files(&cat_dir)? {
            let content = std::fs::read_to_string(&path)?;
            match split_frontmatter(&content) {
                Ok((mut fm, body)) => {
                    fm.normalize(category, repo);
                    let rendered = render(&fm, &body)?;
                    if rendered != content {
                        std::fs::write(&path, rendered)?;
                    }
                    processed += 1;
                }
                Err(e) => {
                    eprintln!("warning: skipping {} — {e:#}", path.display());
                    skipped += 1;
                }
            }
        }
    }

    Ok((processed, skipped))
}

/// Auto-generate .engram/memory/index.md as an OKF v0.2 routing table for agents.
/// Links are relative `./<category>/<slug>.md` paths — never `@`-prefixed —
/// so the index itself stays eager-loadable without pulling in every topic file.
pub fn rebuild_index(repo_root: &Path) -> Result<()> {
    let memory_dir = memory_dir(repo_root);
    if !memory_dir.exists() {
        return Ok(());
    }

    let mut rows = Vec::new();

    for (category, cat_dir) in category_dirs(&memory_dir) {
        for path in sorted_md_files(&cat_dir)? {
            let content = std::fs::read_to_string(&path)?;
            let slug = slug_of(&path);
            let (title, read_when) = match split_frontmatter(&content) {
                Ok((fm, _)) => (fm.title, fm.read_when),
                Err(_) => (slug.clone(), Vec::new()),
            };
            let read_when = if read_when.is_empty() {
                "—".to_string()
            } else {
                read_when.join("; ")
            };
            rows.push(format!(
                "| [{category}/{slug}.md](./{category}/{slug}.md) | {} | {} |",
                table_cell(&title),
                table_cell(&read_when)
            ));
        }
    }

    let body = if rows.is_empty() {
        "_No learnings yet._\n".to_string()
    } else {
        format!(
            "| File | Title | Read when |\n|------|-------|-----------|\n{}\n",
            rows.join("\n")
        )
    };

    let content = format!(
        "---\nokf_version: \"0.2\"\n---\n\n# Engram Memory Index\n\nAgents: read this index to find relevant learned docs. Links are relative to `.engram/memory/`. Load individual files only when their \"Read when\" condition matches your current task.\n\n{body}"
    );

    std::fs::write(memory_dir.join("index.md"), content)?;
    Ok(())
}

pub fn write_claude_md_section(repo_root: &Path) -> Result<()> {
    let claude_md_path = repo_root.join(CLAUDE_MD);
    let existing = if claude_md_path.exists() {
        std::fs::read_to_string(&claude_md_path)
            .with_context(|| format!("reading {}", claude_md_path.display()))?
    } else {
        String::new()
    };
    let has_index = memory_dir(repo_root).join("index.md").exists();
    std::fs::write(&claude_md_path, with_engram_section(&existing, has_index))
        .with_context(|| format!("writing {}", claude_md_path.display()))
}

/// `text` with engram's managed section replaced, or appended when it has none.
fn with_engram_section(text: &str, has_index: bool) -> String {
    let body = if has_index {
        format!("@{MEMORY_DIR}/index.md\n")
    } else {
        "_No learnings yet. Run `engram plan learn <issue>` after closing a plan._\n".to_string()
    };
    let section = format!("{ENGRAM_START}\n## Engram Memory\n\n{body}\n{ENGRAM_END}");
    match engram_section_range(text) {
        Some(range) => format!("{}{section}{}", &text[..range.start], &text[range.end..]),
        None if text.is_empty() => section,
        None => format!("{}\n\n{section}", text.trim_end()),
    }
}

/// Byte range of engram's managed section (`ENGRAM_START`..`ENGRAM_END`,
/// markers included), when both markers are there in that order.
fn engram_section_range(text: &str) -> Option<Range<usize>> {
    let start = text.find(ENGRAM_START)?;
    let end = text.rfind(ENGRAM_END).filter(|&end| end >= start)?;
    Some(start..end + ENGRAM_END.len())
}

/// Removes engram's own managed section (`ENGRAM_START`..`ENGRAM_END`, markers
/// included) so synthesis sees only the human-written instructions.
pub fn strip_engram_section(text: &str) -> String {
    match engram_section_range(text) {
        Some(range) => format!("{}{}", &text[..range.start], &text[range.end..]),
        None => text.to_string(),
    }
}

/// Joins the repo's always-loaded instruction files (`INSTRUCTION_FILES`), each
/// under a `### <name>` heading, with engram's managed section stripped. Missing
/// or empty files are skipped, as is a file identical to one already included
/// (e.g. CLAUDE.md symlinked to AGENTS.md). Returns "" when there is nothing.
pub fn read_always_loaded_instructions(repo_root: &Path) -> Result<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut sections = Vec::new();
    for name in INSTRUCTION_FILES {
        let path = repo_root.join(name);
        if !path.exists() {
            continue;
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let text = strip_engram_section(&raw).trim().to_string();
        if text.is_empty() || seen.contains(&text) {
            continue;
        }
        sections.push(format!("### {name}\n{text}"));
        seen.push(text);
    }
    Ok(sections.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::{LearningItem, Tripwire as ClaudeTripwire};

    const REPO: &str = "acme/widgets";
    const LEGACY: &str = "---\ntitle: \"Legacy\"\nread_when: []\ntripwires: []\nlast_updated: \"2024-01-01\"\nsource_issues: [1]\n---\n\nOld body.\n";

    fn item(slug: &str, title: &str) -> LearningItem {
        LearningItem {
            category: "patterns".to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            read_when: vec!["when doing X".to_string()],
            tripwires: vec![],
            body: "Body text.".to_string(),
        }
    }

    fn read_fm(path: &Path) -> (Frontmatter, String) {
        split_frontmatter(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn render_round_trips_special_characters_and_unknown_keys() {
        let tricky = "A \"quoted\" title: with # hash";
        let mut fm = Frontmatter::new("patterns", tricky);
        fm.read_when = vec![tricky.to_string()];
        fm.extra
            .insert("custom".into(), serde_yaml_ng::Value::String("keep".into()));

        let (parsed, body) = split_frontmatter(&render(&fm, "Body.").unwrap()).unwrap();
        assert_eq!(parsed.title, tricky);
        assert_eq!(parsed.read_when, [tricky]);
        assert_eq!(parsed.extra.get("custom"), fm.extra.get("custom"));
        assert_eq!(body.trim(), "Body.");
    }

    #[test]
    fn write_topic_file_accumulates_deduped_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mut it = item("slug", "Title");
        it.tripwires = vec![ClaudeTripwire {
            action: "Calling foo()".into(),
            warning: "Use bar()".into(),
        }];
        for n in [1, 1, 2] {
            write_topic_file(dir.path(), REPO, &it, n).unwrap();
        }

        let (fm, body) = read_fm(&dir.path().join(".engram/memory/patterns/slug.md"));
        assert_eq!(fm.kind, "pattern");
        assert_eq!(fm.tripwires[0].warning, "Use bar()");
        assert_eq!(fm.generated.unwrap().at, today_iso());
        let ids: Vec<_> = fm.sources.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["1", "2"]);
        assert_eq!(
            fm.sources[0].resource,
            format!("https://github.com/{REPO}/issues/1")
        );
        assert_eq!(body.trim(), "Body text.");
    }

    #[test]
    fn rebuild_index_writes_okf_index_without_imports() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let index_path = root.join(".engram/memory/index.md");
        std::fs::create_dir_all(root.join(".engram/memory")).unwrap();
        rebuild_index(root).unwrap();
        assert!(std::fs::read_to_string(&index_path)
            .unwrap()
            .contains("No learnings yet"));

        write_topic_file(root, REPO, &item("b", "Use a | b"), 1).unwrap();
        rebuild_index(root).unwrap();
        let index = std::fs::read_to_string(&index_path).unwrap();
        assert!(index.starts_with("---\nokf_version: \"0.2\"\n---"));
        assert!(!index.contains('@'));
        assert!(index.contains("| [patterns/b.md](./patterns/b.md) | Use a \\| b | when doing X |"));
    }

    #[test]
    fn migrate_normalizes_legacy_files_idempotently_and_skips_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let cat_dir = dir.path().join(".engram/memory/tripwires");
        std::fs::create_dir_all(&cat_dir).unwrap();
        std::fs::write(cat_dir.join("legacy.md"), LEGACY).unwrap();
        std::fs::write(cat_dir.join("broken.md"), "no frontmatter\n").unwrap();

        assert_eq!(migrate_all(dir.path(), REPO).unwrap(), (1, 1));
        let first = std::fs::read_to_string(cat_dir.join("legacy.md")).unwrap();
        assert!(!first.contains("last_updated") && !first.contains("source_issues"));
        let (fm, _) = split_frontmatter(&first).unwrap();
        assert_eq!(fm.kind, "tripwire");
        assert_eq!(fm.generated.unwrap().at, "2024-01-01");
        assert_eq!(fm.sources[0].id, "1");

        migrate_all(dir.path(), REPO).unwrap();
        assert_eq!(
            std::fs::read_to_string(cat_dir.join("legacy.md")).unwrap(),
            first
        );
        assert_eq!(
            std::fs::read_to_string(cat_dir.join("broken.md")).unwrap(),
            "no frontmatter\n"
        );
    }

    #[test]
    fn compact_merge_only_deletes_sources_that_landed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cat_dir = root.join(".engram/memory/patterns");
        std::fs::create_dir_all(&cat_dir).unwrap();
        std::fs::write(cat_dir.join("target.md"), LEGACY).unwrap();
        std::fs::write(cat_dir.join("broken.md"), "no frontmatter\n").unwrap();
        for (slug, n) in [("a", 2), ("b", 3), ("c", 4)] {
            write_topic_file(root, REPO, &item(slug, slug), n).unwrap();
        }
        let merge = |slug: &str, target: &str| CompactAction {
            action: CompactKind::MergeInto,
            category: "patterns".into(),
            slug: slug.into(),
            reason: None,
            target_category: Some("patterns".into()),
            target_slug: Some(target.into()),
            target_updated_body: Some("Merged.".into()),
        };
        let actions = [
            merge("a", "target"),
            merge("b", "broken"),
            merge("c", "missing"),
        ];

        let topics = list_all_topics(root).unwrap();
        assert_eq!(
            apply_compact_actions(root, REPO, &actions, &topics).unwrap(),
            (0, 1)
        );
        assert!(!cat_dir.join("a.md").exists());
        assert!(cat_dir.join("b.md").exists() && cat_dir.join("c.md").exists());

        let (fm, body) = read_fm(&cat_dir.join("target.md"));
        let ids: Vec<_> = fm.sources.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["1", "2"]);
        assert_eq!(fm.generated.unwrap().at, today_iso());
        assert_eq!(body.trim(), "Merged.");
    }

    #[test]
    fn strip_engram_section_removes_managed_block_only() {
        let text = "# Repo\n\nRule A.\n\n<!-- engram:start -->\n## Engram Memory\n\n@.engram/memory/index.md\n<!-- engram:end -->\n\nRule B.\n";
        assert_eq!(
            strip_engram_section(text),
            "# Repo\n\nRule A.\n\n\n\nRule B.\n"
        );
        assert_eq!(strip_engram_section("no markers"), "no markers");
        let reversed = "<!-- engram:end --> x <!-- engram:start -->";
        assert_eq!(strip_engram_section(reversed), reversed);
    }

    #[test]
    fn read_always_loaded_instructions_joins_strips_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(read_always_loaded_instructions(root).unwrap(), "");

        std::fs::write(
            root.join("CLAUDE.md"),
            "Use anyhow.\n<!-- engram:start -->\n@.engram/memory/index.md\n<!-- engram:end -->\n",
        )
        .unwrap();
        std::fs::write(root.join("AGENTS.md"), "Use anyhow.\n").unwrap();
        assert_eq!(
            read_always_loaded_instructions(root).unwrap(),
            "### CLAUDE.md\nUse anyhow."
        );

        std::fs::write(root.join("AGENTS.md"), "Run cargo test.\n").unwrap();
        assert_eq!(
            read_always_loaded_instructions(root).unwrap(),
            "### CLAUDE.md\nUse anyhow.\n\n### AGENTS.md\nRun cargo test."
        );

        std::fs::write(
            root.join("CLAUDE.md"),
            "<!-- engram:start -->\nonly managed\n<!-- engram:end -->\n",
        )
        .unwrap();
        assert_eq!(
            read_always_loaded_instructions(root).unwrap(),
            "### AGENTS.md\nRun cargo test."
        );
    }

    #[test]
    fn with_engram_section_replaces_or_appends_but_never_duplicates() {
        let fresh = with_engram_section("", true);
        assert!(fresh.starts_with(ENGRAM_START) && fresh.contains("@.engram/memory/index.md"));

        let replaced = with_engram_section(&format!("# Repo\n\n{fresh}\n\nTail.\n"), false);
        assert_eq!(replaced.matches(ENGRAM_START).count(), 1);
        assert!(replaced.contains("engram plan learn <issue>"));
        assert!(replaced.starts_with("# Repo\n\n") && replaced.ends_with("\n\nTail.\n"));

        // Reversed markers are not a section: append one, keep the stray markers.
        let reversed = "<!-- engram:end --> x <!-- engram:start -->";
        let appended = with_engram_section(reversed, true);
        assert!(appended.starts_with(reversed));
        assert_eq!(appended.matches(ENGRAM_START).count(), 2);
    }

    #[test]
    fn find_topics_by_path_or_slug_and_stays_inside_memory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_topic_file(root, REPO, &item("foo", "Foo"), 1).unwrap();
        let mut tripwire = item("foo", "Foo too");
        tripwire.category = "tripwires".into();
        write_topic_file(root, REPO, &tripwire, 1).unwrap();
        std::fs::write(root.join("secret.md"), "outside").unwrap();

        let exact = find_topics(root, "patterns/foo.md").unwrap();
        assert_eq!(exact, [memory_dir(root).join("patterns/foo.md")]);
        assert_eq!(find_topics(root, "foo").unwrap().len(), 2);
        assert!(find_topics(root, "nope").unwrap().is_empty());
        assert!(find_topics(root, "../../secret.md").unwrap().is_empty());
    }
}
