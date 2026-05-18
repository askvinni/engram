use anyhow::Result;
use std::path::Path;

const ENGRAM_START: &str = "<!-- engram:start -->";
const ENGRAM_END: &str = "<!-- engram:end -->";

pub struct TopicFile {
    pub category: String,
    pub slug: String,
    pub content: String,
}

pub fn list_all_topics(repo_root: &Path) -> Result<Vec<TopicFile>> {
    let memory_dir = repo_root.join(".engram/memory");
    let mut topics = Vec::new();
    for category in &["patterns", "tripwires", "architecture", "testing"] {
        let cat_dir = memory_dir.join(category);
        if !cat_dir.exists() {
            continue;
        }
        let mut files: Vec<_> = std::fs::read_dir(&cat_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
            .collect();
        files.sort_by_key(|e| e.path());
        for entry in files {
            let path = entry.path();
            let content = std::fs::read_to_string(&path)?;
            let slug = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            topics.push(TopicFile {
                category: category.to_string(),
                slug,
                content,
            });
        }
    }
    Ok(topics)
}

pub fn apply_compact_actions(
    repo_root: &Path,
    actions: &[crate::claude::CompactAction],
    topics: &[TopicFile],
) -> Result<(usize, usize)> {
    let memory_dir = repo_root.join(".engram/memory");
    let mut deleted = 0;
    let mut merged = 0;

    // First pass: update merge targets before deleting sources
    for action in actions {
        if action.action != "merge_into" {
            continue;
        }
        let (target_cat, target_slug) = match (&action.target_category, &action.target_slug) {
            (Some(c), Some(s)) => (c, s),
            _ => continue,
        };

        let source_issues: Vec<u64> = topics
            .iter()
            .find(|t| t.category == action.category && t.slug == action.slug)
            .map(|f| parse_source_issues(&f.content))
            .unwrap_or_default();

        let target_path = memory_dir.join(target_cat).join(format!("{target_slug}.md"));
        if !target_path.exists() {
            continue;
        }

        let existing = std::fs::read_to_string(&target_path)?;
        let updated = if let Some(new_body) = &action.target_updated_body {
            update_topic_body_and_issues(&existing, new_body, &source_issues)
        } else {
            update_topic_issues_only(&existing, &source_issues)
        };
        std::fs::write(&target_path, updated)?;
    }

    // Second pass: delete files (deletes and merge sources)
    for action in actions {
        if action.action == "delete" || action.action == "merge_into" {
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

fn update_topic_body_and_issues(content: &str, new_body: &str, extra_issues: &[u64]) -> String {
    let today = today_iso();
    let mut existing_issues = parse_source_issues(content);
    for n in extra_issues {
        if !existing_issues.contains(n) {
            existing_issues.push(*n);
        }
    }
    existing_issues.sort();
    let issues_yaml = format!(
        "[{}]",
        existing_issues
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Rebuild file: preserve frontmatter fields except body, last_updated, source_issues
    let fm_end = find_frontmatter_end(content);
    let frontmatter = &content[..fm_end];

    // Replace last_updated and source_issues in frontmatter
    let frontmatter = replace_frontmatter_field(frontmatter, "last_updated", &format!("\"{today}\""));
    let frontmatter = replace_frontmatter_field(&frontmatter, "source_issues", &issues_yaml);

    format!("{frontmatter}\n{new_body}\n")
}

fn update_topic_issues_only(content: &str, extra_issues: &[u64]) -> String {
    let today = today_iso();
    let mut existing_issues = parse_source_issues(content);
    for n in extra_issues {
        if !existing_issues.contains(n) {
            existing_issues.push(*n);
        }
    }
    existing_issues.sort();
    let issues_yaml = format!(
        "[{}]",
        existing_issues
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let fm_end = find_frontmatter_end(content);
    let frontmatter = &content[..fm_end];
    let body = content[fm_end..].trim_start_matches('\n');

    let frontmatter = replace_frontmatter_field(frontmatter, "last_updated", &format!("\"{today}\""));
    let frontmatter = replace_frontmatter_field(&frontmatter, "source_issues", &issues_yaml);

    format!("{frontmatter}\n{body}\n")
}

fn find_frontmatter_end(content: &str) -> usize {
    // Find the closing --- of YAML frontmatter
    let mut in_fm = false;
    let mut line_start = 0;
    for (i, c) in content.char_indices() {
        if c == '\n' || i == content.len() - 1 {
            let line = &content[line_start..if c == '\n' { i } else { i + 1 }];
            if line_start == 0 && line.trim() == "---" {
                in_fm = true;
            } else if in_fm && line.trim() == "---" {
                return i + 1; // include the closing ---\n
            }
            line_start = i + 1;
        }
    }
    // fallback: whole content is frontmatter
    content.len()
}

fn replace_frontmatter_field(frontmatter: &str, field: &str, value: &str) -> String {
    let prefix = format!("{field}:");
    let lines: Vec<&str> = frontmatter.lines().collect();
    let mut result = Vec::new();
    for line in &lines {
        if line.trim_start().starts_with(&prefix) {
            result.push(format!("{field}: {value}"));
        } else {
            result.push(line.to_string());
        }
    }
    result.join("\n") + if frontmatter.ends_with('\n') { "\n" } else { "" }
}

/// Read all memory files for injection into the synthesis prompt.
/// Returns a compact summary of existing topics so Claude avoids duplicates.
pub fn read_all(repo_root: &Path) -> Result<String> {
    let memory_dir = repo_root.join(".engram/memory");
    if !memory_dir.exists() {
        return Ok(String::new());
    }

    let mut all = String::new();
    for category in &["patterns", "tripwires", "architecture", "testing"] {
        let cat_dir = memory_dir.join(category);
        if !cat_dir.exists() {
            continue;
        }
        let mut files: Vec<_> = std::fs::read_dir(&cat_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
            .collect();
        files.sort_by_key(|e| e.path());

        for entry in &files {
            let content = std::fs::read_to_string(entry.path())?;
            // Extract just the frontmatter title and body for context
            let slug = entry
                .path()
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            all.push_str(&format!("### [{category}/{slug}]\n{content}\n\n"));
        }
    }
    Ok(all)
}

/// Write a single learning item to .engram/memory/<category>/<slug>.md
pub fn write_topic_file(
    repo_root: &Path,
    category: &str,
    slug: &str,
    title: &str,
    read_when: &[String],
    tripwires: &[crate::claude::Tripwire],
    body: &str,
    issue_number: u64,
) -> Result<()> {
    let cat_dir = repo_root.join(format!(".engram/memory/{category}"));
    std::fs::create_dir_all(&cat_dir)?;

    let path = cat_dir.join(format!("{slug}.md"));

    // If file exists, append the new source issue rather than overwrite
    let source_issues = if path.exists() {
        let existing = std::fs::read_to_string(&path)?;
        let mut nums = parse_source_issues(&existing);
        if !nums.contains(&issue_number) {
            nums.push(issue_number);
            nums.sort();
        }
        nums
    } else {
        vec![issue_number]
    };

    let today = today_iso();

    let read_when_yaml: String = read_when
        .iter()
        .map(|s| format!("  - \"{s}\"\n"))
        .collect();

    let tripwires_yaml: String = if tripwires.is_empty() {
        "tripwires: []\n".to_string()
    } else {
        let items: String = tripwires
            .iter()
            .map(|t| format!("  - action: \"{}\"\n    warning: \"{}\"\n", t.action, t.warning))
            .collect();
        format!("tripwires:\n{items}")
    };

    let issues_yaml = format!(
        "[{}]",
        source_issues
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let content = format!(
        "---\ntitle: \"{title}\"\nread_when:\n{read_when_yaml}{tripwires_yaml}last_updated: \"{today}\"\nsource_issues: {issues_yaml}\n---\n\n{body}\n"
    );

    std::fs::write(path, content)?;
    Ok(())
}

/// Auto-generate .engram/memory/index.md as a routing table for agents.
pub fn rebuild_index(repo_root: &Path) -> Result<()> {
    let memory_dir = repo_root.join(".engram/memory");
    if !memory_dir.exists() {
        return Ok(());
    }

    let mut rows = Vec::new();

    for category in &["patterns", "tripwires", "architecture", "testing"] {
        let cat_dir = memory_dir.join(category);
        if !cat_dir.exists() {
            continue;
        }
        let mut files: Vec<_> = std::fs::read_dir(&cat_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
            .collect();
        files.sort_by_key(|e| e.path());

        for entry in &files {
            let content = std::fs::read_to_string(entry.path())?;
            let slug = entry
                .path()
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let title = extract_frontmatter_field(&content, "title").unwrap_or_else(|| slug.clone());
            let read_when = extract_frontmatter_list(&content, "read_when");
            let read_when_str = if read_when.is_empty() {
                "—".to_string()
            } else {
                read_when.join("; ")
            };
            let rel_path = format!(".engram/memory/{category}/{slug}.md");
            rows.push(format!("| @{rel_path} | {title} | {read_when_str} |"));
        }
    }

    let table = if rows.is_empty() {
        "_No learnings yet._\n".to_string()
    } else {
        format!(
            "| File | Title | Read when |\n|------|-------|-----------|\n{}\n",
            rows.join("\n")
        )
    };

    let content = format!(
        "# Engram Memory Index\n\nAgents: read this index to find relevant learned docs. Load individual files only when their \"Read when\" condition matches your current task.\n\n{table}"
    );

    std::fs::write(memory_dir.join("index.md"), content)?;
    Ok(())
}

/// Migrate existing flat .engram/memory/<category>.md files to the new topic-file structure.
/// Each bullet item becomes its own file under .engram/memory/<category>/<slug>.md.
pub fn migrate_flat_files(repo_root: &Path) -> Result<usize> {
    let memory_dir = repo_root.join(".engram/memory");
    if !memory_dir.exists() {
        return Ok(0);
    }

    let flat_files: Vec<_> = std::fs::read_dir(&memory_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().map_or(false, |ext| ext == "md")
                && e.path().file_name() != Some(std::ffi::OsStr::new("index.md"))
                // Only top-level files (not inside subdirs)
                && e.path().parent() == Some(&memory_dir)
        })
        .collect();

    if flat_files.is_empty() {
        return Ok(0);
    }

    let mut migrated = 0;

    for entry in &flat_files {
        let path = entry.path();
        let category = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let content = std::fs::read_to_string(&path)?;

        for line in content.lines() {
            let line = line.trim();
            if !line.starts_with("- ") {
                continue;
            }
            let text = line.trim_start_matches("- ");

            // Extract issue number from _(from #N)_ suffix
            let (body_text, issue_num) = extract_issue_ref(text);
            if body_text.is_empty() {
                continue;
            }

            let slug = slugify(&body_text);
            let cat_dir = memory_dir.join(&category);
            std::fs::create_dir_all(&cat_dir)?;
            let dest = cat_dir.join(format!("{slug}.md"));

            // Skip if already migrated
            if dest.exists() {
                continue;
            }

            let today = today_iso();
            let issues_yaml = issue_num
                .map(|n| format!("[{n}]"))
                .unwrap_or_else(|| "[]".to_string());

            let file_content = format!(
                "---\ntitle: \"{}\"\nread_when:\n  - \"(migrated — add read_when conditions)\"\ntripwires: []\nlast_updated: \"{today}\"\nsource_issues: {issues_yaml}\n---\n\n{body_text}\n",
                truncate_title(&body_text, 60)
            );

            std::fs::write(&dest, file_content)?;
            migrated += 1;
        }

        // Remove the flat file after migration
        std::fs::remove_file(&path)?;
    }

    Ok(migrated)
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

    let new_content = if let (Some(start), Some(end_idx)) = (
        existing.find(ENGRAM_START),
        existing.rfind(ENGRAM_END),
    ) {
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

fn today_iso() -> String {
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

fn slugify(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    // Collapse multiple dashes
    let mut slug = String::new();
    let mut prev_dash = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_dash {
                slug.push(c);
            }
            prev_dash = true;
        } else {
            slug.push(c);
            prev_dash = false;
        }
    }
    // Truncate to 60 chars
    slug.chars().take(60).collect()
}

fn truncate_title(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max - 1).collect();
        format!("{truncated}…")
    }
}

fn extract_issue_ref(s: &str) -> (String, Option<u64>) {
    // Strip _(from #N)_ or _(from #N)_ _(from #M)_ suffixes
    let re_suffix = " _(from #";
    if let Some(pos) = s.rfind(re_suffix) {
        let body = s[..pos].trim().to_string();
        let rest = &s[pos + re_suffix.len()..];
        let num: Option<u64> = rest
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|n| n.parse().ok());
        return (body, num);
    }
    (s.to_string(), None)
}

fn parse_source_issues(content: &str) -> Vec<u64> {
    // Look for "source_issues: [N, M, ...]" in frontmatter
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("source_issues:") {
            let rest = line.trim_start_matches("source_issues:").trim();
            return rest
                .trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .filter_map(|s| s.trim().parse::<u64>().ok())
                .collect();
        }
    }
    Vec::new()
}

fn extract_frontmatter_field(content: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with(&prefix) {
            let val = line[prefix.len()..].trim().trim_matches('"').to_string();
            return Some(val);
        }
    }
    None
}

fn extract_frontmatter_list(content: &str, field: &str) -> Vec<String> {
    let prefix = format!("{field}:");
    let mut in_list = false;
    let mut items = Vec::new();
    for line in content.lines() {
        let line_trim = line.trim();
        if line_trim.starts_with(&prefix) {
            in_list = true;
            continue;
        }
        if in_list {
            if line_trim.starts_with("- ") {
                let val = line_trim
                    .trim_start_matches("- ")
                    .trim_matches('"')
                    .to_string();
                items.push(val);
            } else if !line_trim.is_empty() && !line_trim.starts_with(' ') {
                break;
            }
        }
    }
    items
}
