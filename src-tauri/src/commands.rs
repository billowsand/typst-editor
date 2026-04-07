use crate::compiler::CompilerState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;
use tauri::{AppHandle, State};

// ── Data Types ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct PositionResult {
    pub page: usize,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JumpResult {
    #[serde(rename = "type")]
    pub jump_type: String,
    pub file: Option<String>,
    pub offset: Option<usize>,
    pub url: Option<String>,
    pub position: Option<PositionResult>,
}

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
) -> Result<crate::compiler::CompileResult, String> {
    state.open(&file_path, &content)?;
    Ok(state.update_and_compile(&content))
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

#[tauri::command]
pub fn compile_current(
    state: State<'_, CompilerState>,
    file_path: String,
    content: String,
) -> crate::compiler::CompileResult {
    // Spawn a dedicated thread for compilation to avoid blocking the UI.
    // Results are sent back via channel and we block on receiving them.
    let (tx, rx) = channel();
    let inner = state.inner.clone();

    thread::spawn(move || {
        let result = {
            let mut state_guard = inner.write();
            let Some(world) = &mut state_guard.world else {
                tx.send(crate::compiler::CompileResult {
                    success: false,
                    page_count: 0,
                    page_hashes: vec![],
                    errors: vec!["No file is open".into()],
                    warnings: vec![],
                    duration_ms: 0,
                }).ok();
                return;
            };

            if let Err(err) = world.update_source_by_path(&file_path, &content) {
                tx.send(crate::compiler::CompileResult {
                    success: false,
                    page_count: 0,
                    page_hashes: vec![],
                    errors: vec![err],
                    warnings: vec![],
                    duration_ms: 0,
                }).ok();
                return;
            }
            let start = std::time::Instant::now();
            let result = typst::compile::<typst::layout::PagedDocument>(world);
            let duration = start.elapsed();

            let mut compile_result = crate::compiler::CompileResult {
                success: false,
                page_count: 0,
                page_hashes: vec![],
                errors: vec![],
                warnings: vec![],
                duration_ms: duration.as_millis() as u64,
            };

            for warning in &result.warnings {
                compile_result
                    .warnings
                    .push(crate::compiler::format_diagnostic(warning, world));
            }

            match result.output {
                Ok(doc) => {
                    use crate::compiler::hash_page;
                    let rendered_pages: Vec<String> =
                        doc.pages.iter().map(typst_svg::svg).collect();
                    let page_hashes = rendered_pages
                        .iter()
                        .map(|svg| hash_page(svg))
                        .collect::<Vec<_>>();

                    compile_result.success = true;
                    compile_result.page_count = doc.pages.len();
                    compile_result.page_hashes = page_hashes.clone();
                    state_guard.document = Some(std::sync::Arc::new(doc));
                    state_guard.rendered_pages = rendered_pages;
                    state_guard.page_hashes = page_hashes;
                    state_guard.last_compile_ok = true;
                }
                Err(errors) => {
                    for error in errors {
                        compile_result
                            .errors
                            .push(crate::compiler::format_diagnostic(&error, world));
                    }
                    state_guard.last_compile_ok = false;
                }
            }

            compile_result
        };
        tx.send(result).ok();
    });

    // Block on the result - but the compilation is happening in the spawned thread
    rx.recv().unwrap_or_else(|_| crate::compiler::CompileResult {
        success: false,
        page_count: 0,
        page_hashes: vec![],
        errors: vec!["Channel error".into()],
        warnings: vec![],
        duration_ms: 0,
    })
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

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[tauri::command]
pub fn get_project_root(state: State<'_, CompilerState>) -> Option<String> {
    state.project_root().map(|p| p.display().to_string())
}

#[tauri::command]
pub fn resolve_project_root(file_path: String) -> Option<String> {
    let abs = PathBuf::from(file_path);
    crate::compiler::world::discover_project_root(&abs)
        .or_else(|| abs.parent().map(|p| p.to_path_buf()))
        .map(|p| p.display().to_string())
}

#[tauri::command]
pub fn list_dir(path: String) -> Result<Vec<FileEntry>, String> {
    let dir = std::fs::read_dir(&path).map_err(|e| format!("Failed to read dir: {}", e))?;
    let mut entries: Vec<FileEntry> = dir
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            // Skip hidden files/dirs
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

    // Sort: directories first, then files, alphabetically within each group
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

// ── Sync / Jump ─────────────────────────────────────────────

#[tauri::command]
pub fn jump_from_cursor(
    state: State<'_, CompilerState>,
    file_path: String,
    cursor: usize,
) -> Vec<PositionResult> {
    state
        .with_document(|world, doc| {
            let Some(source) = world.source_by_path(&file_path) else {
                return Vec::new();
            };
            let positions = typst_ide::jump_from_cursor(doc, &source, cursor);
            positions
                .into_iter()
                .map(|pos| PositionResult {
                    page: pos.page.get(),
                    x: pos.point.x.to_pt(),
                    y: pos.point.y.to_pt(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub fn jump_from_click(
    state: State<'_, CompilerState>,
    page: usize,
    x: f64,
    y: f64,
) -> Option<JumpResult> {
    state.with_document(|world, doc| {
        let page_idx = page.saturating_sub(1);
        if page_idx >= doc.pages.len() {
            return None;
        }

        let click = typst::layout::Point::new(typst::layout::Abs::pt(x), typst::layout::Abs::pt(y));

        let frame = &doc.pages[page_idx].frame;
        let jump = typst_ide::jump_from_click(world, doc, frame, click)?;

        Some(match jump {
            typst_ide::Jump::File(file_id, offset) => {
                let name = world
                    .resolve_absolute_path(file_id)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| file_id.vpath().as_rootless_path().display().to_string());
                JumpResult {
                    jump_type: "file".into(),
                    file: Some(name),
                    offset: Some(offset),
                    url: None,
                    position: None,
                }
            }
            typst_ide::Jump::Url(url) => JumpResult {
                jump_type: "url".into(),
                file: None,
                offset: None,
                url: Some(url.to_string()),
                position: None,
            },
            typst_ide::Jump::Position(pos) => JumpResult {
                jump_type: "position".into(),
                file: None,
                offset: None,
                url: None,
                position: Some(PositionResult {
                    page: pos.page.get(),
                    x: pos.point.x.to_pt(),
                    y: pos.point.y.to_pt(),
                }),
            },
        })
    })?
}
