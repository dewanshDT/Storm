//! SQLite index, version history, and change log.
//!
//! The markdown files on disk are the content store; this database is a
//! *derived* index that can be rebuilt from them at any time. The one thing it
//! holds that the files don't is version history, which the 3-way merge needs
//! as its base — see [`crate::merge`].
//!
//! It lives in `state/` beside the vault rather than inside it, so the vault
//! directory stays pure markdown: greppable, rsync-able, and readable by
//! Obsidian if Storm ever needs to be abandoned.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::path::Path;

pub struct Db {
    conn: Connection,
}

/// A note's indexed metadata. Content lives on disk, not here.
#[derive(Debug, Clone, Serialize)]
pub struct NoteRow {
    pub id: String,
    pub path: String,
    pub title: String,
    pub version: i64,
    pub content_hash: String,
    pub created: String,
    pub modified: String,
    pub size: i64,
}

/// One entry in the change log — the delta-sync primitive.
///
/// Clients remember the last `seq` they saw and ask for everything after it,
/// so no client ever has to diff a full vault manifest.
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub seq: i64,
    pub note_id: String,
    pub kind: String,
    pub version: i64,
    pub at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentRow {
    pub path: String,
    pub hash: String,
    pub size: i64,
    pub modified: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub id: String,
    pub path: String,
    pub title: String,
    pub snippet: String,
}

pub const KIND_CREATED: &str = "created";
pub const KIND_UPDATED: &str = "updated";
pub const KIND_DELETED: &str = "deleted";
pub const KIND_MOVED: &str = "moved";

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating state dir {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening database {}", path.display()))?;
        Self::from_connection(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        // WAL keeps the file watcher's writes from blocking API reads.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let db = Self { conn };
        db.assert_fts5()?;
        db.migrate()?;
        Ok(db)
    }

    /// Fails loudly at startup if SQLite was built without FTS5.
    ///
    /// Search is a v1 feature; discovering this at first query instead of at
    /// boot would be a nasty surprise.
    fn assert_fts5(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE VIRTUAL TABLE temp.fts5_probe USING fts5(x); DROP TABLE temp.fts5_probe;",
            )
            .context(
                "SQLite was built without FTS5 support, which Storm needs for search. \
                 Rebuild with the `bundled` feature of rusqlite.",
            )
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS notes (
                id            TEXT PRIMARY KEY,
                path          TEXT NOT NULL UNIQUE,
                title         TEXT NOT NULL DEFAULT '',
                version       INTEGER NOT NULL DEFAULT 1,
                content_hash  TEXT NOT NULL,
                created       TEXT NOT NULL,
                modified      TEXT NOT NULL,
                size          INTEGER NOT NULL DEFAULT 0
            );

            -- Full content snapshots rather than diffs. Notes are kilobytes, and
            -- the merge base has to be a direct lookup on the hot write path.
            CREATE TABLE IF NOT EXISTS note_versions (
                note_id     TEXT NOT NULL,
                version     INTEGER NOT NULL,
                content     BLOB NOT NULL,
                created_at  TEXT NOT NULL,
                device_id   TEXT,
                PRIMARY KEY (note_id, version)
            );

            CREATE TABLE IF NOT EXISTS links (
                src_id        TEXT NOT NULL,
                target_title  TEXT NOT NULL,
                target_id     TEXT,
                PRIMARY KEY (src_id, target_title)
            );
            CREATE INDEX IF NOT EXISTS idx_links_target ON links(target_title);

            CREATE TABLE IF NOT EXISTS tags (
                note_id  TEXT NOT NULL,
                tag      TEXT NOT NULL,
                PRIMARY KEY (note_id, tag)
            );
            CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);

            CREATE TABLE IF NOT EXISTS attachments (
                path      TEXT PRIMARY KEY,
                hash      TEXT NOT NULL,
                size      INTEGER NOT NULL,
                modified  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS devices (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                token_hash  TEXT NOT NULL,
                created     TEXT NOT NULL,
                last_seen   TEXT
            );

            CREATE TABLE IF NOT EXISTS change_log (
                seq      INTEGER PRIMARY KEY AUTOINCREMENT,
                note_id  TEXT NOT NULL,
                kind     TEXT NOT NULL,
                version  INTEGER NOT NULL,
                at       TEXT NOT NULL
            );

            -- Maintained by hand on write. An external-content table would save
            -- space but couples the index to `notes` row ids for no real gain
            -- at vault scale.
            CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                note_id UNINDEXED,
                title,
                body,
                tokenize = 'unicode61 remove_diacritics 2'
            );
            "#,
        )?;
        Ok(())
    }

    // ---- notes ---------------------------------------------------------

    pub fn get_note(&self, id: &str) -> Result<Option<NoteRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, path, title, version, content_hash, created, modified, size
                 FROM notes WHERE id = ?1",
                params![id],
                Self::note_from_row,
            )
            .optional()?)
    }

    pub fn get_note_by_path(&self, path: &str) -> Result<Option<NoteRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, path, title, version, content_hash, created, modified, size
                 FROM notes WHERE path = ?1",
                params![path],
                Self::note_from_row,
            )
            .optional()?)
    }

    pub fn list_notes(&self) -> Result<Vec<NoteRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, title, version, content_hash, created, modified, size
             FROM notes ORDER BY path",
        )?;
        let rows = stmt.query_map([], Self::note_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    fn note_from_row(row: &rusqlite::Row) -> rusqlite::Result<NoteRow> {
        Ok(NoteRow {
            id: row.get(0)?,
            path: row.get(1)?,
            title: row.get(2)?,
            version: row.get(3)?,
            content_hash: row.get(4)?,
            created: row.get(5)?,
            modified: row.get(6)?,
            size: row.get(7)?,
        })
    }

    /// Inserts or updates a note's index row, snapshots the content, reindexes
    /// links/tags/FTS, and appends a change — all in one transaction.
    ///
    /// Doing this atomically is what lets a client trust that any `seq` it sees
    /// corresponds to a fully indexed note.
    #[allow(clippy::too_many_arguments)]
    pub fn record_note(
        &mut self,
        note: &NoteRow,
        content: &str,
        extracted: &crate::parse::Extracted,
        kind: &str,
        device_id: Option<&str>,
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;

        tx.execute(
            "INSERT INTO notes (id, path, title, version, content_hash, created, modified, size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                path = excluded.path,
                title = excluded.title,
                version = excluded.version,
                content_hash = excluded.content_hash,
                modified = excluded.modified,
                size = excluded.size",
            params![
                note.id,
                note.path,
                note.title,
                note.version,
                note.content_hash,
                note.created,
                note.modified,
                note.size
            ],
        )?;

        tx.execute(
            "INSERT OR REPLACE INTO note_versions (note_id, version, content, created_at, device_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![note.id, note.version, content, note.modified, device_id],
        )?;

        tx.execute("DELETE FROM links WHERE src_id = ?1", params![note.id])?;
        for target in &extracted.links {
            tx.execute(
                "INSERT OR IGNORE INTO links (src_id, target_title) VALUES (?1, ?2)",
                params![note.id, target],
            )?;
        }

        tx.execute("DELETE FROM tags WHERE note_id = ?1", params![note.id])?;
        for tag in &extracted.tags {
            tx.execute(
                "INSERT OR IGNORE INTO tags (note_id, tag) VALUES (?1, ?2)",
                params![note.id, tag],
            )?;
        }

        tx.execute("DELETE FROM notes_fts WHERE note_id = ?1", params![note.id])?;
        tx.execute(
            "INSERT INTO notes_fts (note_id, title, body) VALUES (?1, ?2, ?3)",
            params![note.id, note.title, content],
        )?;

        tx.execute(
            "INSERT INTO change_log (note_id, kind, version, at) VALUES (?1, ?2, ?3, ?4)",
            params![note.id, kind, note.version, note.modified],
        )?;
        let seq = tx.last_insert_rowid();

        tx.commit()?;
        Ok(seq)
    }

    pub fn delete_note(&mut self, id: &str, at: &str) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let version: i64 = tx
            .query_row(
                "SELECT version FROM notes WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);

        tx.execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        tx.execute("DELETE FROM links WHERE src_id = ?1", params![id])?;
        tx.execute("DELETE FROM tags WHERE note_id = ?1", params![id])?;
        tx.execute("DELETE FROM notes_fts WHERE note_id = ?1", params![id])?;
        // note_versions is kept deliberately: a delete that arrives while
        // another device was editing must still be recoverable.
        tx.execute(
            "INSERT INTO change_log (note_id, kind, version, at) VALUES (?1, ?2, ?3, ?4)",
            params![id, KIND_DELETED, version, at],
        )?;
        let seq = tx.last_insert_rowid();
        tx.commit()?;
        Ok(seq)
    }

    // ---- attachments ---------------------------------------------------

    pub fn record_attachment(
        &self,
        path: &str,
        hash: &str,
        size: i64,
        modified: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO attachments (path, hash, size, modified)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET
                hash = excluded.hash,
                size = excluded.size,
                modified = excluded.modified",
            params![path, hash, size, modified],
        )?;
        Ok(())
    }

    pub fn remove_attachment(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM attachments WHERE path = ?1", params![path])?;
        Ok(())
    }

    pub fn list_attachments(&self) -> Result<Vec<AttachmentRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, hash, size, modified FROM attachments ORDER BY path")?;
        let rows = stmt.query_map([], |r| {
            Ok(AttachmentRow {
                path: r.get(0)?,
                hash: r.get(1)?,
                size: r.get(2)?,
                modified: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ---- versions ------------------------------------------------------

    /// Fetches the exact text a client says it edited from — the merge base.
    pub fn version_content(&self, note_id: &str, version: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT content FROM note_versions WHERE note_id = ?1 AND version = ?2",
                params![note_id, version],
                |r| r.get(0),
            )
            .optional()?)
    }

    // ---- change log ----------------------------------------------------

    pub fn changes_since(&self, since: i64, limit: i64) -> Result<Vec<Change>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, note_id, kind, version, at FROM change_log
             WHERE seq > ?1 ORDER BY seq LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![since, limit], |row| {
            Ok(Change {
                seq: row.get(0)?,
                note_id: row.get(1)?,
                kind: row.get(2)?,
                version: row.get(3)?,
                at: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn latest_seq(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COALESCE(MAX(seq), 0) FROM change_log", [], |r| {
                r.get(0)
            })?)
    }

    // ---- search, tags, backlinks ---------------------------------------

    pub fn search(&self, query: &str, limit: i64) -> Result<Vec<SearchHit>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.path, n.title,
                    snippet(notes_fts, 2, '<<', '>>', ' … ', 16)
             FROM notes_fts f JOIN notes n ON n.id = f.note_id
             WHERE notes_fts MATCH ?1
             ORDER BY rank LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit], |row| {
            Ok(SearchHit {
                id: row.get(0)?,
                path: row.get(1)?,
                title: row.get(2)?,
                snippet: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Notes linking *to* `title` — the "linked mentions" panel.
    pub fn backlinks(&self, title: &str) -> Result<Vec<NoteRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.path, n.title, n.version, n.content_hash, n.created, n.modified, n.size
             FROM links l JOIN notes n ON n.id = l.src_id
             WHERE l.target_title = ?1 ORDER BY n.path",
        )?;
        let rows = stmt.query_map(params![title], Self::note_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn all_tags(&self) -> Result<Vec<(String, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT tag, COUNT(*) FROM tags GROUP BY tag ORDER BY tag")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn notes_with_tag(&self, tag: &str) -> Result<Vec<NoteRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.path, n.title, n.version, n.content_hash, n.created, n.modified, n.size
             FROM tags t JOIN notes n ON n.id = t.note_id
             WHERE t.tag = ?1 ORDER BY n.path",
        )?;
        let rows = stmt.query_map(params![tag], Self::note_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn note(id: &str, path: &str, version: i64) -> NoteRow {
        NoteRow {
            id: id.into(),
            path: path.into(),
            title: format!("Title {id}"),
            version,
            content_hash: format!("hash{version}"),
            created: "2026-08-05T10:00:00Z".into(),
            modified: "2026-08-05T10:00:00Z".into(),
            size: 100,
        }
    }

    fn record(db: &mut Db, n: &NoteRow, content: &str, kind: &str) -> i64 {
        let e = parse::extract(content);
        db.record_note(n, content, &e, kind, None).unwrap()
    }

    #[test]
    fn fts5_is_available() {
        // The whole search feature depends on this.
        Db::open_in_memory().unwrap();
    }

    #[test]
    fn records_and_reads_a_note() {
        let mut db = Db::open_in_memory().unwrap();
        let n = note("a", "Notes/A.md", 1);
        record(&mut db, &n, "# A\n\nbody\n", KIND_CREATED);

        let got = db.get_note("a").unwrap().unwrap();
        assert_eq!(got.path, "Notes/A.md");
        assert_eq!(got.version, 1);
        assert!(db.get_note_by_path("Notes/A.md").unwrap().is_some());
    }

    #[test]
    fn keeps_version_history_for_merges() {
        let mut db = Db::open_in_memory().unwrap();
        let mut n = note("a", "A.md", 1);
        record(&mut db, &n, "version one\n", KIND_CREATED);
        n.version = 2;
        record(&mut db, &n, "version two\n", KIND_UPDATED);

        assert_eq!(
            db.version_content("a", 1).unwrap().unwrap(),
            "version one\n"
        );
        assert_eq!(
            db.version_content("a", 2).unwrap().unwrap(),
            "version two\n"
        );
        assert!(db.version_content("a", 9).unwrap().is_none());
    }

    #[test]
    fn change_log_drives_delta_sync() {
        let mut db = Db::open_in_memory().unwrap();
        record(&mut db, &note("a", "A.md", 1), "a\n", KIND_CREATED);
        let s2 = record(&mut db, &note("b", "B.md", 1), "b\n", KIND_CREATED);
        record(&mut db, &note("c", "C.md", 1), "c\n", KIND_CREATED);

        let changes = db.changes_since(s2, 100).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].note_id, "c");
        assert_eq!(db.latest_seq().unwrap(), changes[0].seq);
    }

    #[test]
    fn reindexes_rather_than_accumulating() {
        // An edit that removes a tag must actually remove it from the index.
        let mut db = Db::open_in_memory().unwrap();
        let mut n = note("a", "A.md", 1);
        record(&mut db, &n, "#alpha #beta\n", KIND_CREATED);
        assert_eq!(db.all_tags().unwrap().len(), 2);

        n.version = 2;
        record(&mut db, &n, "#alpha only\n", KIND_UPDATED);
        let tags = db.all_tags().unwrap();
        assert_eq!(tags, vec![("alpha".to_string(), 1)]);
    }

    #[test]
    fn full_text_search_finds_body_text() {
        let mut db = Db::open_in_memory().unwrap();
        record(
            &mut db,
            &note("a", "A.md", 1),
            "the quick brown fox\n",
            KIND_CREATED,
        );
        record(
            &mut db,
            &note("b", "B.md", 1),
            "lazy dog sleeping\n",
            KIND_CREATED,
        );

        let hits = db.search("brown", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "a");
        assert!(hits[0].snippet.contains("<<brown>>"));

        assert_eq!(db.search("nonexistent", 10).unwrap().len(), 0);
    }

    #[test]
    fn backlinks_resolve() {
        let mut db = Db::open_in_memory().unwrap();
        record(
            &mut db,
            &note("a", "A.md", 1),
            "see [[Target Note]]\n",
            KIND_CREATED,
        );
        record(
            &mut db,
            &note("b", "B.md", 1),
            "also [[Target Note]]\n",
            KIND_CREATED,
        );
        record(&mut db, &note("c", "C.md", 1), "unrelated\n", KIND_CREATED);

        let back = db.backlinks("Target Note").unwrap();
        assert_eq!(back.len(), 2);
        assert!(db.backlinks("Nothing").unwrap().is_empty());
    }

    #[test]
    fn tags_are_browsable() {
        let mut db = Db::open_in_memory().unwrap();
        record(
            &mut db,
            &note("a", "A.md", 1),
            "#homelab #proj\n",
            KIND_CREATED,
        );
        record(&mut db, &note("b", "B.md", 1), "#homelab\n", KIND_CREATED);

        assert_eq!(
            db.all_tags().unwrap(),
            vec![("homelab".to_string(), 2), ("proj".to_string(), 1)]
        );
        assert_eq!(db.notes_with_tag("homelab").unwrap().len(), 2);
    }

    #[test]
    fn delete_clears_the_index_but_keeps_history() {
        let mut db = Db::open_in_memory().unwrap();
        record(
            &mut db,
            &note("a", "A.md", 1),
            "#tag [[L]] words\n",
            KIND_CREATED,
        );
        db.delete_note("a", "2026-08-05T11:00:00Z").unwrap();

        assert!(db.get_note("a").unwrap().is_none());
        assert!(db.all_tags().unwrap().is_empty());
        assert!(db.backlinks("L").unwrap().is_empty());
        assert!(db.search("words", 10).unwrap().is_empty());
        // Recoverable if the delete raced another device's edit.
        assert!(db.version_content("a", 1).unwrap().is_some());
    }

    #[test]
    fn a_move_updates_path_without_changing_identity() {
        let mut db = Db::open_in_memory().unwrap();
        record(&mut db, &note("a", "Old/A.md", 1), "body\n", KIND_CREATED);
        let mut moved = note("a", "New/A.md", 2);
        moved.created = "2026-08-05T10:00:00Z".into();
        record(&mut db, &moved, "body\n", KIND_MOVED);

        let got = db.get_note("a").unwrap().unwrap();
        assert_eq!(got.path, "New/A.md");
        assert!(db.get_note_by_path("Old/A.md").unwrap().is_none());
    }
}
