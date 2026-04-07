use crate::compiler::CompilerState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;
use tauri::{AppHandle, State};
use typst::foundations::{Content, NativeElement, Packed, StyleChain};
use typst::introspection::Introspector;
use typst::layout::{Frame, FrameItem, Point, Position, Size};
use typst::model::HeadingElem;
use typst::syntax::{Span, SyntaxKind};
use typst::visualize::Geometry;
use typst::Document;
use typst::World;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct OutlineItem {
    pub title: String,
    pub level: usize,
    pub file: Option<String>,
    pub offset: Option<usize>,
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
    Ok(state.update_file_and_compile(&file_path, &content))
}

#[tauri::command]
pub fn set_preview_entry(
    state: State<'_, CompilerState>,
    file_path: String,
    content: String,
) -> Result<crate::compiler::CompileResult, String> {
    state.open(&file_path, &content)?;
    state.pin_preview_entry(Some(&file_path))?;
    Ok(state.update_file_and_compile(&file_path, &content))
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
        let state = crate::compiler::CompilerState { inner };
        let result = state.update_file_and_compile(&file_path, &content);
        tx.send(result).ok();
    });

    // Block on the result - but the compilation is happening in the spawned thread
    rx.recv()
        .unwrap_or_else(|_| crate::compiler::CompileResult {
            success: false,
            page_count: 0,
            page_hashes: vec![],
            errors: vec!["Channel error".into()],
            warnings: vec![],
            duration_ms: 0,
            entry_path: None,
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
pub fn get_preview_entry(state: State<'_, CompilerState>) -> Option<String> {
    state.preview_entry().map(|p| p.display().to_string())
}

#[tauri::command]
pub fn get_document_outline(state: State<'_, CompilerState>) -> Vec<OutlineItem> {
    state
        .with_document(|world, doc| build_document_outline(world, doc.introspector()))
        .unwrap_or_default()
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
    line: u32,
    character: u32,
) -> Vec<PositionResult> {
    state
        .with_document(|world, doc| {
            let Some(source) = world.source_by_path(&file_path) else {
                return Vec::new();
            };
            let Some(cursor) = source
                .lines()
                .line_column_to_byte(line as usize, character as usize)
            else {
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
            typst_ide::Jump::Position(pos) => resolve_source_jump_from_position(world, doc, pos)
                .unwrap_or_else(|| JumpResult {
                    jump_type: "position".into(),
                    file: None,
                    offset: None,
                    url: None,
                    position: Some(PositionResult {
                        page: pos.page.get(),
                        x: pos.point.x.to_pt(),
                        y: pos.point.y.to_pt(),
                    }),
                }),
        })
    })?
}

fn resolve_source_jump_from_position(
    world: &crate::compiler::world::TypstWorld,
    doc: &typst::layout::PagedDocument,
    pos: Position,
) -> Option<JumpResult> {
    let page_idx = pos.page.get().checked_sub(1)?;
    let page = doc.pages.get(page_idx)?;
    let (span, offset) = source_span_from_click(world, &page.frame, pos.point)?;
    let (file, offset) = resolve_span_to_file_offset(world, span, offset)?;

    Some(JumpResult {
        jump_type: "file".into(),
        file: Some(file),
        offset: Some(offset),
        url: None,
        position: None,
    })
}

fn source_span_from_click(
    world: &crate::compiler::world::TypstWorld,
    frame: &Frame,
    click: Point,
) -> Option<(Span, usize)> {
    for &(mut pos, ref item) in frame.items().rev() {
        match item {
            FrameItem::Group(group) => {
                if let Some(hit) = source_span_from_click(world, &group.frame, click - pos) {
                    return Some(hit);
                }
            }
            FrameItem::Text(text) => {
                for glyph in &text.glyphs {
                    let width = glyph.x_advance.at(text.size);
                    let hitbox_pos = Point::new(pos.x, pos.y - text.size);
                    if is_in_rect(hitbox_pos, Size::new(width, text.size), click) {
                        let (span, glyph_offset) = glyph.span;
                        let mut resolved_offset = glyph_offset as usize;
                        let source = world.source(span.id()?).ok()?;
                        let node = source.find(span)?;
                        if matches!(node.kind(), SyntaxKind::Text | SyntaxKind::MathText)
                            && (click.x - pos.x) > width / 2.0
                        {
                            resolved_offset += glyph.range().len();
                        }
                        return Some((span, resolved_offset));
                    }
                    pos.x += width;
                }
            }
            FrameItem::Shape(shape, span) => {
                let Geometry::Rect(size) = shape.geometry else {
                    continue;
                };
                if is_in_rect(pos, size, click) {
                    return Some((*span, 0));
                }
            }
            FrameItem::Image(_, size, span) if is_in_rect(pos, *size, click) => {
                return Some((*span, 0));
            }
            _ => {}
        }
    }

    None
}

fn resolve_span_to_file_offset(
    world: &crate::compiler::world::TypstWorld,
    span: Span,
    extra_offset: usize,
) -> Option<(String, usize)> {
    let source = world.source(span.id()?).ok()?;
    let range = source.find(span)?.range();
    let offset = range.start.saturating_add(extra_offset.min(range.len()));
    let path = world.resolve_absolute_path(span.id()?)?;
    Some((path.display().to_string(), offset))
}

fn is_in_rect(pos: Point, size: Size, click: Point) -> bool {
    pos.x <= click.x && pos.x + size.x >= click.x && pos.y <= click.y && pos.y + size.y >= click.y
}

fn build_document_outline(
    world: &crate::compiler::world::TypstWorld,
    introspector: &Introspector,
) -> Vec<OutlineItem> {
    introspector
        .query(&HeadingElem::ELEM.select())
        .iter()
        .filter_map(|elem| {
            let heading = elem.to_packed::<HeadingElem>()?;
            let bookmarked = heading
                .bookmarked
                .get(StyleChain::default())
                .unwrap_or_else(|| heading.outlined.get(StyleChain::default()));
            if !bookmarked {
                return None;
            }

            let title = plain_outline_title(&heading.body);
            if title.is_empty() {
                return None;
            }

            let location = heading.location()?;
            let doc_pos = introspector.position(location);
            let span = effective_heading_span(&heading);
            let (file, offset) =
                resolve_span_to_file_offset(world, span, 0).unwrap_or((String::new(), 0));
            let has_location = !file.is_empty();

            Some(OutlineItem {
                title,
                level: heading.resolve_level(StyleChain::default()).get(),
                file: has_location.then_some(file),
                offset: has_location.then_some(offset),
                position: Some(PositionResult {
                    page: doc_pos.page.get(),
                    x: doc_pos.point.x.to_pt(),
                    y: doc_pos.point.y.to_pt(),
                }),
            })
        })
        .collect()
}

fn plain_outline_title(content: &Content) -> String {
    content.plain_text().trim().to_string()
}

fn effective_heading_span(heading: &Packed<HeadingElem>) -> Span {
    let span = heading.span();
    if span.is_detached() {
        heading.body.span()
    } else {
        span
    }
}
