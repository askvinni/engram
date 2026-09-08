use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

const DB_RELATIVE: &str = ".engram/index.db";

static MIGRATIONS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/migrations");

fn run_migrations(conn: &Connection) -> Result<()> {
    let version: usize = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .context("reading schema version")?;

    let mut files: Vec<_> = MIGRATIONS_DIR.files().collect();
    files.sort_by_key(|f| f.path());

    for (i, file) in files.iter().enumerate().skip(version) {
        let sql = file
            .contents_utf8()
            .with_context(|| format!("migration {} is not UTF-8", i + 1))?;
        conn.execute_batch(sql)
            .with_context(|| format!("applying migration {}", i + 1))?;
        conn.execute_batch(&format!("PRAGMA user_version = {}", i + 1))
            .with_context(|| format!("bumping schema version to {}", i + 1))?;
    }
    Ok(())
}

fn open_db(repo_root: &Path) -> Result<Connection> {
    let db_path = repo_root.join(DB_RELATIVE);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).context("creating .engram dir")?;
    }
    let conn = Connection::open(&db_path).context("opening index.db")?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .context("setting WAL mode")?;
    run_migrations(&conn)?;
    Ok(conn)
}

#[allow(dead_code)]
pub fn rebuild_index(repo_root: &Path) -> Result<()> {
    let conn = open_db(repo_root)?;
    let topics = crate::memory::list_all_topics(repo_root).context("listing memory files")?;

    conn.execute_batch("DELETE FROM file_index; DELETE FROM memory_fts;")
        .context("clearing index")?;

    for topic in &topics {
        let rel_path = format!(".engram/memory/{}/{}.md", topic.category, topic.slug);
        insert_row(
            &conn,
            &rel_path,
            &topic.category,
            &topic.slug,
            &topic.content,
        )?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn upsert_file(
    repo_root: &Path,
    rel_path: &str,
    category: &str,
    slug: &str,
    content: &str,
) -> Result<()> {
    let conn = open_db(repo_root)?;
    upsert_row(&conn, rel_path, category, slug, content)
}

#[allow(dead_code)]
pub fn check_index_health(repo_root: &Path) -> Result<bool> {
    Ok(repo_root.join(DB_RELATIVE).exists())
}

#[derive(Debug)]
pub struct SearchResult {
    pub path: String,
    pub category: String,
    pub snippet: String,
}

/// Query the FTS index, ranked by bm25. Rebuilds the index on demand if missing.
pub fn search(repo_root: &Path, query: &str) -> Result<Vec<SearchResult>> {
    if !repo_root.join(DB_RELATIVE).exists() {
        rebuild_index(repo_root)?;
    }
    let conn = open_db(repo_root)?;
    let mut stmt = conn
        .prepare(
            "SELECT path, category, content, bm25(memory_fts) AS rank
             FROM memory_fts
             WHERE memory_fts MATCH ?1
             ORDER BY rank",
        )
        .context("preparing search query")?;
    let results = stmt
        .query_map(params![query], |row| {
            let content: String = row.get(2)?;
            let snippet: String = content.chars().take(120).collect();
            Ok(SearchResult {
                path: row.get(0)?,
                category: row.get(1)?,
                snippet,
            })
        })
        .context("executing search query")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collecting search results")?;
    Ok(results)
}

fn insert_row(
    conn: &Connection,
    path: &str,
    category: &str,
    slug: &str,
    content: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_fts (path, category, slug, content) VALUES (?1, ?2, ?3, ?4)",
        params![path, category, slug, content],
    )
    .context("inserting into memory_fts")?;
    let rowid = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO file_index VALUES (?1, ?2)",
        params![path, rowid],
    )
    .context("inserting into file_index")?;
    Ok(())
}

#[allow(dead_code)]
fn upsert_row(
    conn: &Connection,
    path: &str,
    category: &str,
    slug: &str,
    content: &str,
) -> Result<()> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT fts_rowid FROM file_index WHERE path = ?1",
            params![path],
            |row| row.get(0),
        )
        .optional()
        .context("looking up existing fts_rowid")?;

    if let Some(old_rowid) = existing {
        conn.execute(
            "DELETE FROM memory_fts WHERE rowid = ?1",
            params![old_rowid],
        )
        .context("deleting stale fts row")?;
        conn.execute("DELETE FROM file_index WHERE path = ?1", params![path])
            .context("deleting stale file_index row")?;
    }
    insert_row(conn, path, category, slug, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::LearningItem;
    use crate::memory::write_topic_file;

    fn make_item(category: &str, slug: &str) -> LearningItem {
        LearningItem {
            category: category.to_string(),
            slug: slug.to_string(),
            title: format!("{slug} title"),
            read_when: vec!["when testing".to_string()],
            tripwires: vec![],
            body: format!("Body of {slug}."),
        }
    }

    fn fts_count(root: &std::path::Path) -> i64 {
        let conn = Connection::open(root.join(".engram/index.db")).unwrap();
        conn.query_row("SELECT count(*) FROM memory_fts", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn rebuild_index_creates_db_indexes_files_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_topic_file(root, &make_item("patterns", "foo"), 1).unwrap();
        write_topic_file(root, &make_item("tripwires", "bar"), 2).unwrap();

        assert!(!check_index_health(root).unwrap());
        rebuild_index(root).unwrap();
        assert!(check_index_health(root).unwrap());
        assert_eq!(fts_count(root), 2);

        rebuild_index(root).unwrap();
        assert_eq!(fts_count(root), 2);
    }

    #[test]
    fn upsert_file_adds_then_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".engram/memory")).unwrap();

        upsert_file(
            root,
            ".engram/memory/patterns/foo.md",
            "patterns",
            "foo",
            "first",
        )
        .unwrap();
        assert_eq!(fts_count(root), 1);

        upsert_file(
            root,
            ".engram/memory/patterns/foo.md",
            "patterns",
            "foo",
            "updated",
        )
        .unwrap();
        assert_eq!(fts_count(root), 1);

        let conn = Connection::open(root.join(".engram/index.db")).unwrap();
        let stored: String = conn
            .query_row(
                "SELECT content FROM memory_fts WHERE slug = 'foo'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "updated");
    }

    #[test]
    fn search_finds_match_and_handles_miss() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let item = LearningItem {
            category: "patterns".to_string(),
            slug: "race-condition".to_string(),
            title: "race-condition title".to_string(),
            read_when: vec!["when testing".to_string()],
            tripwires: vec![],
            body: "A race condition occurs when two threads access shared state concurrently."
                .to_string(),
        };
        write_topic_file(root, &item, 1).unwrap();
        rebuild_index(root).unwrap();

        let hits = search(root, "race condition").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.contains("race-condition"));
        assert_eq!(hits[0].category, "patterns");

        let miss = search(root, "zzzyyyxxx").unwrap();
        assert!(miss.is_empty());
    }

    #[test]
    fn search_rebuilds_index_on_demand_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_topic_file(root, &make_item("patterns", "foo"), 1).unwrap();

        assert!(!root.join(".engram/index.db").exists());
        let results = search(root, "foo title").unwrap();
        assert!(root.join(".engram/index.db").exists());
        assert_eq!(results.len(), 1);
    }
}
