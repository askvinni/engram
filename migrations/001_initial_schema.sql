CREATE TABLE IF NOT EXISTS file_index (
    path      TEXT PRIMARY KEY,
    fts_rowid INTEGER NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    path      UNINDEXED,
    category  UNINDEXED,
    slug      UNINDEXED,
    content,
    tokenize  = 'unicode61'
);
