//! The trash (decision 150).
//!
//! Every delete moves the note or folder to `<vault>/.trash/<trash_id>/`
//! under its original path, and the index keeps who deleted it. The note's
//! row is tombstoned exactly as before, so clients see the same `deleted`
//! change, and its version history stays, so a restore is the same note.
//! Each of these runs under the indexer lock, like any other write.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use serde::Serialize;
use uuid::Uuid;

use crate::db::{self, DeletedBy, NoteRow, TrashItem, TrashRow};
use crate::frontmatter;
use crate::index::{IndexError, IndexResult, Indexer, id_of, normalise_folder, now_rfc3339};
use crate::parse;

/// What a trash entry is kept for when the vault does not say (150b).
pub const DEFAULT_RETENTION_DAYS: i64 = 30;

/// The vault setting, in `_storm/vault.md`: a number of days, or `never`.
pub const RETENTION_KEY: &str = "storm.trash.retention";

const VAULT_CONFIG_PATH: &str = "_storm/vault.md";
const CONFIG_FOLDER: &str = "_storm";

pub const KIND_NOTE: &str = "note";
pub const KIND_FOLDER: &str = "folder";

impl DeletedBy {
    pub fn person(device_id: Option<String>, device_name: Option<String>) -> Self {
        Self {
            kind: "person".into(),
            id: device_id,
            name: device_name,
            ..Self::default()
        }
    }

    pub fn key(key_id: &str) -> Self {
        Self {
            kind: "key".into(),
            id: Some(key_id.into()),
            ..Self::default()
        }
    }

    /// An agent's delete names its host, its session and, under decision
    /// 149, the reason it gave.
    pub fn agent(host_id: &str, session_id: &str, reason: Option<String>) -> Self {
        Self {
            kind: "agent".into(),
            id: Some(host_id.into()),
            session_id: Some(session_id.into()),
            reason,
            ..Self::default()
        }
    }

    /// A file that vanished outside Storm: a sync tool, a shell, another
    /// machine writing the share.
    pub fn sync() -> Self {
        Self {
            kind: "sync".into(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Trashed {
    /// `None` for an empty folder, which is simply removed.
    pub trash_id: Option<String>,
    pub notes: usize,
    #[serde(skip)]
    pub seqs: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrashedNote {
    pub id: String,
    pub path: String,
    pub title: String,
}

/// A trash entry as the Trash view lists it.
#[derive(Debug, Clone, Serialize)]
pub struct TrashEntry {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub original_path: String,
    pub deleted_at: String,
    pub deleted_by: DeletedBy,
    pub note_count: usize,
    pub notes: Vec<TrashedNote>,
    pub attachments: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RestoredNote {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Restored {
    pub notes: Vec<RestoredNote>,
    pub attachments: Vec<String>,
    #[serde(skip)]
    pub seqs: Vec<i64>,
}

fn not_in_trash(id: &str) -> IndexError {
    IndexError::NotFound(format!("nothing in the trash with id {id}"))
}

fn is_markdown(path: &str) -> bool {
    path.ends_with(".md")
}

/// Whether a note's text links to an attachment: by its vault path (a
/// markdown link or image, spaces escaped or not) or, as Obsidian writes
/// it, by file name in a wikilink or embed.
pub fn references(content: &str, attachment: &str) -> bool {
    let name = attachment.rsplit('/').next().unwrap_or(attachment);
    content.contains(attachment)
        || content.contains(&attachment.replace(' ', "%20"))
        || content.contains(&format!("[[{name}"))
}

/// `Dir/Name.md` → `Dir/Name (restored).md`, then `(restored 2)` and on.
fn restored_name(path: &str, n: usize) -> String {
    let (dir, file) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => (&file[..i], &file[i..]),
        _ => (file, ""),
    };
    let tag = if n == 1 {
        "(restored)".to_string()
    } else {
        format!("(restored {n})")
    };
    format!("{dir}{stem} {tag}{ext}")
}

fn display_name(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.strip_suffix(".md").unwrap_or(file).to_string()
}

/// Reads the retention setting's value: days, or `None` for never.
pub fn parse_retention(value: Option<&str>) -> Option<i64> {
    let Some(raw) = value.map(|v| v.trim().trim_matches(['"', '\'']).to_ascii_lowercase()) else {
        return Some(DEFAULT_RETENTION_DAYS);
    };
    if matches!(raw.as_str(), "never" | "off" | "forever") {
        return None;
    }
    match raw.trim_end_matches('d').trim().parse::<i64>() {
        Ok(days) if days > 0 => Some(days),
        _ => Some(DEFAULT_RETENTION_DAYS),
    }
}

impl Indexer {
    /// Moves a note to the trash, with the attachments only it links to.
    pub fn trash_note(&mut self, id: &str, by: &DeletedBy) -> IndexResult<Trashed> {
        let note = self
            .db
            .get_note(id)?
            .ok_or_else(|| IndexError::NotFound(format!("no note with id {id}")))?;
        if !self.vault.exists(&note.path) {
            // Gone from disk already: what the history last held goes instead.
            return Ok(self.keep_vanished(&note, by)?);
        }
        let content = self.vault.read(&note.path).unwrap_or_default();
        let trash_id = Uuid::new_v4().to_string();
        let keep = self.protected_folders();
        let lone = self.lone_attachments(&[(note.id.clone(), content)], None)?;

        self.vault.move_to_trash(&note.path, &trash_id, &keep)?;
        let mut items = vec![TrashItem {
            path: note.path.clone(),
            note_id: Some(note.id.clone()),
            title: Some(note.title.clone()),
        }];
        items.extend(self.move_attachments(&lone, &trash_id, &keep)?);

        let row = TrashRow {
            id: trash_id.clone(),
            kind: KIND_NOTE.into(),
            original_path: note.path,
            deleted_at: now_rfc3339(),
            by: by.clone(),
            folders: Vec::new(),
        };
        let seqs = self.db.trash(&row, &items)?;
        Ok(Trashed {
            trash_id: Some(trash_id),
            notes: 1,
            seqs,
        })
    }

    /// Moves a folder and everything in it to the trash as one entry. An
    /// empty folder has nothing to keep and is simply removed.
    pub fn trash_folder(&mut self, path: &str, by: &DeletedBy) -> IndexResult<Trashed> {
        let clean = normalise_folder(path)?;
        if clean == CONFIG_FOLDER || clean.starts_with(&format!("{CONFIG_FOLDER}/")) {
            return Err(IndexError::Invalid(
                "“_storm” holds the vault's own settings and cannot be deleted".into(),
            ));
        }
        if self.vault.dir_is_empty(&clean)? {
            self.vault.remove_dir(&clean)?;
            self.db.forget_folders_under(&clean)?;
            return Ok(Trashed {
                trash_id: None,
                notes: 0,
                seqs: Vec::new(),
            });
        }

        let notes = self.db.notes_under(&clean)?;
        let contents: Vec<(String, String)> = notes
            .iter()
            .map(|n| (n.id.clone(), self.vault.read(&n.path).unwrap_or_default()))
            .collect();
        let lone = self.lone_attachments(&contents, Some(&clean))?;
        let prefix = format!("{clean}/");
        let inside: Vec<String> = self
            .db
            .list_attachments()?
            .into_iter()
            .map(|a| a.path)
            .filter(|p| p.starts_with(&prefix))
            .collect();

        let trash_id = Uuid::new_v4().to_string();
        let keep = self.protected_folders();
        self.vault.move_to_trash(&clean, &trash_id, &keep)?;
        let folders = self.db.forget_folders_under(&clean)?;

        let mut items: Vec<TrashItem> = notes
            .iter()
            .map(|n| TrashItem {
                path: n.path.clone(),
                note_id: Some(n.id.clone()),
                title: Some(n.title.clone()),
            })
            .collect();
        for a in inside {
            self.db.remove_attachment(&a)?;
            items.push(TrashItem {
                path: a,
                note_id: None,
                title: None,
            });
        }
        items.extend(self.move_attachments(&lone, &trash_id, &keep)?);

        let row = TrashRow {
            id: trash_id.clone(),
            kind: KIND_FOLDER.into(),
            original_path: clean,
            deleted_at: now_rfc3339(),
            by: by.clone(),
            folders,
        };
        let seqs = self.db.trash(&row, &items)?;
        Ok(Trashed {
            trash_id: Some(trash_id),
            notes: notes.len(),
            seqs,
        })
    }

    /// Tombstones a note whose file is already gone, keeping what its
    /// history last held in the trash, so a delete that arrived by sync is
    /// as recoverable as one made here.
    pub fn trash_vanished(&mut self, note: &NoteRow, by: &DeletedBy) -> Result<i64> {
        Ok(self
            .keep_vanished(note, by)?
            .seqs
            .first()
            .copied()
            .unwrap_or(0))
    }

    fn keep_vanished(&mut self, note: &NoteRow, by: &DeletedBy) -> Result<Trashed> {
        let now = now_rfc3339();
        let gone = |seq: i64| Trashed {
            trash_id: None,
            notes: 1,
            seqs: vec![seq],
        };
        let Some(content) = self.db.version_content(&note.id, note.version)? else {
            return Ok(gone(self.db.delete_note(&note.id, &now)?));
        };
        let trash_id = Uuid::new_v4().to_string();
        if let Err(e) = self
            .vault
            .write_to_trash(&trash_id, &note.path, content.as_bytes())
        {
            tracing::warn!(path = %note.path, error = format!("{e:#}"), "could not keep a vanished note in the trash");
            return Ok(gone(self.db.delete_note(&note.id, &now)?));
        }
        let row = TrashRow {
            id: trash_id.clone(),
            kind: KIND_NOTE.into(),
            original_path: note.path.clone(),
            deleted_at: now,
            by: by.clone(),
            folders: Vec::new(),
        };
        let items = [TrashItem {
            path: note.path.clone(),
            note_id: Some(note.id.clone()),
            title: Some(note.title.clone()),
        }];
        Ok(Trashed {
            trash_id: Some(trash_id),
            notes: 1,
            seqs: self.db.trash(&row, &items)?,
        })
    }

    /// Attachments the trashed notes link to and no other note does (150a).
    /// Ones inside `within` move with that folder and are not listed.
    fn lone_attachments(
        &self,
        trashed: &[(String, String)],
        within: Option<&str>,
    ) -> Result<Vec<String>> {
        let prefix = within.map(|w| format!("{w}/"));
        let candidates: Vec<String> = self
            .db
            .list_attachments()?
            .into_iter()
            .map(|a| a.path)
            .filter(|p| prefix.as_ref().is_none_or(|pre| !p.starts_with(pre)))
            .filter(|p| trashed.iter().any(|(_, c)| references(c, p)))
            .collect();
        if candidates.is_empty() {
            return Ok(candidates);
        }
        let ids: HashSet<&str> = trashed.iter().map(|(id, _)| id.as_str()).collect();
        let mut shared = HashSet::new();
        for note in self.db.list_notes()? {
            if ids.contains(note.id.as_str()) {
                continue;
            }
            // A note that cannot be read might link to any of them.
            let Ok(content) = self.vault.read(&note.path) else {
                return Ok(Vec::new());
            };
            for p in &candidates {
                if references(&content, p) {
                    shared.insert(p.clone());
                }
            }
        }
        Ok(candidates
            .into_iter()
            .filter(|p| !shared.contains(p))
            .collect())
    }

    fn move_attachments(
        &mut self,
        paths: &[String],
        trash_id: &str,
        keep: &HashSet<String>,
    ) -> Result<Vec<TrashItem>> {
        let mut items = Vec::new();
        for path in paths {
            match self.vault.move_to_trash(path, trash_id, keep) {
                Ok(()) => {
                    self.db.remove_attachment(path)?;
                    items.push(TrashItem {
                        path: path.clone(),
                        note_id: None,
                        title: None,
                    });
                }
                Err(e) => {
                    tracing::warn!(%path, error = format!("{e:#}"), "attachment left in place");
                }
            }
        }
        Ok(items)
    }

    pub fn list_trash(&self) -> Result<Vec<TrashEntry>> {
        let mut out = Vec::new();
        for row in self.db.list_trash()? {
            let items = self.db.trash_items(&row.id)?;
            let notes: Vec<TrashedNote> = items
                .iter()
                .filter_map(|i| {
                    Some(TrashedNote {
                        id: i.note_id.clone()?,
                        path: i.path.clone(),
                        title: i.title.clone().unwrap_or_else(|| display_name(&i.path)),
                    })
                })
                .collect();
            let attachments = items
                .iter()
                .filter(|i| i.note_id.is_none())
                .map(|i| i.path.clone())
                .collect();
            out.push(TrashEntry {
                name: display_name(&row.original_path),
                note_count: notes.len(),
                notes,
                attachments,
                id: row.id,
                kind: row.kind,
                original_path: row.original_path,
                deleted_at: row.deleted_at,
                deleted_by: row.by,
            });
        }
        Ok(out)
    }

    /// Puts an entry back at its original paths with its original ids. A
    /// path that is taken gets "(restored)" and nothing is overwritten.
    pub fn restore_trash(&mut self, trash_id: &str) -> IndexResult<Restored> {
        let entry = self
            .db
            .get_trash(trash_id)?
            .ok_or_else(|| not_in_trash(trash_id))?;
        let ids: HashMap<String, Option<String>> = self
            .db
            .trash_items(trash_id)?
            .into_iter()
            .map(|i| (i.path, i.note_id))
            .collect();
        let (files, dirs) = self.vault.trashed(trash_id)?;
        let now = now_rfc3339();

        for dir in &dirs {
            if !self.vault.occupied(dir) {
                self.vault.create_dir(dir)?;
            }
        }
        let mut restored = Restored::default();
        for rel in files {
            let dest = self.free_path(&rel)?;
            self.vault.restore_from_trash(trash_id, &rel, &dest)?;
            if is_markdown(&rel) {
                let known = ids.get(&rel).cloned().flatten();
                let (id, seq) = self.index_restored(&dest, known)?;
                restored.seqs.push(seq);
                restored.notes.push(RestoredNote { id, path: dest });
            } else {
                let bytes = self.vault.read_bytes(&dest)?;
                self.db.record_attachment(
                    &dest,
                    blake3::hash(&bytes).to_hex().as_ref(),
                    bytes.len() as i64,
                    &now,
                )?;
                restored.attachments.push(dest);
            }
        }
        for folder in &entry.folders {
            if self.vault.resolve(folder).is_ok_and(|p| p.is_dir()) {
                self.db.record_folder(folder, &now)?;
            }
        }
        self.vault.purge_trash(trash_id)?;
        self.db.forget_trash(trash_id, false)?;
        Ok(restored)
    }

    /// The original path if it is free, else the first free "(restored)" one.
    fn free_path(&self, rel: &str) -> Result<String> {
        let free = |p: &str| -> Result<bool> {
            Ok(!self.vault.occupied(p) && self.db.get_note_by_path(p)?.is_none())
        };
        if free(rel)? {
            return Ok(rel.to_string());
        }
        let mut n = 1;
        loop {
            let candidate = restored_name(rel, n);
            if free(&candidate)? {
                return Ok(candidate);
            }
            n += 1;
        }
    }

    /// Indexes a restored note under the id it had. Should that id be live
    /// again meanwhile, this copy gets a fresh one, as any copy does.
    fn index_restored(&mut self, path: &str, known: Option<String>) -> Result<(String, i64)> {
        let mut content = self.vault.read(path)?;
        let claimed = known.or_else(|| id_of(&content));
        let id = match claimed {
            Some(id) if self.db.get_note(&id)?.is_none() => id,
            _ => Uuid::new_v4().to_string(),
        };
        if id_of(&content).as_deref() != Some(id.as_str()) {
            content = frontmatter::set_scalars(&content, &[("id", &id)]);
            self.vault.write(path, &content)?;
        }
        let version = self.db.next_version(&id)?;
        let row = self.row_for(&id, path, &content, version, None);
        let extracted = parse::extract(&content);
        let seq = self
            .db
            .record_note(&row, &content, &extracted, db::KIND_CREATED, None)?;
        Ok((id, seq))
    }

    /// Delete forever: the files, the entry and its notes' history.
    pub fn purge_trash(&mut self, trash_id: &str) -> IndexResult<()> {
        self.db
            .get_trash(trash_id)?
            .ok_or_else(|| not_in_trash(trash_id))?;
        self.vault.purge_trash(trash_id)?;
        self.db.forget_trash(trash_id, true)?;
        Ok(())
    }

    /// Empty trash. Only Storm's own entries: anything else under `.trash/`
    /// (Obsidian keeps its own there) is left alone.
    pub fn empty_trash(&mut self) -> IndexResult<usize> {
        let entries = self.db.list_trash()?;
        for entry in &entries {
            self.purge_trash(&entry.id)?;
        }
        Ok(entries.len())
    }

    /// The vault's retention period in days, `None` for never.
    pub fn trash_retention_days(&self) -> Option<i64> {
        let value = self.vault.read(VAULT_CONFIG_PATH).ok().and_then(|raw| {
            frontmatter::parse(&raw)
                .frontmatter
                .and_then(|fm| frontmatter::get_scalar(&fm.raw, RETENTION_KEY))
        });
        parse_retention(value.as_deref())
    }

    /// Purges entries deleted more than the retention period before `now`.
    pub fn purge_expired_trash(&mut self, now: time::OffsetDateTime) -> Result<usize> {
        let Some(days) = self.trash_retention_days() else {
            return Ok(0);
        };
        let cutoff = now - time::Duration::days(days);
        let mut purged = 0;
        for entry in self.db.list_trash()? {
            let Ok(at) = time::OffsetDateTime::parse(
                &entry.deleted_at,
                &time::format_description::well_known::Rfc3339,
            ) else {
                continue;
            };
            if at < cutoff {
                self.purge_trash(&entry.id)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                purged += 1;
            }
        }
        Ok(purged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::vault::Vault;

    fn indexer() -> (tempdir::TempDir, Indexer) {
        let dir = tempdir::TempDir::new("storm-trash").unwrap();
        let vault = Vault::new(dir.path()).unwrap();
        (dir, Indexer::new(vault, Db::open_in_memory().unwrap()))
    }

    fn me() -> DeletedBy {
        DeletedBy::person(Some("dev_1".into()), Some("Laptop".into()))
    }

    fn last_kind(ix: &Indexer) -> String {
        let seq = ix.db.latest_seq().unwrap();
        ix.db.changes_since(seq - 1, 1).unwrap()[0].kind.clone()
    }

    fn edited(ix: &mut Indexer, path: &str) -> (String, String) {
        let id = ix.create_note(path, "first\n").unwrap().note.id;
        let saved = ix
            .put_note(&id, 1, "second [[Other]] #tag\n", None)
            .unwrap();
        (id, saved.content)
    }

    fn assert_same_note_back(ix: &Indexer, id: &str, path: &str, content: &str) {
        let row = ix.db.get_note(id).unwrap().expect("same id back");
        assert_eq!(row.path, path);
        assert_eq!(ix.vault.read(path).unwrap(), content);
        let history: Vec<i64> = ix
            .db
            .list_versions(id)
            .unwrap()
            .iter()
            .map(|v| v.version)
            .collect();
        assert!(
            history.ends_with(&[2, 1]),
            "history kept and continued: {history:?}"
        );
        assert_eq!(ix.db.search("second", 10).unwrap().len(), 1);
    }

    /// A-150-1: a delete made here, by an agent, and one that arrived by
    /// sync are all in the trash, and restore brings back the same note.
    #[test]
    fn a_deleted_note_is_recoverable_whoever_deleted_it() {
        let (_d, mut ix) = indexer();
        let (id, content) = edited(&mut ix, "Notes/A.md");
        let trashed = ix.delete_note(&id, &me()).unwrap();
        assert_eq!(last_kind(&ix), db::KIND_DELETED);
        assert!(!ix.vault.exists("Notes/A.md"));
        assert!(ix.db.get_note(&id).unwrap().is_none());
        assert!(ix.db.search("second", 10).unwrap().is_empty());

        let listed = ix.list_trash().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "A");
        assert_eq!(listed[0].original_path, "Notes/A.md");
        assert_eq!(listed[0].deleted_by, me());
        assert_eq!(listed[0].note_count, 1);

        let tid = trashed.trash_id.unwrap();
        let restored = ix.restore_trash(&tid).unwrap();
        assert_eq!(restored.notes[0].id, id);
        assert_eq!(last_kind(&ix), db::KIND_CREATED);
        assert_same_note_back(&ix, &id, "Notes/A.md", &content);
        assert!(ix.list_trash().unwrap().is_empty());
        assert!(!ix.vault.root().join(".trash").join(&tid).exists());

        let agent = DeletedBy::agent("hst_1", "ses_1", Some("duplicate".into()));
        let tid = ix.delete_note(&id, &agent).unwrap().trash_id.unwrap();
        let by = ix.list_trash().unwrap()[0].deleted_by.clone();
        assert_eq!(
            (
                by.kind.as_str(),
                by.session_id.as_deref(),
                by.reason.as_deref()
            ),
            ("agent", Some("ses_1"), Some("duplicate"))
        );
        ix.restore_trash(&tid).unwrap();
        assert_same_note_back(&ix, &id, "Notes/A.md", &content);

        std::fs::remove_file(ix.vault.root().join("Notes/A.md")).unwrap();
        ix.refresh_paths(&["Notes/A.md".to_string()]).unwrap();
        assert!(ix.db.get_note(&id).unwrap().is_none());
        let listed = ix.list_trash().unwrap();
        assert_eq!(listed[0].deleted_by.kind, "sync");
        ix.restore_trash(&listed[0].id).unwrap();
        assert_same_note_back(&ix, &id, "Notes/A.md", &content);
    }

    #[test]
    fn a_note_gone_at_boot_is_kept_in_the_trash() {
        let (_d, mut ix) = indexer();
        let (id, content) = edited(&mut ix, "A.md");
        ix.create_note("B.md", "b\n").unwrap();
        std::fs::remove_file(ix.vault.root().join("A.md")).unwrap();
        assert_eq!(ix.reconcile(false).unwrap().removed, 1);
        let listed = ix.list_trash().unwrap();
        assert_eq!(listed[0].deleted_by.kind, "sync");
        ix.restore_trash(&listed[0].id).unwrap();
        assert_same_note_back(&ix, &id, "A.md", &content);
    }

    /// A-150-2: a folder holding notes is one entry, and restore brings back
    /// the folder, every note under its id, and a recorded empty subfolder.
    #[test]
    fn a_folder_is_one_entry_and_comes_back_whole() {
        let (_d, mut ix) = indexer();
        ix.create_folder("Projects").unwrap();
        ix.create_folder("Projects/Empty").unwrap();
        let a = ix.create_note("Projects/A.md", "a\n").unwrap().note.id;
        let b = ix.create_note("Projects/Sub/B.md", "b\n").unwrap().note.id;
        let kept = ix.create_note("Elsewhere.md", "k\n").unwrap().note.id;

        let trashed = ix.trash_folder("Projects", &me()).unwrap();
        assert_eq!(trashed.notes, 2);
        assert_eq!(trashed.seqs.len(), 2);
        assert!(!ix.vault.root().join("Projects").exists());
        assert!(
            !ix.all_folders()
                .unwrap()
                .iter()
                .any(|f| f.starts_with("Projects"))
        );
        assert!(ix.db.get_note(&kept).unwrap().is_some());

        let listed = ix.list_trash().unwrap();
        assert_eq!(listed.len(), 1, "one entry for the folder");
        assert_eq!(
            (
                listed[0].kind.as_str(),
                listed[0].name.as_str(),
                listed[0].note_count
            ),
            ("folder", "Projects", 2)
        );

        let restored = ix.restore_trash(&listed[0].id).unwrap();
        assert_eq!(restored.notes.len(), 2);
        assert_eq!(ix.db.get_note(&a).unwrap().unwrap().path, "Projects/A.md");
        assert_eq!(
            ix.db.get_note(&b).unwrap().unwrap().path,
            "Projects/Sub/B.md"
        );
        let folders = ix.all_folders().unwrap();
        assert!(folders.contains(&"Projects".to_string()));
        assert!(folders.contains(&"Projects/Empty".to_string()));
        assert!(ix.vault.root().join("Projects/Empty").is_dir());
    }

    #[test]
    fn an_empty_folder_is_removed_and_the_settings_folder_is_refused() {
        let (_d, mut ix) = indexer();
        ix.create_folder("Empty").unwrap();
        let trashed = ix.trash_folder("Empty", &me()).unwrap();
        assert!(trashed.trash_id.is_none());
        assert!(ix.list_trash().unwrap().is_empty());
        assert!(ix.all_folders().unwrap().is_empty());

        ix.create_note("_storm/vault.md", "---\n---\n").unwrap();
        assert!(matches!(
            ix.trash_folder("_storm", &me()),
            Err(IndexError::Invalid(_))
        ));
        assert!(ix.vault.exists("_storm/vault.md"));
    }

    /// A-150-3: a restore onto a taken path never overwrites.
    #[test]
    fn a_restore_onto_a_taken_path_is_renamed() {
        let (_d, mut ix) = indexer();
        let id = ix.create_note("Ideas.md", "old\n").unwrap().note.id;
        let first = ix.delete_note(&id, &me()).unwrap().trash_id.unwrap();
        let newer = ix.create_note("Ideas.md", "new\n").unwrap();

        let restored = ix.restore_trash(&first).unwrap();
        assert_eq!(restored.notes[0].path, "Ideas (restored).md");
        assert_eq!(ix.vault.read("Ideas.md").unwrap(), newer.content);
        assert_eq!(
            ix.db.get_note(&id).unwrap().unwrap().path,
            "Ideas (restored).md"
        );

        let other = ix.create_note("Other.md", "o\n").unwrap().note.id;
        let again = ix.delete_note(&other, &me()).unwrap().trash_id.unwrap();
        ix.vault.write("Other.md", "squatter\n").unwrap();
        ix.vault.write("Other (restored).md", "squatter\n").unwrap();
        let restored = ix.restore_trash(&again).unwrap();
        assert_eq!(restored.notes[0].path, "Other (restored 2).md");
        assert_eq!(ix.vault.read("Other (restored).md").unwrap(), "squatter\n");
    }

    #[test]
    fn a_folder_restored_where_one_exists_again_merges_without_overwriting() {
        let (_d, mut ix) = indexer();
        ix.create_note("P/A.md", "a\n").unwrap();
        ix.create_note("P/B.md", "b\n").unwrap();
        let tid = ix.trash_folder("P", &me()).unwrap().trash_id.unwrap();
        let fresh = ix.create_note("P/A.md", "fresh\n").unwrap();
        ix.restore_trash(&tid).unwrap();
        assert_eq!(ix.vault.read("P/A.md").unwrap(), fresh.content);
        assert!(ix.vault.exists("P/A (restored).md"));
        assert!(ix.vault.exists("P/B.md"));
    }

    fn backdate(ix: &Indexer, trash_id: &str, days: i64) {
        let at = (time::OffsetDateTime::now_utc() - time::Duration::days(days))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        ix.db.set_trash_deleted_at_for_test(trash_id, &at).unwrap();
    }

    /// A-150-4: items past retention go, newer ones stay, and the purge
    /// takes the files and the history.
    #[test]
    fn retention_purges_only_what_is_past_it() {
        let (_d, mut ix) = indexer();
        let old = ix.create_note("Old.md", "o\n").unwrap().note.id;
        let new = ix.create_note("New.md", "n\n").unwrap().note.id;
        let old_t = ix.delete_note(&old, &me()).unwrap().trash_id.unwrap();
        let new_t = ix.delete_note(&new, &me()).unwrap().trash_id.unwrap();
        backdate(&ix, &old_t, 31);
        backdate(&ix, &new_t, 29);

        assert_eq!(ix.trash_retention_days(), Some(30));
        let now = time::OffsetDateTime::now_utc();
        assert_eq!(ix.purge_expired_trash(now).unwrap(), 1);
        let left: Vec<String> = ix.list_trash().unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(left, vec![new_t.clone()]);
        assert!(!ix.vault.root().join(".trash").join(&old_t).exists());
        assert!(
            ix.db.list_versions(&old).unwrap().is_empty(),
            "history goes too"
        );
        assert!(!ix.db.list_versions(&new).unwrap().is_empty());

        ix.create_note("_storm/vault.md", "---\nstorm.trash.retention: 7\n---\n")
            .unwrap();
        assert_eq!(ix.trash_retention_days(), Some(7));
        assert_eq!(ix.purge_expired_trash(now).unwrap(), 1);
        assert!(ix.list_trash().unwrap().is_empty());
    }

    #[test]
    fn retention_never_keeps_everything() {
        let (_d, mut ix) = indexer();
        ix.create_note(
            "_storm/vault.md",
            "---\nstorm.trash.retention: never\n---\n",
        )
        .unwrap();
        let id = ix.create_note("A.md", "a\n").unwrap().note.id;
        let tid = ix.delete_note(&id, &me()).unwrap().trash_id.unwrap();
        backdate(&ix, &tid, 400);
        assert_eq!(ix.trash_retention_days(), None);
        assert_eq!(
            ix.purge_expired_trash(time::OffsetDateTime::now_utc())
                .unwrap(),
            0
        );
    }

    #[test]
    fn the_retention_setting_reads_days_or_never() {
        assert_eq!(parse_retention(None), Some(30));
        assert_eq!(parse_retention(Some("90")), Some(90));
        assert_eq!(parse_retention(Some("\"7d\"")), Some(7));
        assert_eq!(parse_retention(Some("never")), None);
        assert_eq!(parse_retention(Some("soon")), Some(30));
        assert_eq!(parse_retention(Some("0")), Some(30));
    }

    #[test]
    fn delete_forever_and_empty_trash_leave_nothing_and_spare_other_files() {
        let (_d, mut ix) = indexer();
        let a = ix.create_note("A.md", "a\n").unwrap().note.id;
        let b = ix.create_note("B.md", "b\n").unwrap().note.id;
        let ta = ix.delete_note(&a, &me()).unwrap().trash_id.unwrap();
        ix.delete_note(&b, &me()).unwrap();
        std::fs::write(ix.vault.root().join(".trash/obsidian.md"), "theirs").unwrap();

        ix.purge_trash(&ta).unwrap();
        assert!(matches!(
            ix.restore_trash(&ta),
            Err(IndexError::NotFound(_))
        ));
        assert!(ix.db.list_versions(&a).unwrap().is_empty());
        assert_eq!(ix.empty_trash().unwrap(), 1);
        assert!(ix.list_trash().unwrap().is_empty());
        assert!(ix.vault.root().join(".trash/obsidian.md").exists());
    }

    /// A-150-5: a trashed note is in no search, list, tag or backlinks.
    #[test]
    fn trashed_notes_are_invisible() {
        let (_d, mut ix) = indexer();
        let id = ix
            .create_note("A.md", "findme [[Target]] #gone\n")
            .unwrap()
            .note
            .id;
        ix.create_note("Target.md", "t\n").unwrap();
        ix.delete_note(&id, &me()).unwrap();
        assert!(ix.db.search("findme", 10).unwrap().is_empty());
        assert!(ix.db.list_notes().unwrap().iter().all(|n| n.id != id));
        assert!(ix.db.backlinks("Target").unwrap().is_empty());
        assert!(ix.db.notes_with_tag("gone").unwrap().is_empty());
        assert_eq!(
            ix.reconcile(false).unwrap().indexed,
            0,
            "the scan skips .trash/"
        );
        assert!(ix.db.get_note(&id).unwrap().is_none());
    }

    /// A-150-6: an attachment only the trashed note links to goes and comes
    /// back with it; one another note links to stays.
    #[test]
    fn lone_attachments_travel_with_the_note_and_shared_ones_stay() {
        let (_d, mut ix) = indexer();
        ix.put_attachment("attachments/lone.png", &[1, 2]).unwrap();
        ix.put_attachment("attachments/shared.png", &[3]).unwrap();
        ix.put_attachment("attachments/unused.png", &[4]).unwrap();
        let id = ix
            .create_note("A.md", "![](attachments/lone.png) ![[shared.png]]\n")
            .unwrap()
            .note
            .id;
        ix.create_note("B.md", "![](attachments/shared.png)\n")
            .unwrap();

        let tid = ix.delete_note(&id, &me()).unwrap().trash_id.unwrap();
        let listed: Vec<String> = ix
            .db
            .list_attachments()
            .unwrap()
            .into_iter()
            .map(|a| a.path)
            .collect();
        assert_eq!(
            listed,
            vec!["attachments/shared.png", "attachments/unused.png"]
        );
        assert!(!ix.vault.root().join("attachments/lone.png").exists());
        assert_eq!(
            ix.list_trash().unwrap()[0].attachments,
            vec!["attachments/lone.png".to_string()]
        );

        let restored = ix.restore_trash(&tid).unwrap();
        assert_eq!(
            restored.attachments,
            vec!["attachments/lone.png".to_string()]
        );
        assert_eq!(ix.attachment("attachments/lone.png").unwrap(), vec![1, 2]);
        assert_eq!(ix.db.list_attachments().unwrap().len(), 3);
    }

    #[test]
    fn a_folder_takes_its_own_attachments_and_lone_outside_ones() {
        let (_d, mut ix) = indexer();
        ix.put_attachment("P/inside.pdf", &[1]).unwrap();
        ix.put_attachment("attachments/out.png", &[2]).unwrap();
        ix.create_note("P/A.md", "![](attachments/out.png)\n")
            .unwrap();
        let tid = ix.trash_folder("P", &me()).unwrap().trash_id.unwrap();
        assert!(ix.db.list_attachments().unwrap().is_empty());
        ix.restore_trash(&tid).unwrap();
        assert_eq!(ix.db.list_attachments().unwrap().len(), 2);
        assert!(ix.vault.root().join("P/inside.pdf").exists());
    }

    #[test]
    fn restored_names_keep_the_extension() {
        assert_eq!(restored_name("A/B.md", 1), "A/B (restored).md");
        assert_eq!(restored_name("B.md", 3), "B (restored 3).md");
        assert_eq!(restored_name("img/a.png", 1), "img/a (restored).png");
        assert_eq!(restored_name("README", 1), "README (restored)");
    }
}
