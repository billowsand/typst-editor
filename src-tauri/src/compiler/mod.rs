pub(crate) mod world;

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use typst::layout::PagedDocument;
use typst::World;
use world::{discover_project_root, TypstWorld};

/// Shared compiler state managed by Tauri.
pub struct CompilerState {
    pub(crate) inner: Arc<RwLock<InnerState>>,
}

pub(crate) struct InnerState {
    pub preview_entry: Option<PathBuf>,
    pub pinned_preview_entry: Option<PathBuf>,
    pub overlays: HashMap<PathBuf, String>,
    pub route_table: HashMap<PathBuf, PathBuf>,
    pub world: Option<TypstWorld>,
    pub document: Option<Arc<PagedDocument>>,
    pub rendered_pages: Vec<String>,
    pub page_hashes: Vec<u64>,
    pub last_compile_ok: bool,
}

impl CompilerState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(InnerState {
                preview_entry: None,
                pinned_preview_entry: None,
                overlays: HashMap::new(),
                route_table: HashMap::new(),
                world: None,
                document: None,
                rendered_pages: Vec::new(),
                page_hashes: Vec::new(),
                last_compile_ok: false,
            })),
        }
    }

    /// Force the preview to use this file as entry.
    pub fn open(&self, file_path: &str, content: &str) -> Result<(), String> {
        let entry = absolutize_editor_path(file_path)?;
        let mut state = self.inner.write();
        state.overlays.insert(entry.clone(), content.to_string());
        state.preview_entry = Some(entry);
        Ok(())
    }

    /// Pins or unpins the preview entry explicitly.
    pub fn pin_preview_entry(&self, file_path: Option<&str>) -> Result<(), String> {
        let mut state = self.inner.write();
        state.pinned_preview_entry = match file_path {
            Some(path) => Some(absolutize_editor_path(path)?),
            None => None,
        };
        Ok(())
    }

    pub fn reset_session(&self) {
        let mut state = self.inner.write();
        state.preview_entry = None;
        state.pinned_preview_entry = None;
        state.overlays.clear();
        state.route_table.clear();
        state.world = None;
        state.document = None;
        state.rendered_pages.clear();
        state.page_hashes.clear();
        state.last_compile_ok = false;
    }

    /// Update any file content and compile the resolved entry for it.
    pub fn update_file_and_compile(&self, file_path: &str, content: &str) -> CompileResult {
        let abs = match absolutize_editor_path(file_path) {
            Ok(path) => path,
            Err(err) => {
                return CompileResult::error(vec![err]);
            }
        };

        let mut state = self.inner.write();
        state.overlays.insert(abs.clone(), content.to_string());

        let entry = resolve_entry_for_file(&state, &abs);
        let previous_entry = state.preview_entry.clone();
        let entry_content = match load_overlay_or_disk(&state, &entry) {
            Ok(content) => content,
            Err(err) => return CompileResult::error(vec![err]),
        };

        let root = resolve_root_for_path(&entry);
        let mut world = match TypstWorld::new_with_root(
            entry.to_string_lossy().as_ref(),
            &entry_content,
            &root,
        ) {
            Ok(world) => world,
            Err(err) => {
                return CompileResult::error(vec![format!("Failed to create TypstWorld: {err}")])
            }
        };

        for (path, overlay) in state.overlays.iter() {
            if *path == entry || !path_starts_with_root(path, &root) {
                continue;
            }

            if let Err(err) = world.update_source_by_path(path.to_string_lossy().as_ref(), overlay)
            {
                return CompileResult::error(vec![err]);
            }
        }

        let start = std::time::Instant::now();
        let result = typst::compile::<PagedDocument>(&world);
        let duration = start.elapsed();

        let mut compile_result = CompileResult {
            success: false,
            page_count: 0,
            page_hashes: vec![],
            errors: vec![],
            warnings: vec![],
            duration_ms: duration.as_millis() as u64,
            entry_path: Some(entry.display().to_string()),
        };

        for warning in &result.warnings {
            compile_result
                .warnings
                .push(format_diagnostic(warning, &world));
        }

        match result.output {
            Ok(doc) => {
                let rendered_pages: Vec<String> = doc.pages.iter().map(typst_svg::svg).collect();
                let page_hashes = rendered_pages
                    .iter()
                    .map(|svg| hash_page(svg))
                    .collect::<Vec<_>>();
                let depended_paths = world.depended_paths();

                compile_result.success = true;
                compile_result.page_count = doc.pages.len();
                compile_result.page_hashes = page_hashes.clone();

                state.preview_entry = Some(entry.clone());
                state.world = Some(world);
                state.document = Some(Arc::new(doc));
                state.rendered_pages = rendered_pages;
                state.page_hashes = page_hashes;
                state.last_compile_ok = true;

                for dep in depended_paths {
                    state.route_table.insert(dep, entry.clone());
                }
                state.route_table.insert(abs, entry);
            }
            Err(errors) => {
                for error in errors {
                    compile_result
                        .errors
                        .push(format_diagnostic(&error, &world));
                }
                state.preview_entry = Some(entry);
                state.world = Some(world);
                if previous_entry.as_ref() != state.preview_entry.as_ref() {
                    state.document = None;
                    state.rendered_pages.clear();
                    state.page_hashes.clear();
                }
                state.last_compile_ok = false;
            }
        }

        compile_result
    }

    /// Render a specific page as SVG.
    pub fn render_page_svg(&self, page_index: usize) -> Option<String> {
        let state = self.inner.read();
        if page_index >= state.rendered_pages.len() {
            return None;
        }
        Some(state.rendered_pages[page_index].clone())
    }

    /// Export the current document as PDF bytes.
    pub fn export_pdf(&self) -> Result<Vec<u8>, String> {
        let state = self.inner.read();
        if state.document.is_none() {
            return Err("No compiled document available".into());
        }
        if !state.last_compile_ok {
            return Err("Last compilation had errors. Fix errors before exporting.".into());
        }
        let doc = state.document.as_ref().unwrap();
        let options = typst_pdf::PdfOptions {
            ident: typst::foundations::Smart::Auto,
            timestamp: None,
            page_ranges: None,
            standards: typst_pdf::PdfStandards::default(),
            tagged: true,
        };
        typst_pdf::pdf(doc, &options).map_err(|e| {
            e.into_iter()
                .map(|d| d.message.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    /// Get the project root directory path for the active preview entry.
    pub fn project_root(&self) -> Option<PathBuf> {
        let state = self.inner.read();
        let entry = state.preview_entry.as_ref()?;
        Some(resolve_root_for_path(entry))
    }

    /// Get the current preview entry path.
    pub fn preview_entry(&self) -> Option<PathBuf> {
        self.inner.read().preview_entry.clone()
    }

    /// Get the page count of the current document.
    pub fn page_count(&self) -> usize {
        let state = self.inner.read();
        state.document.as_ref().map(|d| d.pages.len()).unwrap_or(0)
    }

    /// Get references for IDE features.
    pub fn with_document<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&TypstWorld, &PagedDocument) -> R,
    {
        let state = self.inner.read();
        let world = state.world.as_ref()?;
        let doc = state.document.as_ref()?;
        Some(f(world, doc))
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CompileResult {
    pub success: bool,
    pub page_count: usize,
    pub page_hashes: Vec<u64>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub duration_ms: u64,
    pub entry_path: Option<String>,
}

impl CompileResult {
    fn error(errors: Vec<String>) -> Self {
        Self {
            success: false,
            page_count: 0,
            page_hashes: vec![],
            errors,
            warnings: vec![],
            duration_ms: 0,
            entry_path: None,
        }
    }
}

fn resolve_entry_for_file(state: &InnerState, file_path: &Path) -> PathBuf {
    if let Some(entry) = state.pinned_preview_entry.as_ref() {
        return entry.clone();
    }

    if let Some(entry) = state.route_table.get(file_path) {
        return entry.clone();
    }

    if let Some(entry) = state.preview_entry.as_ref() {
        if resolve_root_for_path(entry) == resolve_root_for_path(file_path) {
            return entry.clone();
        }
    }

    file_path.to_path_buf()
}

fn load_overlay_or_disk(state: &InnerState, path: &Path) -> Result<String, String> {
    if let Some(content) = state.overlays.get(path) {
        return Ok(content.clone());
    }

    std::fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))
}

fn resolve_root_for_path(path: &Path) -> PathBuf {
    discover_project_root(path)
        .or_else(|| path.parent().map(|p| p.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn absolutize_editor_path(path: &str) -> Result<PathBuf, String> {
    let raw = PathBuf::from(path);
    if raw.is_absolute() {
        Ok(raw)
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(raw))
            .map_err(|e| format!("Failed to resolve {}: {}", path, e))
    }
}

fn path_starts_with_root(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

pub(crate) fn hash_page(svg: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    svg.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn format_diagnostic(diag: &typst::diag::SourceDiagnostic, world: &dyn World) -> String {
    let span = diag.span;
    let mut msg = String::new();
    if let Some(id) = span.id() {
        if let Ok(source) = world.source(id) {
            if let Some(range) = span.range() {
                let line = source.lines().byte_to_line(range.start).unwrap_or(0) + 1;
                msg.push_str(&format!(
                    "{}:{}: ",
                    source.id().vpath().as_rootless_path().display(),
                    line
                ));
            } else {
                msg.push_str(&format!(
                    "{}: ",
                    source.id().vpath().as_rootless_path().display()
                ));
            }
        }
    }
    msg.push_str(&diag.message);
    msg
}
