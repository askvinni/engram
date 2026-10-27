use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const ENGRAM_START: &str = "<!-- engram:start -->";
const ENGRAM_END: &str = "<!-- engram:end -->";
/// Instruction files every agent session loads; learnings must never restate them.
const INSTRUCTION_FILES: &[&str] = &["CLAUDE.md", "AGENTS.md"];

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

fn okf_type_for(category: &str) -> &'static str {
    CATEGORIES
        .iter()
        .find(|(c, _)| *c == category)
        .map(|(_, t)| *t)
        .unwrap_or("pattern")
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
                        resource: format!("https://github.com/{repo}/issues/{n}"),
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
                id: id.clone(),
                resource: format!("https://github.com/{repo}/issues/{issue_number}"),
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
            .map(|p| offset + p)
            .unwrap_or(content.len());
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
        .filter_map(|e| e.ok())
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
    let memory_dir = repo_root.join(".engram/memory");
    let mut topics = Vec::new();
    for (category, cat_dir) in category_dirs(&memory_dir) {
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

pub fn apply_compact_actions(
    repo_root: &Path,
    repo: &str,
    actions: &[crate::claude::CompactAction],
    topics: &[TopicFile],
) -> Result<(usize, usize)> {
    let memory_dir = repo_root.join(".engram/memory");
    let mut deleted = 0;
    let mut merged = 0;
    let mut merged_sources = std::collections::HashSet::new();

    // First pass: update merge targets before deleting sources
    for action in actions {
        if action.action != "merge_into" {
            continue;
        }
        let (target_cat, target_slug) = match (&action.target_category, &action.target_slug) {
            (Some(c), Some(s)) => (c, s),
            _ => continue,
        };

        let source_entries: Vec<Source> = topics
            .iter()
            .find(|t| t.category == action.category && t.slug == action.slug)
            .map(|f| match split_frontmatter(&f.content) {
                Ok((mut fm, _)) => {
                    fm.normalize(&action.category, repo);
                    fm.sources
                }
                Err(_) => Vec::new(),
            })
            .unwrap_or_default();

        let target_path = memory_dir
            .join(target_cat)
            .join(format!("{target_slug}.md"));
        if !target_path.exists() {
            continue;
        }

        let existing = std::fs::read_to_string(&target_path)?;
        let (mut fm, existing_body) = match split_frontmatter(&existing) {
            Ok(parsed) => parsed,
            Err(e) => {
                eprintln!(
                    "warning: not merging {}/{} — target {} is malformed: {e:#}",
                    action.category,
                    action.slug,
                    target_path.display()
                );
                continue;
            }
        };
        fm.normalize(target_cat, repo);
        fm.merge_sources(source_entries);
        fm.touch();
        let body = action
            .target_updated_body
            .as_deref()
            .unwrap_or(&existing_body);
        let updated = render(&fm, body)?;
        std::fs::write(&target_path, updated)?;
        merged_sources.insert((action.category.as_str(), action.slug.as_str()));
    }

    // Second pass: delete files. A merge source is only deleted once its
    // merge landed, so a missing or malformed target never loses content.
    for action in actions {
        let merge_landed = action.action == "merge_into"
            && merged_sources.contains(&(action.category.as_str(), action.slug.as_str()));
        if action.action == "delete" || merge_landed {
            let path = memory_dir
                .join(&action.category)
                .join(format!("{}.md", action.slug));
            if path.exists() {
                std::fs::remove_file(&path)?;
                if action.action == "delete" {
                    deleted += 1;
                } else {
                    merged += 1;
                }
            }
        }
    }

    Ok((deleted, merged))
}

/// Read all memory files for injection into the synthesis prompt.
/// Returns a compact summary of existing topics so Claude avoids duplicates.
pub fn read_all(repo_root: &Path) -> Result<String> {
    let memory_dir = repo_root.join(".engram/memory");
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
    let cat_dir = repo_root.join(format!(".engram/memory/{category}"));
    std::fs::create_dir_all(&cat_dir)?;

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

    fm.title = item.title.clone();
    fm.read_when = item.read_when.clone();
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
    std::fs::write(path, content)?;
    Ok(())
}

/// Rewrites every topic file's frontmatter into OKF v0.2 shape, accepting
/// legacy `last_updated`/`source_issues` fields. Files with malformed YAML
/// are reported and left untouched. Returns (processed, skipped).
pub fn migrate_all(repo_root: &Path, repo: &str) -> Result<(usize, usize)> {
    let memory_dir = repo_root.join(".engram/memory");
    let mut processed = 0;
    let mut skipped = 0;

    for (category, cat_dir) in category_dirs(&memory_dir) {
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
    let memory_dir = repo_root.join(".engram/memory");
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
    let claude_md_path = repo_root.join("CLAUDE.md");
    let memory_dir = repo_root.join(".engram/memory");
    let index_path = memory_dir.join("index.md");

    let existing = if claude_md_path.exists() {
        std::fs::read_to_string(&claude_md_path)?
    } else {
        String::new()
    };

    let body = if index_path.exists() {
        "@.engram/memory/index.md\n".to_string()
    } else {
        "_No learnings yet. Run `engram learn <issue>` after closing a plan._\n".to_string()
    };

    let section = format!("{ENGRAM_START}\n## Engram Memory\n\n{body}\n{ENGRAM_END}");

    let new_content = if let (Some(start), Some(end_idx)) =
        (existing.find(ENGRAM_START), existing.rfind(ENGRAM_END))
    {
        let end = end_idx + ENGRAM_END.len();
        format!("{}{}{}", &existing[..start], section, &existing[end..])
    } else if existing.is_empty() {
        section
    } else {
        format!("{}\n\n{}", existing.trim_end(), section)
    };

    std::fs::write(claude_md_path, new_content)?;
    Ok(())
}

/// Removes engram's own managed section (`ENGRAM_START`..`ENGRAM_END`, markers
/// included) so synthesis sees only the human-written instructions.
pub fn strip_engram_section(text: &str) -> String {
    match (text.find(ENGRAM_START), text.rfind(ENGRAM_END)) {
        (Some(start), Some(end_idx)) if end_idx >= start => {
            let end = end_idx + ENGRAM_END.len();
            format!("{}{}", &text[..start], &text[end..])
        }
        _ => text.to_string(),
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

// --- helpers ---

pub fn today_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    // Days since epoch to Gregorian date
    let (y, m, d) = days_to_ymd(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn days_to_ymd(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + (era as i32) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::{CompactAction, LearningItem, Tripwire as ClaudeTripwire};

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
    fn today_iso_format() {
        let s = today_iso();
        assert_eq!(s.len(), 10);
        assert!(s[..4].parse::<u32>().is_ok() && &s[4..5] == "-" && &s[7..8] == "-");
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
            action: "merge_into".into(),
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
}
