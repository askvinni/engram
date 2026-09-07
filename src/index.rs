use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Debug, serde::Serialize)]
pub struct SearchResult {
    pub path: String,
    pub category: String,
    pub snippet: String,
}

const DB_RELATIVE: &str = ".engram/index.db";

fn open_db(repo_root: &Path) -> Result<Connection> {
    let db_path = repo_root.join(DB_RELATIVE);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).context("creating .engram dir")?;
    }
    let conn = Connection::open(&db_path).context("opening index.db")?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS file_index (
             path     TEXT PRIMARY KEY,
             fts_rowid INTEGER NOT NULL
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
             path     UNINDEXED,
             category UNINDEXED,
             slug     UNINDEXED,
             content,
             tokenize='unicode61'
         );",
    )
    .context("initializing schema")?;
    Ok(conn)
}

/// Rebuild the FTS index from scratch by scanning all memory files.
pub fn rebuild_index(repo_root: &Path) -> Result<()> {
    let conn = open_db(repo_root)?;
    let topics = crate::memory::list_all_topics(repo_root).context("listing memory files")?;

    conn.execute_batch("DELETE FROM file_index; DELETE FROM memory_fts;")
        .context("clearing index")?;

    for topic in &topics {
        let rel_path = format!(".engram/memory/{}/{}.md", topic.category, topic.slug);
        insert_row(&conn, &rel_path, &topic.category, &topic.slug, &topic.content)?;
    }
    Ok(())
}

/// Insert or replace one file's entry in the FTS index.
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

/// Returns true if the index db file exists. Consumed by `engram doctor` (node 4.4).
pub fn check_index_health(repo_root: &Path) -> Result<bool> {
    Ok(repo_root.join(DB_RELATIVE).exists())
}

/// Search the FTS index, rebuilding it on demand if missing or empty.
pub fn search(repo_root: &Path, query: &str) -> Result<Vec<SearchResult>> {
    if !repo_root.join(DB_RELATIVE).exists() {
        rebuild_index(repo_root)?;
    }
    let conn = open_db(repo_root)?;
    let row_count: i64 = conn
        .query_row("SELECT count(*) FROM memory_fts", [], |r| r.get(0))
        .context("counting index rows")?;
    if row_count == 0 {
        drop(conn);
        rebuild_index(repo_root)?;
        let conn = open_db(repo_root)?;
        return query_fts(&conn, query);
    }
    query_fts(&conn, query)
}

fn query_fts(conn: &Connection, query: &str) -> Result<Vec<SearchResult>> {
    let mut stmt = conn
        .prepare(
            "SELECT path, category,
                    snippet(memory_fts, 3, '', '', '...', 15)
             FROM memory_fts
             WHERE memory_fts MATCH ?1
             ORDER BY rank",
        )
        .context("preparing search query")?;
    let results = stmt
        .query_map(params![query], |row| {
            Ok(SearchResult {
                path: row.get(0)?,
                category: row.get(1)?,
                snippet: row.get::<_, String>(2)?.trim().replace('\n', " "),
            })
        })
        .context("executing search query")?
        .collect::<Result<Vec<_>, _>>()
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
        conn.execute("DELETE FROM memory_fts WHERE rowid = ?1", params![old_rowid])
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

    #[test]
    fn rebuild_index_empty_memory_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".engram/memory")).unwrap();
        rebuild_index(root).unwrap();
        assert!(root.join(".engram/index.db").exists());
    }

    #[test]
    fn rebuild_index_indexes_written_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_topic_file(root, &make_item("patterns", "foo"), 1).unwrap();
        write_topic_file(root, &make_item("tripwires", "bar"), 2).unwrap();

        rebuild_index(root).unwrap();

        let conn = Connection::open(root.join(".engram/index.db")).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM memory_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn rebuild_index_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_topic_file(root, &make_item("patterns", "foo"), 1).unwrap();

        rebuild_index(root).unwrap();
        rebuild_index(root).unwrap();

        let conn = Connection::open(root.join(".engram/index.db")).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM memory_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn upsert_file_adds_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".engram/memory")).unwrap();

        upsert_file(root, ".engram/memory/patterns/foo.md", "patterns", "foo", "first content")
            .unwrap();
        upsert_file(root, ".engram/memory/patterns/foo.md", "patterns", "foo", "updated content")
            .unwrap();

        let conn = Connection::open(root.join(".engram/index.db")).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM memory_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let stored: String = conn
            .query_row(
                "SELECT content FROM memory_fts WHERE slug = 'foo'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "updated content");
    }

    #[test]
    fn upsert_file_malformed_frontmatter_indexed_anyway() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".engram/memory")).unwrap();

        let bad_content = "not yaml frontmatter\njust raw text\n";
        upsert_file(
            root,
            ".engram/memory/patterns/bad.md",
            "patterns",
            "bad",
            bad_content,
        )
        .unwrap();

        let conn = Connection::open(root.join(".engram/index.db")).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM memory_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn search_returns_ranked_results() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut a = make_item("patterns", "race-condition");
        a.body = "A race condition can occur when two threads access shared state.".to_string();
        let mut b = make_item("tripwires", "lock-ordering");
        b.body = "Lock ordering prevents deadlock but has nothing to do with races.".to_string();
        write_topic_file(root, &a, 1).unwrap();
        write_topic_file(root, &b, 2).unwrap();
        rebuild_index(root).unwrap();

        let results = search(root, "race condition").unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].path, ".engram/memory/patterns/race-condition.md");
    }

    #[test]
    fn search_empty_result_returns_empty_vec() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".engram/memory")).unwrap();
        rebuild_index(root).unwrap();
        let results = search(root, "nonexistent query xyz").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_rebuilds_missing_index_on_demand() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut item = make_item("patterns", "foo");
        item.body = "unique search term: xyzzy".to_string();
        write_topic_file(root, &item, 1).unwrap();
        // No explicit rebuild — search should build it on demand
        let results = search(root, "xyzzy").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn check_index_health_false_before_init() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!check_index_health(dir.path()).unwrap());
    }

    #[test]
    fn check_index_health_true_after_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".engram/memory")).unwrap();
        rebuild_index(root).unwrap();
        assert!(check_index_health(root).unwrap());
    }
}
