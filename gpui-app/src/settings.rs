//! Persistent preferences and the last session, stored as JSON in the data
//! directory. Unknown or missing fields fall back to defaults, so files
//! from older or newer versions always load.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const RECENT_LIMIT: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Settings file format; bump with a migration when fields change shape.
    pub version: u32,
    pub font_px: f32,
    pub compact: bool,
    pub outline_open: bool,
    pub show_sidebar: bool,
    /// "drafts" or "files".
    pub nav: String,
    /// Most recent first.
    pub recent_projects: Vec<String>,
    /// Starred documents (normalized paths).
    pub starred: Vec<String>,
    pub last_session: Option<Session>,
}

/// What was open when the app last closed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Session {
    pub root: String,
    pub tabs: Vec<String>,
    pub active: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            font_px: 15.,
            compact: false,
            outline_open: true,
            show_sidebar: true,
            nav: "drafts".into(),
            recent_projects: vec![],
            starred: vec![],
            last_session: None,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::recovery::data_dir().join("settings.json")
    }

    /// Load from `path`; a missing or unreadable file gives the defaults.
    pub fn load_from(path: &std::path::Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .map(Settings::migrate)
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        crate::files::atomic_write(path, &json).map(|_| ())
    }

    /// Bring older settings up to the current format, clamping values.
    fn migrate(mut self) -> Self {
        self.version = 1;
        self.font_px = self.font_px.clamp(11., 24.);
        self.recent_projects.truncate(RECENT_LIMIT);
        self
    }

    /// Move `root` to the front of the recent list.
    pub fn note_recent(&mut self, root: &str) {
        self.recent_projects.retain(|r| r != root);
        self.recent_projects.insert(0, root.to_string());
        self.recent_projects.truncate(RECENT_LIMIT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "zj-settings-{tag}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn round_trips_and_tolerates_missing_or_unknown_fields() {
        let path = temp_file("rt");
        let mut s = Settings::default();
        s.font_px = 17.;
        s.note_recent("E:/a");
        s.last_session = Some(Session {
            root: "E:/a".into(),
            tabs: vec!["E:/a/main.typ".into()],
            active: Some("E:/a/main.typ".into()),
        });
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);

        // A file from another version: missing fields, an unknown one.
        std::fs::write(&path, r#"{"font_px": 40, "future_field": true}"#).unwrap();
        let loaded = Settings::load_from(&path);
        assert_eq!(loaded.font_px, 24., "clamped");
        assert!(loaded.outline_open, "missing field takes its default");

        std::fs::write(&path, b"garbage").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn recent_projects_are_unique_and_bounded() {
        let mut s = Settings::default();
        for i in 0..12 {
            s.note_recent(&format!("p{i}"));
        }
        s.note_recent("p5");
        assert_eq!(s.recent_projects.len(), RECENT_LIMIT);
        assert_eq!(s.recent_projects[0], "p5");
        assert_eq!(s.recent_projects.iter().filter(|r| *r == "p5").count(), 1);
    }
}
