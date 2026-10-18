use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// List a directory: hidden entries skipped, directories first, then by name.
pub fn list_dir(path: &str) -> Result<Vec<FileEntry>, String> {
    let dir = std::fs::read_dir(path).map_err(|e| format!("Failed to read dir: {}", e))?;
    let mut entries: Vec<FileEntry> = dir
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let file_type = e.file_type().ok()?;
            Some(FileEntry {
                name,
                path: e.path().display().to_string(),
                is_dir: file_type.is_dir(),
            })
        })
        .collect();

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

/// Return all font family names discovered by typst-kit's font searcher.
pub fn list_system_fonts() -> Vec<String> {
    let fonts = crate::world::get_or_init_fonts();
    let mut families: Vec<String> = fonts
        .book
        .families()
        .map(|(name, _)| name.to_string())
        .collect();
    families.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    families.dedup();
    families
}
