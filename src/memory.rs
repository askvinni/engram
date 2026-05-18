use anyhow::Result;
use std::path::Path;

const ENGRAM_START: &str = "<!-- engram:start -->";
const ENGRAM_END: &str = "<!-- engram:end -->";

pub fn read_all(repo_root: &Path) -> Result<String> {
    let memory_dir = repo_root.join(".engram/memory");
    if !memory_dir.exists() {
        return Ok(String::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(&memory_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
        .collect();
    entries.sort_by_key(|e| e.path());

    let mut all = String::new();
    for entry in entries {
        let content = std::fs::read_to_string(entry.path())?;
        let category = entry
            .path()
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        all.push_str(&format!("### {category}\n{content}\n\n"));
    }
    Ok(all)
}

pub fn merge_item(repo_root: &Path, category: &str, content: &str, issue_number: u64) -> Result<()> {
    let memory_dir = repo_root.join(".engram/memory");
    std::fs::create_dir_all(&memory_dir)?;

    let path = memory_dir.join(format!("{category}.md"));
    let mut body = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        format!("# {category}\n\n")
    };

    body.push_str(&format!("- {content} _(from #{})\n", issue_number));
    std::fs::write(path, body)?;
    Ok(())
}

pub fn write_claude_md_section(repo_root: &Path) -> Result<()> {
    let memory = read_all(repo_root)?;
    let claude_md_path = repo_root.join("CLAUDE.md");

    let existing = if claude_md_path.exists() {
        std::fs::read_to_string(&claude_md_path)?
    } else {
        String::new()
    };

    let memory_content = if memory.is_empty() {
        "_No learnings yet. Run `engram learn <issue>` after closing a plan._\n".to_string()
    } else {
        memory
    };

    let section = format!(
        "{ENGRAM_START}\n## Engram Memory\n\nLearnings from past development work:\n\n{memory_content}\n{ENGRAM_END}"
    );

    let new_content = if let (Some(start), Some(end_idx)) = (
        existing.find(ENGRAM_START),
        existing.find(ENGRAM_END),
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
