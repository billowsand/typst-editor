//! Editing session state, kept free of GPUI so it can be unit-tested:
//! the project root, open buffers in tab order, the active document and a
//! generation counter that invalidates work started for an older session.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One open document: what is on disk and what is in the editor.
#[derive(Debug, Clone)]
pub struct Buffer {
    pub saved: String,
    pub current: String,
}

impl Buffer {
    pub fn is_dirty(&self) -> bool {
        self.current != self.saved
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
        load: impl FnOnce(&str) -> std::io::Result<String>,
    ) -> Result<String, String> {
        let key = normalize(path);
        if !self.buffers.contains_key(&key) {
            let text = load(&key).map_err(|e| format!("无法读取 {key}：{e}"))?;
            self.buffers.insert(
                key.clone(),
                Buffer {
                    saved: text.clone(),
                    current: text,
                },
            );
            self.order.push(key.clone());
        }
        self.active = Some(key.clone());
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

    pub fn mark_saved(&mut self, path: &str) {
        if let Some(buffer) = self.buffers.get_mut(&normalize(path)) {
            buffer.saved = buffer.current.clone();
        }
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

    fn text(s: &str) -> impl FnOnce(&str) -> std::io::Result<String> + '_ {
        move |_| Ok(s.to_string())
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
        ws.mark_saved("a.typ");
        assert!(ws.dirty_paths().is_empty());
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
