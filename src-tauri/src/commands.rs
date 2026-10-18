use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, State};
use typst_core::fs::FileEntry;
use typst_core::ide::{JumpResult, PositionResult};
use typst_core::{CompileResult, CompilerState, OutlineItem};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

// ── Data Types ──────────────────────────────────────────────

/// Return type for `export_pdf_dialog`: distinguishes cancel from success.
#[derive(Debug, Serialize)]
pub enum ExportResult {
    /// User cancelled the dialog.
    Cancelled,
    /// PDF was written successfully.
    Success,
}

// ── File Operations ─────────────────────────────────────────

/// Read a text file from disk (used by the frontend to load file content).
#[tauri::command]
pub fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read {}: {}", path, e))
}

/// Open a Typst project: creates a new World rooted at the file's directory,
/// then compiles.
#[tauri::command]
pub fn open_file(
    state: State<'_, CompilerState>,
    file_path: String,
    content: String,
) -> Result<CompileResult, String> {
    let _guard = state.compile_lock();
    state.open(&file_path, &content)?;
    let mut result = state.update_file_and_compile(&file_path, &content);
    typst_core::ide::populate_outline(&state, &mut result);
    Ok(result)
}

#[tauri::command]
pub fn set_preview_entry(
    state: State<'_, CompilerState>,
    file_path: String,
    content: String,
) -> Result<CompileResult, String> {
    let _guard = state.compile_lock();
    state.open(&file_path, &content)?;
    state.pin_preview_entry(Some(&file_path))?;
    // NOTE: do NOT call note_compile_request here.
    // set_preview_entry uses update_file_and_compile (not _with_id), so it
    // bypasses request deduplication. Calling note_compile_request with a
    // sentinel like u64::MAX would corrupt current_request_id and cause
    // subsequent compile_current calls to return stale results.
    let mut result = state.update_file_and_compile(&file_path, &content);
    typst_core::ide::populate_outline(&state, &mut result);
    Ok(result)
}

#[tauri::command]
pub fn clear_preview_entry_pin(state: State<'_, CompilerState>) -> Result<(), String> {
    state.pin_preview_entry(None)
}

#[tauri::command]
pub fn reset_session(state: State<'_, CompilerState>) {
    state.reset_session();
}

#[tauri::command]
pub fn save_file(content: String, file_path: String) -> Result<(), String> {
    std::fs::write(&file_path, &content).map_err(|e| format!("Failed to save: {}", e))
}

#[tauri::command]
pub async fn open_file_dialog(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let file_path = app
        .dialog()
        .file()
        .add_filter("Typst", &["typ"])
        .blocking_pick_file();
    Ok(file_path.map(|p| p.to_string()))
}

#[tauri::command]
pub async fn save_file_dialog(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let file_path = app
        .dialog()
        .file()
        .add_filter("Typst", &["typ"])
        .set_file_name("document.typ")
        .blocking_save_file();
    Ok(file_path.map(|p| p.to_string()))
}

/// Export PDF. Returns `ExportResult::Cancelled` when the user cancels the
/// dialog so the frontend can display an appropriate message.
#[tauri::command]
pub async fn export_pdf_dialog(
    app: AppHandle,
    state: State<'_, CompilerState>,
) -> Result<ExportResult, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(path) = app
        .dialog()
        .file()
        .add_filter("PDF", &["pdf"])
        .set_file_name("document.pdf")
        .blocking_save_file()
    else {
        return Ok(ExportResult::Cancelled);
    };

    let pdf_bytes = state.export_pdf()?;

    let path_str = path.as_path().ok_or("Invalid file path")?;
    std::fs::write(path_str, &pdf_bytes).map_err(|e| format!("Failed to write PDF: {}", e))?;

    Ok(ExportResult::Success)
}

// ── Compilation ─────────────────────────────────────────────

/// Atomic counter for compile request deduplication
static COMPILE_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

#[tauri::command]
pub fn compile_current(
    state: State<'_, CompilerState>,
    file_path: String,
    content: String,
) -> CompileResult {
    let request_id = COMPILE_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    state.note_compile_request(request_id);

    let _guard = state.compile_lock();
    let mut result = if state.is_stale_request(request_id) {
        state.current_result()
    } else {
        state.update_file_and_compile_with_id(request_id, &file_path, &content)
    };
    typst_core::ide::populate_outline(&state, &mut result);
    result
}

#[tauri::command]
pub fn render_page(state: State<'_, CompilerState>, page: usize) -> Option<String> {
    state.render_page_svg(page)
}

#[tauri::command]
pub fn page_count(state: State<'_, CompilerState>) -> usize {
    state.page_count()
}

// ── File Tree ───────────────────────────────────────────────

#[tauri::command]
pub fn get_project_root(state: State<'_, CompilerState>) -> Option<String> {
    state.project_root().map(|p| p.display().to_string())
}

#[tauri::command]
pub fn get_preview_entry(state: State<'_, CompilerState>) -> Option<String> {
    state.preview_entry().map(|p| p.display().to_string())
}

#[tauri::command]
pub fn get_document_outline(state: State<'_, CompilerState>) -> Vec<OutlineItem> {
    typst_core::ide::document_outline(&state)
}

#[tauri::command]
pub fn resolve_project_root(file_path: String) -> Option<String> {
    let abs = std::path::PathBuf::from(file_path);
    typst_core::discover_project_root(&abs)
        .or_else(|| abs.parent().map(|p| p.to_path_buf()))
        .map(|p| p.display().to_string())
}

#[tauri::command]
pub fn list_dir(path: String) -> Result<Vec<FileEntry>, String> {
    typst_core::fs::list_dir(&path)
}

// ── Sync / Jump ─────────────────────────────────────────────

#[tauri::command]
pub fn jump_from_cursor(
    state: State<'_, CompilerState>,
    file_path: String,
    line: u32,
    character: u32,
) -> Vec<PositionResult> {
    typst_core::ide::jump_from_cursor(&state, &file_path, line as usize, character as usize)
}

#[tauri::command]
pub fn jump_from_offset(
    state: State<'_, CompilerState>,
    file_path: String,
    offset: usize,
) -> Vec<PositionResult> {
    typst_core::ide::jump_from_offset(&state, &file_path, offset)
}

#[tauri::command]
pub fn jump_from_click(
    state: State<'_, CompilerState>,
    page: usize,
    x: f64,
    y: f64,
) -> Option<JumpResult> {
    typst_core::ide::jump_from_click(&state, page, x, y)
}

// ── Font Discovery ──────────────────────────────────────────

#[tauri::command]
pub fn list_system_fonts() -> Vec<String> {
    typst_core::fs::list_system_fonts()
}

// ── File System Operations ───────────────────────────────────

/// Create a new empty file (creates parent directories as needed).
#[tauri::command]
pub fn create_file(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create parent dirs: {e}"))?;
    }
    if p.exists() {
        return Err(format!("File already exists: {path}"));
    }
    std::fs::File::create(p).map_err(|e| format!("Cannot create file: {e}"))?;
    Ok(())
}

/// Create a directory (and all parent directories).
#[tauri::command]
pub fn create_directory(path: String) -> Result<(), String> {
    std::fs::create_dir_all(&path).map_err(|e| format!("Cannot create directory: {e}"))
}

/// Delete a file or directory (directories are removed recursively).
#[tauri::command]
pub fn delete_path(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if p.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| format!("Cannot delete directory: {e}"))
    } else {
        std::fs::remove_file(p).map_err(|e| format!("Cannot delete file: {e}"))
    }
}

/// Rename or move a file or directory.
#[tauri::command]
pub fn rename_path(old_path: String, new_path: String) -> Result<(), String> {
    if std::path::Path::new(&new_path).exists() {
        return Err(format!("Target already exists: {new_path}"));
    }
    std::fs::rename(&old_path, &new_path).map_err(|e| format!("Cannot rename: {e}"))
}

/// Reveal a file or folder in the native file manager.
#[tauri::command]
pub fn reveal_in_explorer(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // /select highlights the item; use the parent dir if it's a directory
        let arg = if std::path::Path::new(&path).is_dir() {
            path.clone()
        } else {
            format!("/select,{path}")
        };
        std::process::Command::new("explorer")
            .raw_arg(&arg)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        let dir = std::path::Path::new(&path)
            .parent()
            .unwrap_or(std::path::Path::new("/"))
            .to_string_lossy()
            .into_owned();
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
