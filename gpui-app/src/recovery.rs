//! Where the app keeps its own data, and the crash-recovery journal: the
//! text of unsaved documents, written a moment after each edit so it can be
//! offered back after a crash or a killed process.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The app's data directory (settings, recovery drafts, logs).
///
/// `ZIJIAN_DATA_DIR` overrides it; otherwise the platform's per-user data
/// directory is used (`%LOCALAPPDATA%\Zijian` on Windows).
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ZIJIAN_DATA_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Library").join("Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    base.unwrap_or_else(std::env::temp_dir).join("Zijian")
}

/// One unsaved document as written to the journal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Draft {
    pub path: String,
    pub text: String,
    /// Seconds since the Unix epoch when the draft was written.
    pub written: u64,
}

/// The recovery journal: one JSON file per document.
#[derive(Clone)]
pub struct Journal {
    dir: PathBuf,
    revisions: Arc<Mutex<HashMap<String, u64>>>,
}

/// A reserved write can be invalidated before its background task runs.
pub struct DraftWrite {
    journal: Journal,
    path: String,
    text: String,
    revision: u64,
}

impl DraftWrite {
    pub fn commit(self) -> std::io::Result<()> {
        // Hold the same lock as remove through the atomic replacement. Thus
        // deletion either invalidates this write, or runs after it finishes.
        let revisions = self
            .journal
            .revisions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if revisions.get(&self.path) != Some(&self.revision) {
            return Ok(());
        }
        self.journal.write_now(&self.path, &self.text)
    }
}

impl Journal {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            revisions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn default_location() -> Self {
        Self::new(data_dir().join("recovery"))
    }

    fn file_for(&self, path: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        self.dir.join(format!("{:016x}.json", hasher.finish()))
    }

    /// Save the unsaved `text` of `path`. Written atomically so a crash in
    /// the middle never corrupts the previous draft.
    #[cfg(test)]
    pub fn write(&self, path: &str, text: &str) -> std::io::Result<()> {
        self.prepare_write(path, text).commit()
    }

    pub fn prepare_write(&self, path: &str, text: &str) -> DraftWrite {
        let mut revisions = self.revisions.lock().unwrap_or_else(|e| e.into_inner());
        let revision = revisions.entry(path.to_string()).or_default();
        *revision += 1;
        DraftWrite {
            journal: self.clone(),
            path: path.into(),
            text: text.into(),
            revision: *revision,
        }
    }

    fn write_now(&self, path: &str, text: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let draft = Draft {
            path: path.to_string(),
            text: text.to_string(),
            written: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let json = serde_json::to_vec(&draft).map_err(std::io::Error::other)?;
        crate::files::atomic_write(&self.file_for(path), &json).map(|_| ())
    }

    /// Forget the draft of `path` (saved, discarded or closed cleanly).
    pub fn remove(&self, path: &str) {
        let mut revisions = self.revisions.lock().unwrap_or_else(|e| e.into_inner());
        *revisions.entry(path.to_string()).or_default() += 1;
        let _ = std::fs::remove_file(self.file_for(path));
    }

    /// All readable drafts, newest first. Unreadable files are skipped.
    pub fn drafts(&self) -> Vec<Draft> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return vec![];
        };
        let mut drafts: Vec<Draft> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|bytes| serde_json::from_slice::<Draft>(&bytes).ok())
            .collect();
        drafts.sort_by(|a, b| b.written.cmp(&a.written));
        drafts
    }

    /// Whether a draft is worth offering: its text differs from the file.
    pub fn differs_from_disk(draft: &Draft) -> bool {
        std::fs::read_to_string(Path::new(&draft.path))
            .ok()
            .as_deref()
            != Some(draft.text.as_str())
    }

    #[cfg(test)]
    pub fn clear(&self) {
        for draft in self.drafts() {
            self.remove(&draft.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal(tag: &str) -> Journal {
        let dir = std::env::temp_dir().join(format!(
            "zj-journal-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Journal::new(dir)
    }

    #[test]
    fn drafts_survive_until_removed() {
        let j = journal("roundtrip");
        j.write(r"E:\p\main.typ", "未保存的草稿").unwrap();
        j.write(r"E:\p\chapter.typ", "另一份").unwrap();
        // Rewriting replaces the draft for the same document.
        j.write(r"E:\p\main.typ", "更新后的草稿").unwrap();
        let drafts = j.drafts();
        assert_eq!(drafts.len(), 2);
        let main = drafts
            .iter()
            .find(|d| d.path.ends_with("main.typ"))
            .unwrap();
        assert_eq!(main.text, "更新后的草稿");

        j.remove(r"E:\p\main.typ");
        assert_eq!(j.drafts().len(), 1);
        j.clear();
        assert!(j.drafts().is_empty());
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn corrupt_entries_are_ignored() {
        let j = journal("corrupt");
        j.write("a.typ", "ok").unwrap();
        std::fs::write(j.dir.join("broken.json"), b"{not json").unwrap();
        assert_eq!(j.drafts().len(), 1);
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn pending_write_cannot_recreate_a_saved_or_discarded_draft() {
        let j = journal("cancel-pending");
        j.write("main.typ", "previous draft").unwrap();
        let pending = j.clone().prepare_write("main.typ", "queued stale draft");
        j.remove("main.typ");
        pending.commit().unwrap();
        assert!(j.drafts().is_empty());
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn newest_reserved_draft_wins_when_tasks_finish_in_reverse_order() {
        let j = journal("write-order");
        let old = j.prepare_write("main.typ", "old text");
        let new = j.clone().prepare_write("main.typ", "new text");
        new.commit().unwrap();
        old.commit().unwrap();
        assert_eq!(j.drafts()[0].text, "new text");
        let _ = std::fs::remove_dir_all(&j.dir);
    }

    #[test]
    fn only_drafts_that_differ_from_disk_are_offered() {
        let j = journal("differs");
        std::fs::create_dir_all(&j.dir).unwrap();
        let file = j.dir.join("doc.typ");
        std::fs::write(&file, "same").unwrap();
        let same = Draft {
            path: file.display().to_string(),
            text: "same".into(),
            written: 0,
        };
        assert!(!Journal::differs_from_disk(&same));
        let changed = Draft {
            text: "changed".into(),
            ..same
        };
        assert!(Journal::differs_from_disk(&changed));
        let _ = std::fs::remove_dir_all(&j.dir);
    }
}
