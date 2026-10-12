use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const ENGRAM_START: &str = "<!-- engram:start -->";
const ENGRAM_END: &str = "<!-- engram:end -->";

/// Memory categories and their OKF v0.2 `type` value.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("patterns", "pattern"),
    ("tripwires", "tripwire"),
    ("architecture", "architecture"),
    ("testing", "testing"),
];

fn okf_type_for(category: &str) -> &'static str {
    CATEGORIES
        .iter()
        .find(|(c, _)| *c == category)
        .map(|(_, t)| *t)
        .unwrap_or("pattern")
}

fn generator() -> String {
    format!("engram/{}", env!("CARGO_PKG_VERSION"))
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
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

    let mut sections = Vec::new();

    for (category, cat_dir) in category_dirs(&memory_dir) {
        let files = sorted_md_files(&cat_dir)?;
        if files.is_empty() {
            continue;
        }

        let mut entries = Vec::new();
        for path in &files {
            let content = std::fs::read_to_string(path)?;
            let slug = slug_of(path);
            let (title, read_when) = match split_frontmatter(&content) {
                Ok((fm, _)) => (fm.title, fm.read_when),
                Err(_) => (slug.clone(), Vec::new()),
            };
            let suffix = if read_when.is_empty() {
                String::new()
            } else {
                format!(" - Read when: {}", read_when.join("; "))
            };
            let title = title.replace('[', "\\[").replace(']', "\\]");
            entries.push(format!("* [{title}](./{category}/{slug}.md){suffix}"));
        }

        sections.push(format!(
            "## {}\n\n{}\n",
            capitalize(category),
            entries.join("\n")
        ));
    }

    let body = if sections.is_empty() {
        "_No learnings yet._\n".to_string()
    } else {
        sections.join("\n")
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
    use crate::claude::{LearningItem, Tripwire as ClaudeTripwire};

    const REPO: &str = "acme/widgets";

    // --- split_frontmatter / render round-trip ---

    #[test]
    fn round_trip_preserves_special_characters_in_title_and_read_when() {
        let fm = Frontmatter::new("patterns", "A \"quoted\" title: with # hash");
        let mut fm = fm;
        fm.read_when = vec!["a \"quoted\" condition: with # hash".to_string()];
        fm.normalize("patterns", REPO);

        let rendered = render(&fm, "Body text.").unwrap();
        let (parsed, body) = split_frontmatter(&rendered).unwrap();

        assert_eq!(parsed.title, "A \"quoted\" title: with # hash");
        assert_eq!(
            parsed.read_when,
            vec!["a \"quoted\" condition: with # hash".to_string()]
        );
        assert_eq!(body.trim(), "Body text.");
    }

    #[test]
    fn parsing_legacy_frontmatter_yields_sources_and_generated() {
        let legacy = "---\ntitle: \"Old style\"\nread_when:\n  - \"doing X\"\ntripwires: []\nlast_updated: \"2024-01-01\"\nsource_issues: [1, 2]\n---\n\nBody.\n";
        let (mut fm, _) = split_frontmatter(legacy).unwrap();
        assert!(fm.generated.is_none());
        assert!(fm.sources.is_empty());

        fm.normalize("patterns", REPO);

        assert_eq!(fm.kind, "pattern");
        let generated = fm.generated.clone().unwrap();
        assert_eq!(generated.at, "2024-01-01");
        assert_eq!(
            generated.by,
            format!("engram/{}", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            fm.sources,
            vec![
                Source {
                    id: "1".to_string(),
                    resource: format!("https://github.com/{REPO}/issues/1")
                },
                Source {
                    id: "2".to_string(),
                    resource: format!("https://github.com/{REPO}/issues/2")
                },
            ]
        );
        assert!(fm.last_updated.is_none());
        assert!(fm.legacy_source_issues.is_none());

        let rendered = render(&fm, "Body.").unwrap();
        assert!(!rendered.contains("last_updated"));
        assert!(!rendered.contains("source_issues"));
    }

    #[test]
    fn sources_merge_dedupes_by_id() {
        let mut fm = Frontmatter::new("patterns", "Title");
        fm.add_source(REPO, 1);
        fm.add_source(REPO, 1);
        fm.add_source(REPO, 2);
        assert_eq!(fm.sources.len(), 2);

        fm.merge_sources(vec![
            Source {
                id: "2".to_string(),
                resource: "https://example.com/other".to_string(),
            },
            Source {
                id: "3".to_string(),
                resource: format!("https://github.com/{REPO}/issues/3"),
            },
        ]);
        assert_eq!(fm.sources.len(), 3);
        // first-seen wins — the id=2 resource is not overwritten
        assert_eq!(
            fm.sources.iter().find(|s| s.id == "2").unwrap().resource,
            format!("https://github.com/{REPO}/issues/2")
        );
    }

    #[test]
    fn unknown_frontmatter_keys_survive_a_rewrite() {
        let content = "---\ntype: \"pattern\"\ntitle: \"Title\"\nread_when: []\ntripwires: []\ncustom_field: \"keep me\"\n---\n\nBody.\n";
        let (mut fm, body) = split_frontmatter(content).unwrap();
        fm.normalize("patterns", REPO);
        let rendered = render(&fm, &body).unwrap();
        let (reparsed, _) = split_frontmatter(&rendered).unwrap();
        assert_eq!(
            reparsed.extra.get("custom_field"),
            Some(&serde_yaml_ng::Value::String("keep me".to_string()))
        );
    }

    fn compact_action(
        slug: &str,
        target: &str,
        body: Option<&str>,
    ) -> crate::claude::CompactAction {
        crate::claude::CompactAction {
            action: "merge_into".to_string(),
            category: "patterns".to_string(),
            slug: slug.to_string(),
            reason: None,
            target_category: Some("patterns".to_string()),
            target_slug: Some(target.to_string()),
            target_updated_body: body.map(str::to_string),
        }
    }

    #[test]
    fn compact_merge_combines_sources_refreshes_date_and_deletes_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cat_dir = root.join(".engram/memory/patterns");
        std::fs::create_dir_all(&cat_dir).unwrap();
        std::fs::write(
            cat_dir.join("target.md"),
            "---\ntitle: \"Target\"\nread_when: []\ntripwires: []\nlast_updated: \"2024-01-01\"\nsource_issues: [1]\n---\n\nOld body.\n",
        )
        .unwrap();
        write_topic_file(root, REPO, &make_item("patterns", "dupe", "Dupe", &[]), 2).unwrap();

        let topics = list_all_topics(root).unwrap();
        let actions = [compact_action("dupe", "target", Some("Merged body."))];
        let (deleted, merged) = apply_compact_actions(root, REPO, &actions, &topics).unwrap();
        assert_eq!((deleted, merged), (0, 1));
        assert!(!cat_dir.join("dupe.md").exists());

        let (fm, body) =
            split_frontmatter(&std::fs::read_to_string(cat_dir.join("target.md")).unwrap())
                .unwrap();
        let ids: Vec<_> = fm.sources.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["1", "2"]);
        assert_eq!(fm.generated.unwrap().at, today_iso());
        assert_eq!(body.trim(), "Merged body.");
    }

    #[test]
    fn compact_merge_into_malformed_target_keeps_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cat_dir = root.join(".engram/memory/patterns");
        std::fs::create_dir_all(&cat_dir).unwrap();
        std::fs::write(cat_dir.join("target.md"), "not frontmatter\n").unwrap();
        write_topic_file(root, REPO, &make_item("patterns", "dupe", "Dupe", &[]), 2).unwrap();

        let topics = list_all_topics(root).unwrap();
        let actions = [
            compact_action("dupe", "target", None),
            compact_action("dupe", "missing", None),
        ];
        let (_, merged) = apply_compact_actions(root, REPO, &actions, &topics).unwrap();
        assert_eq!(merged, 0);
        assert!(cat_dir.join("dupe.md").exists());
        assert_eq!(
            std::fs::read_to_string(cat_dir.join("target.md")).unwrap(),
            "not frontmatter\n"
        );
    }

    #[test]
    fn index_escapes_brackets_in_titles() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let item = make_item(
            "patterns",
            "brackets",
            "Use features = [\"bundled\" ] or ]",
            &[],
        );
        write_topic_file(root, REPO, &item, 1).unwrap();
        rebuild_index(root).unwrap();
        let index = std::fs::read_to_string(root.join(".engram/memory/index.md")).unwrap();
        assert!(
            index.contains("* [Use features = \\[\"bundled\" \\] or \\]](./patterns/brackets.md)")
        );
    }

    #[test]
    fn split_frontmatter_rejects_missing_delimiters() {
        assert!(split_frontmatter("no frontmatter here").is_err());
        assert!(split_frontmatter("---\ntitle: \"x\"\n").is_err());
    }

    // --- today_iso ---

    #[test]
    fn today_iso_format() {
        let s = today_iso();
        assert_eq!(s.len(), 10);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert!(s[..4].parse::<u32>().is_ok());
        assert!(s[5..7].parse::<u32>().is_ok());
        assert!(s[8..10].parse::<u32>().is_ok());
    }

    // --- integration: write_topic_file + rebuild_index ---

    fn make_item(category: &str, slug: &str, title: &str, read_when: &[&str]) -> LearningItem {
        LearningItem {
            category: category.to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            read_when: read_when.iter().map(|s| s.to_string()).collect(),
            tripwires: vec![],
            body: "Body text.".to_string(),
        }
    }

    #[test]
    fn write_topic_file_creates_file_with_correct_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let item = make_item("patterns", "test-slug", "Test Title", &["when testing"]);

        write_topic_file(root, REPO, &item, 42).unwrap();

        let content =
            std::fs::read_to_string(root.join(".engram/memory/patterns/test-slug.md")).unwrap();
        assert!(content.contains("title: Test Title"));
        assert!(content.contains("id: '42'"));
        assert!(content.contains(&format!("https://github.com/{REPO}/issues/42")));
        assert!(content.contains("when testing"));
        assert!(content.contains("Body text."));
        assert!(content.contains("type: pattern"));
    }

    #[test]
    fn write_topic_file_accumulates_source_issues() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let item = make_item("patterns", "my-slug", "My Title", &[]);

        write_topic_file(root, REPO, &item, 1).unwrap();
        write_topic_file(root, REPO, &item, 2).unwrap();

        let content =
            std::fs::read_to_string(root.join(".engram/memory/patterns/my-slug.md")).unwrap();
        let (fm, _) = split_frontmatter(&content).unwrap();
        assert_eq!(fm.generated.as_ref().unwrap().at, today_iso());
        assert_eq!(fm.sources.len(), 2);
        assert!(fm.sources.iter().any(|s| s.id == "1"));
        assert!(fm.sources.iter().any(|s| s.id == "2"));
    }

    #[test]
    fn write_topic_file_with_tripwires() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut item = make_item("tripwires", "my-tripwire", "My Tripwire", &[]);
        item.tripwires = vec![ClaudeTripwire {
            action: "Calling foo()".to_string(),
            warning: "Use bar() instead".to_string(),
        }];

        write_topic_file(root, REPO, &item, 7).unwrap();

        let content =
            std::fs::read_to_string(root.join(".engram/memory/tripwires/my-tripwire.md")).unwrap();
        assert!(content.contains("Calling foo()"));
        assert!(content.contains("Use bar() instead"));
        assert!(content.contains("type: tripwire"));
    }

    #[test]
    fn rebuild_index_produces_okf_entry() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let item = make_item("patterns", "my-pattern", "My Pattern", &["when doing X"]);

        write_topic_file(root, REPO, &item, 1).unwrap();
        rebuild_index(root).unwrap();

        let index = std::fs::read_to_string(root.join(".engram/memory/index.md")).unwrap();
        assert!(index.starts_with("---\nokf_version: \"0.2\"\n---"));
        assert!(!index.contains('@'));
        assert!(index.contains("My Pattern"));
        assert!(index.contains("when doing X"));
        assert!(index.contains("(./patterns/my-pattern.md)"));
    }

    #[test]
    fn rebuild_index_empty_memory_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".engram/memory")).unwrap();

        rebuild_index(root).unwrap();

        let index = std::fs::read_to_string(root.join(".engram/memory/index.md")).unwrap();
        assert!(index.starts_with("---\nokf_version: \"0.2\"\n---"));
        assert!(index.contains("No learnings yet"));
    }

    #[test]
    fn migrate_all_normalizes_legacy_files_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cat_dir = root.join(".engram/memory/tripwires");
        std::fs::create_dir_all(&cat_dir).unwrap();
        std::fs::write(
            cat_dir.join("legacy.md"),
            "---\ntitle: \"Legacy\"\nread_when:\n  - \"doing X\"\ntripwires: []\nlast_updated: \"2024-01-01\"\nsource_issues: [9]\n---\n\nBody.\n",
        )
        .unwrap();

        let (processed, skipped) = migrate_all(root, REPO).unwrap();
        assert_eq!(processed, 1);
        assert_eq!(skipped, 0);

        let after_first = std::fs::read_to_string(cat_dir.join("legacy.md")).unwrap();
        assert!(!after_first.contains("last_updated"));
        assert!(!after_first.contains("source_issues"));
        assert!(after_first.contains("type: tripwire"));

        migrate_all(root, REPO).unwrap();
        let after_second = std::fs::read_to_string(cat_dir.join("legacy.md")).unwrap();
        assert_eq!(after_first, after_second);
    }

    #[test]
    fn migrate_all_reports_malformed_files_without_rewriting() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let cat_dir = root.join(".engram/memory/patterns");
        std::fs::create_dir_all(&cat_dir).unwrap();
        std::fs::write(cat_dir.join("broken.md"), "no frontmatter here\n").unwrap();

        let (processed, skipped) = migrate_all(root, REPO).unwrap();
        assert_eq!(processed, 0);
        assert_eq!(skipped, 1);

        let content = std::fs::read_to_string(cat_dir.join("broken.md")).unwrap();
        assert_eq!(content, "no frontmatter here\n");
    }
}
