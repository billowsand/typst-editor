//! Editing session state, kept free of GPUI so it can be unit-tested:
//! the project root, open buffers in tab order, the active document and a
//! generation counter that invalidates work started for an older session.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::files::DiskVersion;

/// One open document: what is on disk and what is in the editor.
#[derive(Debug, Clone)]
pub struct Buffer {
    pub saved: String,
    pub current: String,
    /// The file's version when it was last read or written by us; a
    /// different version on disk means someone else changed the file.
    pub disk: Option<DiskVersion>,
    /// The user kept the editor's text over an outside edit: unsaved even if
    /// the text equals what we last saved.
    pub diverged: bool,
}

impl Buffer {
    pub fn is_dirty(&self) -> bool {
        self.diverged || self.current != self.saved
    }
}

/// Canonical spelling of a path used as a buffer key, so `E:\a/b.typ` from
/// the compiler and `E:\a\b.typ` from the file tree name the same buffer.
pub fn normalize(path: &str) -> String {
    Path::new(path)
        .components()
        .collect::<PathBuf>()
        .display()
        .to_string()
}

#[derive(Default)]
pub struct Workspace {
    root: Option<PathBuf>,
    buffers: HashMap<String, Buffer>,
    /// Open documents in tab order.
    order: Vec<String>,
    active: Option<String>,
    generation: u64,
}

impl Workspace {
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Bumped on every session reset; async results tagged with an older
    /// value belong to a project that is no longer open.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn tabs(&self) -> impl Iterator<Item = (&str, &Buffer)> {
        self.order
            .iter()
            .filter_map(|p| self.buffers.get(p).map(|b| (p.as_str(), b)))
    }

    pub fn tab_index(&self, path: &str) -> Option<usize> {
        let path = normalize(path);
        self.order.iter().position(|p| *p == path)
    }

    pub fn buffer(&self, path: &str) -> Option<&Buffer> {
        self.buffers.get(&normalize(path))
    }

    pub fn is_active(&self, path: &str) -> bool {
        self.active.as_deref() == Some(normalize(path).as_str())
    }

    pub fn active_buffer(&self) -> Option<&Buffer> {
        self.active.as_ref().and_then(|p| self.buffers.get(p))
    }

    pub fn dirty_paths(&self) -> Vec<String> {
        self.order
            .iter()
            .filter(|p| self.buffers.get(*p).is_some_and(Buffer::is_dirty))
            .cloned()
            .collect()
    }

    /// Whether `root` is the project already open.
    pub fn is_root(&self, root: &Path) -> bool {
        self.root.as_deref().is_some_and(|r| {
            normalize(&r.display().to_string()) == normalize(&root.display().to_string())
        })
    }

    /// Whether `path` lies inside the open project.
    pub fn contains(&self, path: &Path) -> bool {
        self.root.as_deref().is_some_and(|r| {
            Path::new(&normalize(&path.display().to_string()))
                .starts_with(normalize(&r.display().to_string()))
        })
    }

    /// Start a fresh session for `root`, dropping every buffer. Callers must
    /// resolve unsaved buffers first.
    pub fn reset(&mut self, root: Option<PathBuf>) {
        self.root = root;
        self.buffers.clear();
        self.order.clear();
        self.active = None;
        self.generation += 1;
    }

    /// Make `path` the active document, loading it if it has no buffer yet.
    /// Returns the normalized key.
    pub fn open(
        &mut self,
        path: &str,
        load: impl FnOnce(&str) -> std::io::Result<(String, Option<DiskVersion>)>,
    ) -> Result<String, String> {
        let key = normalize(path);
        if !self.buffers.contains_key(&key) {
            let (text, disk) = load(&key).map_err(|e| format!("无法读取 {key}：{e}"))?;
            self.buffers.insert(
                key.clone(),
                Buffer {
                    saved: text.clone(),
                    current: text,
                    disk,
                    diverged: false,
                },
            );
            self.order.push(key.clone());
        }
        self.active = Some(key.clone());
        Ok(key)
    }

    /// Recover text even when the original was moved or deleted. Keep it
    /// dirty until the user saves it or chooses a new destination.
    pub fn restore_draft(&mut self, path: &str, text: String) -> Result<String, String> {
        let key = self.open(path, |p| {
            // A missing or unreadable original must not prevent recovery.
            Ok(crate::files::read_text(Path::new(p)).unwrap_or_default())
        })?;
        let buffer = self.buffers.get_mut(&key).expect("opened buffer");
        buffer.current = text;
        buffer.diverged = true;
        Ok(key)
    }

    /// Record editor text for `path`. Returns whether it changed.
    pub fn set_current(&mut self, path: &str, text: String) -> bool {
        match self.buffers.get_mut(&normalize(path)) {
            Some(buffer) if buffer.current != text => {
                buffer.current = text;
                true
            }
            _ => false,
        }
    }

    /// `written` reached disk at version `disk`.
    pub fn mark_saved(&mut self, path: &str, written: &str, disk: Option<DiskVersion>) {
        if let Some(buffer) = self.buffers.get_mut(&normalize(path)) {
            buffer.saved = written.to_string();
            buffer.disk = disk;
            buffer.diverged = false;
        }
    }

    /// Replace a buffer with what is now on disk (outside edit accepted).
    pub fn reload(&mut self, path: &str, text: String, disk: Option<DiskVersion>) {
        if let Some(buffer) = self.buffers.get_mut(&normalize(path)) {
            buffer.saved = text.clone();
            buffer.current = text;
            buffer.disk = disk;
            buffer.diverged = false;
        }
    }

    /// Keep the editor text but accept the disk version as the new base, so
    /// the next save overwrites the outside edit deliberately.
    pub fn keep_mine(&mut self, path: &str, disk: Option<DiskVersion>) {
        if let Some(buffer) = self.buffers.get_mut(&normalize(path)) {
            buffer.disk = disk;
            buffer.diverged = true;
        }
    }

    /// Open documents whose file changed on disk since we read or wrote it,
    /// with the version now on disk (`None`: deleted or moved).
    pub fn changed_on_disk(&self) -> Vec<(String, Option<DiskVersion>)> {
        self.order
            .iter()
            .filter_map(|p| {
                let buffer = self.buffers.get(p)?;
                let now = DiskVersion::of(Path::new(p));
                (now != buffer.disk).then(|| (p.clone(), now))
            })
            .collect()
    }

    /// A file was renamed or moved: carry its buffer (and tab) along.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        let (from, to) = (normalize(from), normalize(to));
        let Some(buffer) = self.buffers.remove(&from) else {
            return false;
        };
        self.buffers.insert(to.clone(), buffer);
        for p in self.order.iter_mut().filter(|p| **p == from) {
            *p = to.clone();
        }
        if self.active.as_deref() == Some(from.as_str()) {
            self.active = Some(to);
        }
        true
    }

    /// Open documents at or below `path` (a renamed or deleted folder).
    pub fn paths_under(&self, path: &str) -> Vec<String> {
        let base = normalize(path);
        self.order
            .iter()
            .filter(|p| Path::new(p.as_str()).starts_with(&base))
            .cloned()
            .collect()
    }

    /// Close `path`. Returns the document that should become active when the
    /// closed one was active (its right neighbour, else its left one).
    pub fn close(&mut self, path: &str) -> Option<String> {
        let key = normalize(path);
        let ix = self.order.iter().position(|p| *p == key)?;
        self.order.remove(ix);
        self.buffers.remove(&key);
        if self.active.as_deref() != Some(key.as_str()) {
            return None;
        }
        self.active = None;
        self.order
            .get(ix)
            .or_else(|| ix.checked_sub(1).and_then(|i| self.order.get(i)))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_original_draft_opens_dirty_without_creating_a_disk_file() {
        let path = std::env::temp_dir().join(format!("zj-missing-{}.typ", std::process::id()));
        assert!(!path.exists());
        let mut ws = Workspace::default();
        let key = ws
            .restore_draft(&path.display().to_string(), "恢复的内容".into())
            .unwrap();
        let buffer = ws.buffer(&key).unwrap();
        assert_eq!(buffer.current, "恢复的内容");
        assert!(buffer.is_dirty());
        assert!(buffer.disk.is_none());
        assert!(!path.exists());
    }

    fn text(s: &str) -> impl FnOnce(&str) -> std::io::Result<(String, Option<DiskVersion>)> + '_ {
        move |_| Ok((s.to_string(), None))
    }

    #[test]
    fn reopening_after_reset_rebuilds_the_buffer() {
        // Regression: resetting the session and reopening the same path used
        // to leave an active path without a buffer, so edits were dropped.
        let mut ws = Workspace::default();
        ws.reset(Some(PathBuf::from("E:/p")));
        ws.open("E:/p/main.typ", text("a")).unwrap();
        ws.reset(Some(PathBuf::from("E:/p")));
        assert_eq!(ws.active(), None);
        ws.open("E:/p/main.typ", text("a")).unwrap();
        assert!(ws.active_buffer().is_some());
        assert!(ws.set_current("E:/p/main.typ", "ab".into()));
        assert_eq!(ws.dirty_paths().len(), 1);
    }

    #[test]
    fn reset_bumps_generation_and_clears_tabs() {
        let mut ws = Workspace::default();
        ws.open("a.typ", text("1")).unwrap();
        let generation = ws.generation();
        ws.reset(None);
        assert!(ws.generation() > generation);
        assert_eq!(ws.tabs().count(), 0);
    }

    #[test]
    fn paths_are_normalized() {
        let mut ws = Workspace::default();
        ws.open(r"E:\p\chapters/intro.typ", text("x")).unwrap();
        assert!(ws.is_active(r"E:\p\chapters\intro.typ"));
        assert!(ws.buffer("E:/p/chapters/intro.typ").is_some() || cfg!(not(windows)));
        assert_eq!(ws.tabs().count(), 1);
    }

    #[test]
    fn edits_survive_switching_documents() {
        let mut ws = Workspace::default();
        ws.open("a.typ", text("a")).unwrap();
        ws.set_current("a.typ", "a!".into());
        ws.open("b.typ", text("b")).unwrap();
        ws.open("a.typ", |_| panic!("must not reload")).unwrap();
        assert_eq!(ws.active_buffer().unwrap().current, "a!");
        assert_eq!(ws.dirty_paths(), vec![normalize("a.typ")]);
    }

    #[test]
    fn closing_the_active_tab_picks_a_neighbour() {
        let mut ws = Workspace::default();
        for p in ["a.typ", "b.typ", "c.typ"] {
            ws.open(p, text(p)).unwrap();
        }
        ws.open("b.typ", text("")).unwrap();
        assert_eq!(ws.close("b.typ"), Some(normalize("c.typ")));
        ws.open("c.typ", text("")).unwrap();
        assert_eq!(ws.close("c.typ"), Some(normalize("a.typ")));
        assert_eq!(ws.close("missing.typ"), None);
        // Closing an inactive tab leaves the active document alone.
        ws.open("a.typ", text("")).unwrap();
        ws.open("d.typ", text("d")).unwrap();
        assert_eq!(ws.close("a.typ"), None);
        assert!(ws.is_active("d.typ"));
    }

    #[test]
    fn saving_clears_dirty_state() {
        let mut ws = Workspace::default();
        ws.open("a.typ", text("a")).unwrap();
        ws.set_current("a.typ", "b".into());
        ws.mark_saved("a.typ", "b", None);
        assert!(ws.dirty_paths().is_empty());
        // Saved text is what was written, even if the editor moved on.
        ws.set_current("a.typ", "bc".into());
        ws.mark_saved("a.typ", "b", None);
        assert_eq!(ws.dirty_paths().len(), 1);
    }

    #[test]
    fn renames_carry_buffers_and_tabs() {
        let mut ws = Workspace::default();
        ws.open("E:/p/a.typ", text("a")).unwrap();
        ws.set_current("E:/p/a.typ", "edited".into());
        assert!(ws.rename("E:/p/a.typ", "E:/p/b.typ"));
        assert!(ws.is_active("E:/p/b.typ"));
        assert_eq!(ws.buffer("E:/p/b.typ").unwrap().current, "edited");
        assert!(ws.buffer("E:/p/a.typ").is_none());
        assert_eq!(ws.tabs().count(), 1);
        assert_eq!(ws.paths_under("E:/p"), vec![normalize("E:/p/b.typ")]);
        assert!(ws.paths_under("E:/q").is_empty());
    }

    #[test]
    fn outside_changes_are_detected_and_resolved() {
        let dir = std::env::temp_dir().join(format!("zj-ws-disk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.typ");
        std::fs::write(&path, "v1").unwrap();
        let key = path.display().to_string();
        let mut ws = Workspace::default();
        ws.open(&key, |p| crate::files::read_text(Path::new(p)))
            .unwrap();
        assert!(ws.changed_on_disk().is_empty());

        std::fs::write(&path, "version two").unwrap();
        let changed = ws.changed_on_disk();
        assert_eq!(changed.len(), 1);
        // Reloading takes the outside text and clears the report.
        ws.reload(&key, "version two".into(), changed[0].1);
        assert!(ws.changed_on_disk().is_empty());
        assert!(ws.dirty_paths().is_empty());

        // Keeping the editor's text instead leaves the buffer unsaved.
        std::fs::write(&path, "third, from elsewhere").unwrap();
        ws.keep_mine(&key, crate::files::DiskVersion::of(&path));
        assert!(ws.changed_on_disk().is_empty());
        assert_eq!(ws.dirty_paths().len(), 1);

        // A deleted file shows up with no version.
        std::fs::remove_file(&path).unwrap();
        assert_eq!(ws.changed_on_disk(), vec![(normalize(&key), None)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn project_membership() {
        let mut ws = Workspace::default();
        ws.reset(Some(PathBuf::from("E:/proj")));
        assert!(ws.contains(Path::new("E:/proj/chapters/a.typ")));
        assert!(!ws.contains(Path::new("E:/other/a.typ")));
        assert!(ws.is_root(Path::new("E:/proj")));
    }
}
