//! Ties the vault on disk to the SQLite index.
//!
//! Two responsibilities:
//!
//!  * **Reconcile** — walk the vault, assign missing `id` frontmatter, and
//!    bring the index in line with what is actually on disk. This runs at
//!    startup and is also the Obsidian vault importer: pointing Storm at an
//!    existing vault and pointing it at a fresh one are the same code path.
//!  * **Write** — the `PUT` path, including the 3-way merge. This is the only
//!    place note content is written.

use anyhow::{Result, anyhow};
use serde::Serialize;
use uuid::Uuid;

use crate::db::{self, Db, NoteRow};
use crate::frontmatter;
use crate::merge;
use crate::parse;
use crate::vault::{self, Vault};

/// Timestamps are excluded from merges by normalising them to this first.
///
/// Without it, every concurrent write would conflict on the `modified:` line —
/// the server rewrites it on each save, so it differs between base and server
/// in every single reconciliation. The server owns this field; clients never
/// send a meaningful value for it.
const VOLATILE: &str = "@@storm-modified@@";

pub struct Indexer {
    pub vault: Vault,
    pub db: Db,
}

#[derive(Debug, Default, Serialize)]
pub struct ReconcileReport {
    pub scanned: usize,
    pub ids_assigned: usize,
    pub indexed: usize,
    pub updated: usize,
    pub removed: usize,
    pub attachments: usize,
    /// Paths that would get `id` frontmatter, populated only on a dry run.
    pub would_assign: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct WriteResult {
    pub note: NoteRow,
    pub content: String,
    pub seq: i64,
    /// True when the server merged and the client must adopt `content`.
    pub merged: bool,
    /// True when the merge conflicted and `content` carries markers.
    pub conflict: bool,
}

impl Indexer {
    pub fn new(vault: Vault, db: Db) -> Self {
        Self { vault, db }
    }

    /// Brings the index in line with the vault on disk.
    ///
    /// Safe to run repeatedly. With `dry_run`, nothing is written to disk or
    /// to the database — it only reports what it *would* do, which is what
    /// makes it safe to point at a real vault for the first time.
    pub fn reconcile(&mut self, dry_run: bool) -> Result<ReconcileReport> {
        let mut report = ReconcileReport::default();
        let files = self.vault.scan()?;
        report.scanned = files.len();

        let mut seen_ids = Vec::new();

        for file in files {
            let parsed = frontmatter::parse(&file.content);
            let existing_id = parsed
                .frontmatter
                .as_ref()
                .and_then(|fm| frontmatter::get_scalar(&fm.raw, "id"))
                .filter(|s| !s.is_empty());

            let (id, content) = match existing_id {
                Some(id) => (id, file.content.clone()),
                None => {
                    report.ids_assigned += 1;
                    if dry_run {
                        report.would_assign.push(file.rel_path.clone());
                        seen_ids.push(Uuid::new_v4().to_string());
                        continue;
                    }
                    let id = Uuid::new_v4().to_string();
                    let now = now_rfc3339();
                    let stamped = frontmatter::set_scalars(
                        &file.content,
                        &[("id", &id), ("created", &now), ("modified", &now)],
                    );
                    self.vault.write(&file.rel_path, &stamped)?;
                    (id, stamped)
                }
            };

            seen_ids.push(id.clone());
            if dry_run {
                continue;
            }

            let hash = vault::hash(&content);
            let existing = self.db.get_note(&id)?;

            // Unchanged and already indexed at this path — nothing to do.
            if existing
                .as_ref()
                .is_some_and(|p| p.content_hash == hash && p.path == file.rel_path)
            {
                continue;
            }

            let version = existing.as_ref().map(|p| p.version + 1).unwrap_or(1);
            let kind = if existing.is_some() {
                db::KIND_UPDATED
            } else {
                db::KIND_CREATED
            };
            if existing.is_some() {
                report.updated += 1;
            } else {
                report.indexed += 1;
            }

            let row = self.row_for(&id, &file.rel_path, &content, version, existing.as_ref());
            let extracted = parse::extract(&content);
            self.db
                .record_note(&row, &content, &extracted, kind, None)?;
        }

        if !dry_run {
            // Attachments are indexed too, so the vault's binaries are visible
            // to clients without a second store.
            for rel in self.vault.scan_attachments()? {
                if let Ok(bytes) = self.vault.read_bytes(&rel) {
                    self.db.record_attachment(
                        &rel,
                        blake3::hash(&bytes).to_hex().as_ref(),
                        bytes.len() as i64,
                        &now_rfc3339(),
                    )?;
                    report.attachments += 1;
                }
            }

            // Anything indexed but no longer on disk was deleted externally.
            for note in self.db.list_notes()? {
                if !seen_ids.contains(&note.id) {
                    self.db.delete_note(&note.id, &now_rfc3339())?;
                    report.removed += 1;
                }
            }
        }

        Ok(report)
    }

    /// Builds an index row, carrying `created` forward from any previous one.
    fn row_for(
        &self,
        id: &str,
        path: &str,
        content: &str,
        version: i64,
        previous: Option<&NoteRow>,
    ) -> NoteRow {
        let fm = frontmatter::parse(content);
        let raw = fm
            .frontmatter
            .as_ref()
            .map(|f| f.raw.as_str())
            .unwrap_or("");
        let now = now_rfc3339();

        let created = previous
            .map(|p| p.created.clone())
            .or_else(|| frontmatter::get_scalar(raw, "created"))
            .unwrap_or_else(|| now.clone());

        NoteRow {
            id: id.to_string(),
            path: path.to_string(),
            title: parse::title_for(content, path),
            version,
            content_hash: vault::hash(content),
            created,
            modified: now,
            size: content.len() as i64,
        }
    }

    pub fn create_note(&mut self, path: &str, content: &str) -> Result<WriteResult> {
        if self.vault.exists(path) {
            return Err(anyhow!("a note already exists at {path}"));
        }
        let id = Uuid::new_v4().to_string();
        let now = now_rfc3339();
        let stamped = frontmatter::set_scalars(
            content,
            &[("id", &id), ("created", &now), ("modified", &now)],
        );

        self.vault.write(path, &stamped)?;
        let row = self.row_for(&id, path, &stamped, 1, None);
        let extracted = parse::extract(&stamped);
        let seq = self
            .db
            .record_note(&row, &stamped, &extracted, db::KIND_CREATED, None)?;

        Ok(WriteResult {
            note: row,
            content: stamped,
            seq,
            merged: false,
            conflict: false,
        })
    }

    /// The `PUT` path: reconcile a client's edit against the server's state.
    ///
    /// `base_version` is the version the client last saw. When it matches, the
    /// write is a fast-forward. When it doesn't, the client's text is merged
    /// against the server's using the base version's stored content.
    pub fn put_note(
        &mut self,
        id: &str,
        base_version: i64,
        client_content: &str,
        device_id: Option<&str>,
    ) -> Result<WriteResult> {
        let current = self
            .db
            .get_note(id)?
            .ok_or_else(|| anyhow!("no note with id {id}"))?;
        let server_content = self.vault.read(&current.path)?;

        let outcome = if base_version == current.version {
            merge::Outcome::FastForward(client_content.to_string())
        } else {
            // Normalise the volatile timestamp on all three sides so it never
            // participates in the merge.
            let ours = normalise(client_content);
            let theirs = normalise(&server_content);
            let base = match self.db.version_content(id, base_version)? {
                Some(b) => normalise(&b),
                // History was pruned, so there is no base to merge from.
                // An empty base makes diff3 treat both sides as additions,
                // which surfaces an honest conflict rather than silently
                // discarding one device's work.
                None => String::new(),
            };
            merge::reconcile(&base, &theirs, &ours)
        };

        let version = current.version + 1;
        let now = now_rfc3339();
        let final_content =
            frontmatter::set_scalars(outcome.text(), &[("id", id), ("modified", &now)]);

        self.vault.write(&current.path, &final_content)?;
        let row = self.row_for(id, &current.path, &final_content, version, Some(&current));
        let extracted = parse::extract(&final_content);
        let seq = self.db.record_note(
            &row,
            &final_content,
            &extracted,
            db::KIND_UPDATED,
            device_id,
        )?;

        Ok(WriteResult {
            note: row,
            content: final_content,
            seq,
            merged: outcome.rewrites_client(),
            conflict: outcome.is_conflicted(),
        })
    }

    /// Folders the user created explicitly, which pruning must not remove.
    ///
    /// Read fresh on each mutation rather than cached: the set changes through
    /// the folder API, and a stale copy would either delete a folder someone
    /// just made or strand one they just removed.
    fn protected_folders(&self) -> std::collections::HashSet<String> {
        self.db
            .list_folders()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    pub fn move_note(&mut self, id: &str, new_path: &str) -> Result<WriteResult> {
        let current = self
            .db
            .get_note(id)?
            .ok_or_else(|| anyhow!("no note with id {id}"))?;
        self.vault
            .rename(&current.path, new_path, &self.protected_folders())?;

        let content = self.vault.read(new_path)?;
        let row = self.row_for(id, new_path, &content, current.version + 1, Some(&current));
        let extracted = parse::extract(&content);
        let seq = self
            .db
            .record_note(&row, &content, &extracted, db::KIND_MOVED, None)?;

        Ok(WriteResult {
            note: row,
            content,
            seq,
            merged: false,
            conflict: false,
        })
    }

    pub fn delete_note(&mut self, id: &str) -> Result<i64> {
        let current = self
            .db
            .get_note(id)?
            .ok_or_else(|| anyhow!("no note with id {id}"))?;
        self.vault
            .delete(&current.path, &self.protected_folders())?;
        self.db.delete_note(id, &now_rfc3339())
    }

    // ---- folders -------------------------------------------------------

    /// Creates a folder and records it, so pruning leaves it alone.
    ///
    /// Recording is the whole point: a folder that exists only as a directory
    /// disappears the moment the tree around it is tidied.
    pub fn create_folder(&mut self, path: &str) -> Result<()> {
        let clean = normalise_folder(path)?;
        if self.vault.exists(&clean) {
            return Err(anyhow!("a note already exists at {clean}"));
        }
        self.vault.create_dir(&clean)?;
        self.db.record_folder(&clean, &now_rfc3339())?;
        Ok(())
    }

    /// Removes an empty folder. Refuses while it still holds notes — deleting
    /// someone's notes because they tapped "delete folder" is not a thing this
    /// should ever do silently.
    pub fn delete_folder(&mut self, path: &str) -> Result<()> {
        let clean = normalise_folder(path)?;
        let contained = self.db.notes_under(&clean)?;
        if !contained.is_empty() {
            return Err(anyhow!(
                "“{clean}” still holds {} note{}",
                contained.len(),
                if contained.len() == 1 { "" } else { "s" }
            ));
        }
        self.vault.remove_dir(&clean)?;
        self.db.forget_folder(&clean)?;
        Ok(())
    }

    /// Renames a folder and rewrites every contained note's path.
    ///
    /// One directory rename, then one index update per note, each appending a
    /// `moved` change so clients follow. Notes keep their ids — a move is a
    /// metadata change, not delete-plus-create.
    pub fn rename_folder(&mut self, from: &str, to: &str) -> Result<Vec<i64>> {
        let from = normalise_folder(from)?;
        let to = normalise_folder(to)?;
        if from == to {
            return Ok(Vec::new());
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(anyhow!("cannot move a folder inside itself"));
        }

        let notes = self.db.notes_under(&from)?;
        self.vault.rename_dir(&from, &to)?;
        self.db.rename_folder_records(&from, &to)?;

        let mut seqs = Vec::with_capacity(notes.len());
        for note in notes {
            let new_path = format!("{to}{}", &note.path[from.len()..]);
            let content = self.vault.read(&new_path)?;
            let row = self.row_for(&note.id, &new_path, &content, note.version + 1, Some(&note));
            let extracted = parse::extract(&content);
            seqs.push(
                self.db
                    .record_note(&row, &content, &extracted, db::KIND_MOVED, None)?,
            );
        }
        Ok(seqs)
    }

    /// Every folder in the vault: the ones derived from note paths, plus the
    /// ones created explicitly. Only the union makes an empty folder visible.
    pub fn all_folders(&self) -> Result<Vec<String>> {
        let mut folders: Vec<String> = self
            .db
            .list_notes()?
            .iter()
            .flat_map(|n| {
                let parts: Vec<&str> = n.path.split('/').collect();
                (1..parts.len())
                    .map(|i| parts[..i].join("/"))
                    .collect::<Vec<_>>()
            })
            .chain(self.db.list_folders()?)
            .collect();
        folders.sort();
        folders.dedup();
        Ok(folders)
    }

    /// Stores an attachment and indexes it.
    ///
    /// Attachments are opaque blobs: no parsing, no merge, last write wins by
    /// modification time. They live in the vault beside the notes, so the
    /// whole thing stays one tree of ordinary files.
    pub fn put_attachment(&mut self, rel_path: &str, bytes: &[u8]) -> Result<()> {
        self.vault.write_bytes(rel_path, bytes)?;
        self.db.record_attachment(
            rel_path,
            blake3::hash(bytes).to_hex().as_ref(),
            bytes.len() as i64,
            &now_rfc3339(),
        )
    }

    pub fn attachment(&self, rel_path: &str) -> Result<Vec<u8>> {
        self.vault.read_bytes(rel_path)
    }

    pub fn delete_attachment(&mut self, rel_path: &str) -> Result<()> {
        self.vault.delete(rel_path, &self.protected_folders())?;
        self.db.remove_attachment(rel_path)
    }

    /// Re-reads one file after the watcher saw it change externally.
    ///
    /// Returns the new change seq, or `None` if the content was unchanged
    /// (which is how the server's own writes are filtered out).
    pub fn refresh_path(&mut self, rel_path: &str) -> Result<Option<i64>> {
        if !self.vault.exists(rel_path) {
            if let Some(note) = self.db.get_note_by_path(rel_path)? {
                return self.db.delete_note(&note.id, &now_rfc3339()).map(Some);
            }
            return Ok(None);
        }

        let content = self.vault.read(rel_path)?;
        let hash = vault::hash(&content);

        if let Some(existing) = self.db.get_note_by_path(rel_path)? {
            if existing.content_hash == hash {
                return Ok(None);
            }
            let row = self.row_for(
                &existing.id,
                rel_path,
                &content,
                existing.version + 1,
                Some(&existing),
            );
            let extracted = parse::extract(&content);
            return self
                .db
                .record_note(&row, &content, &extracted, db::KIND_UPDATED, None)
                .map(Some);
        }

        // A file that appeared from outside — give it an id and index it.
        let parsed = frontmatter::parse(&content);
        let id = parsed
            .frontmatter
            .as_ref()
            .and_then(|fm| frontmatter::get_scalar(&fm.raw, "id"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let now = now_rfc3339();
        let stamped = frontmatter::set_scalars(&content, &[("id", &id), ("modified", &now)]);
        if stamped != content {
            self.vault.write(rel_path, &stamped)?;
        }

        let row = self.row_for(&id, rel_path, &stamped, 1, None);
        let extracted = parse::extract(&stamped);
        self.db
            .record_note(&row, &stamped, &extracted, db::KIND_CREATED, None)
            .map(Some)
    }
}

/// Blanks the server-owned `modified` field so it can't cause a conflict.
fn normalise(content: &str) -> String {
    frontmatter::set_scalars(content, &[("modified", VOLATILE)])
}

/// Canonical form for a folder path: no leading or trailing slash, and
/// validated by the same rules as a note path.
///
/// `Vault::resolve` already rejects `..`, absolute paths and dotted segments;
/// running the folder through it here means the folder API cannot be a way
/// around the checks the note API enforces.
fn normalise_folder(path: &str) -> Result<String> {
    let clean = path.trim().trim_matches('/');
    if clean.is_empty() {
        return Err(anyhow!("the vault root is not a folder you can change"));
    }
    Ok(clean.to_string())
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexer() -> (tempdir::TempDir, Indexer) {
        let dir = tempdir::TempDir::new("storm-index").unwrap();
        let vault = Vault::new(dir.path()).unwrap();
        let db = Db::open_in_memory().unwrap();
        (dir, Indexer::new(vault, db))
    }

    #[test]
    fn imports_an_existing_obsidian_vault() {
        let (_d, mut ix) = indexer();
        // Notes as Obsidian leaves them: no id, arbitrary frontmatter, none.
        ix.vault
            .write("A.md", "# Note A\n\nlinks to [[Note B]] #homelab\n")
            .unwrap();
        ix.vault
            .write("Folder/B.md", "---\naliases: [b]\n---\n\n# Note B\n")
            .unwrap();

        let report = ix.reconcile(false).unwrap();
        assert_eq!(report.scanned, 2);
        assert_eq!(report.ids_assigned, 2);
        assert_eq!(report.indexed, 2);

        // Both files now carry a stable id, and the user's own keys survive.
        let b = ix.vault.read("Folder/B.md").unwrap();
        assert!(b.contains("aliases: [b]"));
        assert!(
            frontmatter::get_scalar(&frontmatter::parse(&b).frontmatter.unwrap().raw, "id")
                .is_some()
        );

        assert_eq!(ix.db.list_notes().unwrap().len(), 2);
        assert_eq!(ix.db.backlinks("Note B").unwrap().len(), 1);
    }

    #[test]
    fn dry_run_touches_nothing() {
        let (_d, mut ix) = indexer();
        let original = "# Note\n\nbody\n";
        ix.vault.write("A.md", original).unwrap();

        let report = ix.reconcile(true).unwrap();
        assert_eq!(report.scanned, 1);
        assert_eq!(report.ids_assigned, 1);
        assert_eq!(report.would_assign, vec!["A.md"]);

        assert_eq!(ix.vault.read("A.md").unwrap(), original);
        assert!(ix.db.list_notes().unwrap().is_empty());
    }

    #[test]
    fn reconcile_is_idempotent() {
        let (_d, mut ix) = indexer();
        ix.vault.write("A.md", "# A\n").unwrap();

        ix.reconcile(false).unwrap();
        let after_first = ix.vault.read("A.md").unwrap();

        let second = ix.reconcile(false).unwrap();
        assert_eq!(second.ids_assigned, 0);
        assert_eq!(second.indexed, 0);
        assert_eq!(second.updated, 0);
        // Crucially, a rescan must not rewrite files.
        assert_eq!(ix.vault.read("A.md").unwrap(), after_first);
    }

    #[test]
    fn reconcile_notices_external_deletion() {
        let (_d, mut ix) = indexer();
        ix.vault.write("A.md", "# A\n").unwrap();
        ix.reconcile(false).unwrap();

        ix.vault.delete("A.md", &Default::default()).unwrap();
        let report = ix.reconcile(false).unwrap();
        assert_eq!(report.removed, 1);
        assert!(ix.db.list_notes().unwrap().is_empty());
    }

    #[test]
    fn fast_forward_write() {
        let (_d, mut ix) = indexer();
        let created = ix.create_note("A.md", "# A\n\noriginal\n").unwrap();

        let result = ix
            .put_note(
                &created.note.id,
                created.note.version,
                "# A\n\nedited\n",
                None,
            )
            .unwrap();

        assert!(!result.merged);
        assert!(!result.conflict);
        assert!(result.content.contains("edited"));
        assert_eq!(result.note.version, 2);
    }

    #[test]
    fn merges_a_stale_write_to_a_different_region() {
        let (_d, mut ix) = indexer();
        let body = "# A\n\nAlpha.\n\nBeta.\n\nGamma.\n\nDelta.\n\nEpsilon.\n";
        let created = ix.create_note("A.md", body).unwrap();
        let base_version = created.note.version;

        // Another device edits the top and lands first.
        let their_edit = created
            .content
            .replace("Alpha.", "Alpha edited on desktop.");
        ix.put_note(&created.note.id, base_version, &their_edit, None)
            .unwrap();

        // Our offline device writes against the now-stale base, editing the end.
        let our_edit = created
            .content
            .replace("Epsilon.", "Epsilon edited on phone.");
        let result = ix
            .put_note(&created.note.id, base_version, &our_edit, None)
            .unwrap();

        assert!(result.merged, "should have merged");
        assert!(!result.conflict, "should not conflict: {}", result.content);
        assert!(result.content.contains("edited on desktop"));
        assert!(result.content.contains("edited on phone"));
    }

    #[test]
    fn conflicting_writes_keep_both_sides_and_stay_recoverable() {
        let (_d, mut ix) = indexer();
        let created = ix.create_note("A.md", "# A\n\nShared line.\n").unwrap();
        let base_version = created.note.version;

        let theirs = created.content.replace("Shared line.", "Desktop version.");
        let after_theirs = ix
            .put_note(&created.note.id, base_version, &theirs, None)
            .unwrap();

        let ours = created.content.replace("Shared line.", "Phone version.");
        let result = ix
            .put_note(&created.note.id, base_version, &ours, None)
            .unwrap();

        assert!(result.conflict);
        assert!(result.content.contains("Desktop version."));
        assert!(result.content.contains("Phone version."));
        assert!(result.content.contains("<<<<<<<"));

        // The pre-merge server text is still retrievable from history.
        let recovered = ix
            .db
            .version_content(&created.note.id, after_theirs.note.version)
            .unwrap()
            .unwrap();
        assert!(recovered.contains("Desktop version."));
    }

    #[test]
    fn the_modified_timestamp_never_causes_a_conflict() {
        // Regression guard. The server rewrites `modified:` on every save, so
        // without normalisation that line differs between base and server in
        // every reconciliation and every concurrent write would conflict.
        let (_d, mut ix) = indexer();
        let body = "# A\n\nAlpha.\n\nBeta.\n\nGamma.\n\nDelta.\n\nEpsilon.\n";
        let created = ix.create_note("A.md", body).unwrap();
        let base_version = created.note.version;

        let theirs = created.content.replace("Alpha.", "Alpha desktop.");
        ix.put_note(&created.note.id, base_version, &theirs, None)
            .unwrap();

        let ours = created.content.replace("Epsilon.", "Epsilon phone.");
        let result = ix
            .put_note(&created.note.id, base_version, &ours, None)
            .unwrap();

        assert!(
            !result.conflict,
            "timestamp leaked into the merge: {}",
            result.content
        );
        // Exactly one modified line survives, and it is the server's.
        assert_eq!(result.content.matches("modified:").count(), 1);
        assert!(!result.content.contains(VOLATILE));
    }

    #[test]
    fn moving_a_note_keeps_its_identity() {
        // Scenario 6: this is the whole point of tracking by UUID.
        let (_d, mut ix) = indexer();
        let created = ix.create_note("Old/A.md", "# A\n\nbody\n").unwrap();
        let id = created.note.id.clone();

        ix.move_note(&id, "New/B.md").unwrap();

        let got = ix.db.get_note(&id).unwrap().unwrap();
        assert_eq!(got.path, "New/B.md");
        assert!(ix.vault.exists("New/B.md"));
        assert!(!ix.vault.exists("Old/A.md"));

        // And a subsequent edit still finds it by id.
        let after = ix
            .put_note(&id, got.version, "# A\n\nedited\n", None)
            .unwrap();
        assert!(after.content.contains("edited"));
    }

    #[test]
    fn refresh_ignores_our_own_writes() {
        // The watcher calls this for every change including ones we made. An
        // unchanged hash must not produce a spurious version bump.
        let (_d, mut ix) = indexer();
        let created = ix.create_note("A.md", "# A\n").unwrap();
        assert_eq!(ix.refresh_path("A.md").unwrap(), None);
        assert_eq!(
            ix.db.get_note(&created.note.id).unwrap().unwrap().version,
            1
        );
    }

    #[test]
    fn refresh_picks_up_an_external_edit() {
        // Scenario 7: someone edits a note with nvim on the server.
        let (_d, mut ix) = indexer();
        let created = ix.create_note("A.md", "# A\n\noriginal\n").unwrap();

        let edited = created.content.replace("original", "edited in nvim");
        ix.vault.write("A.md", &edited).unwrap();

        assert!(ix.refresh_path("A.md").unwrap().is_some());
        let got = ix.db.get_note(&created.note.id).unwrap().unwrap();
        assert_eq!(got.version, 2);
    }

    #[test]
    fn refresh_indexes_a_brand_new_external_file() {
        let (_d, mut ix) = indexer();
        ix.vault.write("New.md", "# Brand new\n\n#tag\n").unwrap();

        assert!(ix.refresh_path("New.md").unwrap().is_some());
        let notes = ix.db.list_notes().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "Brand new");
        assert!(ix.vault.read("New.md").unwrap().contains("id:"));
    }

    #[test]
    fn refresh_handles_external_deletion() {
        let (_d, mut ix) = indexer();
        let created = ix.create_note("A.md", "# A\n").unwrap();
        ix.vault.delete("A.md", &Default::default()).unwrap();

        assert!(ix.refresh_path("A.md").unwrap().is_some());
        assert!(ix.db.get_note(&created.note.id).unwrap().is_none());
    }

    #[test]
    fn attachments_round_trip_as_opaque_bytes() {
        let (_d, mut ix) = indexer();
        // Deliberately not valid UTF-8 — attachments are blobs, and reading
        // them as text would corrupt every image in the vault.
        let bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0xFF, 0xFE, 0x01];

        ix.put_attachment("attachments/logo.png", &bytes).unwrap();
        assert_eq!(ix.attachment("attachments/logo.png").unwrap(), bytes);
    }

    #[test]
    fn attachments_are_indexed_and_removable() {
        let (_d, mut ix) = indexer();
        ix.put_attachment("attachments/a.png", &[1, 2, 3]).unwrap();
        ix.put_attachment("attachments/b.pdf", &[4, 5]).unwrap();

        let listed = ix.db.list_attachments().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].size, 3);

        ix.delete_attachment("attachments/a.png").unwrap();
        assert_eq!(ix.db.list_attachments().unwrap().len(), 1);
        assert!(!ix.vault.exists("attachments/a.png"));
    }

    #[test]
    fn a_reupload_replaces_the_bytes() {
        // Last write wins: attachments are not merged.
        let (_d, mut ix) = indexer();
        ix.put_attachment("attachments/a.png", &[1, 2, 3]).unwrap();
        ix.put_attachment("attachments/a.png", &[9]).unwrap();

        assert_eq!(ix.attachment("attachments/a.png").unwrap(), vec![9]);
        assert_eq!(ix.db.list_attachments().unwrap()[0].size, 1);
    }

    #[test]
    fn the_scan_indexes_attachments_already_on_disk() {
        let (_d, mut ix) = indexer();
        ix.vault
            .write_bytes("attachments/existing.png", &[1, 2])
            .unwrap();
        ix.vault.write("Note.md", "# Note\n").unwrap();

        let report = ix.reconcile(false).unwrap();
        assert_eq!(report.scanned, 1, "only markdown counts as a note");
        assert_eq!(report.attachments, 1);
        assert_eq!(ix.db.list_attachments().unwrap().len(), 1);
    }

    #[test]
    fn attachments_cannot_escape_the_vault() {
        let (_d, mut ix) = indexer();
        assert!(ix.put_attachment("../escape.png", &[1]).is_err());
        assert!(ix.put_attachment("/etc/passwd", &[1]).is_err());
    }

    #[test]
    fn create_refuses_to_overwrite() {
        let (_d, mut ix) = indexer();
        ix.create_note("A.md", "# A\n").unwrap();
        assert!(ix.create_note("A.md", "# Other\n").is_err());
    }

    #[test]
    fn delete_removes_the_file_and_the_index_row() {
        let (_d, mut ix) = indexer();
        let created = ix.create_note("A.md", "# A\n").unwrap();
        ix.delete_note(&created.note.id).unwrap();
        assert!(!ix.vault.exists("A.md"));
        assert!(ix.db.get_note(&created.note.id).unwrap().is_none());
    }

    // ---- folders -------------------------------------------------------

    #[test]
    fn a_created_folder_survives_the_delete_that_empties_it() {
        // The regression this whole feature turns on. Before the `folders`
        // table, creating a folder and removing its only note deleted the
        // folder too — silently, at the next tidy-up.
        let (_d, mut ix) = indexer();
        ix.create_folder("Projects").unwrap();
        let note = ix.create_note("Projects/A.md", "# A\n").unwrap();

        ix.delete_note(&note.note.id).unwrap();

        assert!(ix.all_folders().unwrap().contains(&"Projects".to_string()));
    }

    #[test]
    fn a_derived_folder_is_still_pruned_when_emptied() {
        // Folders that only ever existed because a note was inside them keep
        // the old behaviour — otherwise every transient directory sticks.
        let (_d, mut ix) = indexer();
        let note = ix.create_note("Derived/A.md", "# A\n").unwrap();
        ix.delete_note(&note.note.id).unwrap();
        assert!(!ix.all_folders().unwrap().contains(&"Derived".to_string()));
    }

    #[test]
    fn an_empty_created_folder_appears_in_the_tree() {
        let (_d, mut ix) = indexer();
        ix.create_folder("Empty").unwrap();
        assert_eq!(ix.all_folders().unwrap(), vec!["Empty".to_string()]);
    }

    #[test]
    fn folders_are_the_union_of_derived_and_created() {
        let (_d, mut ix) = indexer();
        ix.create_note("Daily/2026/A.md", "# A\n").unwrap();
        ix.create_folder("Archive").unwrap();

        let folders = ix.all_folders().unwrap();
        assert_eq!(folders, vec!["Archive", "Daily", "Daily/2026"]);
    }

    #[test]
    fn deleting_a_folder_refuses_while_it_holds_notes() {
        let (_d, mut ix) = indexer();
        ix.create_folder("Projects").unwrap();
        let note = ix.create_note("Projects/A.md", "# A\n").unwrap();

        let err = ix.delete_folder("Projects").unwrap_err().to_string();
        assert!(err.contains("1 note"), "unhelpful message: {err}");
        assert!(ix.vault.exists("Projects/A.md"), "notes must be untouched");

        ix.delete_note(&note.note.id).unwrap();
        ix.delete_folder("Projects").unwrap();
        assert!(ix.all_folders().unwrap().is_empty());
    }

    #[test]
    fn renaming_a_folder_rewrites_every_contained_note_path() {
        let (_d, mut ix) = indexer();
        let a = ix.create_note("Old/A.md", "# A\n").unwrap();
        let b = ix.create_note("Old/Sub/B.md", "# B\n").unwrap();

        let seqs = ix.rename_folder("Old", "New").unwrap();
        assert_eq!(seqs.len(), 2, "one `moved` change per note");

        // Ids survive: a move is a metadata change, not delete-plus-create.
        assert_eq!(
            ix.db.get_note(&a.note.id).unwrap().unwrap().path,
            "New/A.md"
        );
        assert_eq!(
            ix.db.get_note(&b.note.id).unwrap().unwrap().path,
            "New/Sub/B.md"
        );
        assert!(ix.vault.exists("New/Sub/B.md"));
        assert!(!ix.vault.exists("Old/A.md"));
    }

    #[test]
    fn renaming_a_folder_carries_its_created_record() {
        let (_d, mut ix) = indexer();
        ix.create_folder("Old").unwrap();
        ix.create_folder("Old/Inner").unwrap();

        ix.rename_folder("Old", "New").unwrap();

        let folders = ix.all_folders().unwrap();
        assert!(folders.contains(&"New".to_string()));
        assert!(folders.contains(&"New/Inner".to_string()));
        assert!(!folders.iter().any(|f| f.starts_with("Old")));
    }

    #[test]
    fn a_folder_cannot_be_moved_inside_itself() {
        let (_d, mut ix) = indexer();
        ix.create_note("Projects/A.md", "# A\n").unwrap();
        assert!(ix.rename_folder("Projects", "Projects/Nested").is_err());
        assert!(ix.vault.exists("Projects/A.md"));
    }

    #[test]
    fn folder_paths_go_through_the_same_checks_as_note_paths() {
        let (_d, mut ix) = indexer();
        for bad in ["", "/", "   ", ".."] {
            assert!(
                ix.create_folder(bad).is_err(),
                "should have rejected {bad:?}"
            );
        }
        // Escapes are caught by `Vault::resolve`, which the folder API reuses
        // rather than reimplementing.
        assert!(ix.create_folder("../escape").is_err());
        assert!(ix.create_folder(".hidden").is_err());
    }

    #[test]
    fn trailing_slashes_do_not_make_a_second_folder() {
        let (_d, mut ix) = indexer();
        ix.create_folder("Projects").unwrap();
        ix.create_folder("Projects/").unwrap();
        assert_eq!(ix.all_folders().unwrap(), vec!["Projects".to_string()]);
    }

    // ---- recents -------------------------------------------------------

    #[test]
    fn opening_a_note_does_not_change_it() {
        // Opening is not an edit. If it bumped `version` or wrote to the
        // change log, every device would resync because someone looked.
        let (_d, mut ix) = indexer();
        let created = ix.create_note("A.md", "# A\n").unwrap();
        let seq_before = ix.db.latest_seq().unwrap();

        ix.db
            .touch_note_access(&created.note.id, "2026-08-07T10:00:00Z")
            .unwrap();

        let after = ix.db.get_note(&created.note.id).unwrap().unwrap();
        assert_eq!(after.version, created.note.version);
        assert_eq!(ix.db.latest_seq().unwrap(), seq_before);
    }

    #[test]
    fn recents_come_back_newest_first() {
        let (_d, mut ix) = indexer();
        let a = ix.create_note("A.md", "# A\n").unwrap();
        let b = ix.create_note("B.md", "# B\n").unwrap();

        ix.db
            .touch_note_access(&a.note.id, "2026-08-07T09:00:00Z")
            .unwrap();
        ix.db
            .touch_note_access(&b.note.id, "2026-08-07T11:00:00Z")
            .unwrap();

        let recents = ix.db.recent_notes(10).unwrap();
        assert_eq!(recents[0].note_id, b.note.id);
        assert_eq!(recents[1].note_id, a.note.id);

        // Re-opening moves it to the top rather than adding a second row.
        ix.db
            .touch_note_access(&a.note.id, "2026-08-07T12:00:00Z")
            .unwrap();
        let recents = ix.db.recent_notes(10).unwrap();
        assert_eq!(recents.len(), 2);
        assert_eq!(recents[0].note_id, a.note.id);
    }

    #[test]
    fn a_deleted_note_leaves_the_recents_list() {
        let (_d, mut ix) = indexer();
        let created = ix.create_note("A.md", "# A\n").unwrap();
        ix.db
            .touch_note_access(&created.note.id, "2026-08-07T10:00:00Z")
            .unwrap();

        ix.delete_note(&created.note.id).unwrap();
        assert!(ix.db.recent_notes(10).unwrap().is_empty());
    }
}
