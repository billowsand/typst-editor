use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;

use crate::world::{discover_access_root, discover_project_root, TypstWorld};
use parking_lot::RwLock;
use typst::layout::PagedDocument;
use typst::World;

/// Mutex to prevent concurrent compilations from racing
static COMPILE_MUTEX: Mutex<()> = Mutex::new(());

fn import_trace_enabled() -> bool {
    cfg!(debug_assertions)
        || matches!(
            std::env::var("TYPST_EDITOR_TRACE_IMPORTS").as_deref(),
            Ok("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
        )
}

fn trace_import(message: impl AsRef<str>) {
    if import_trace_enabled() {
        eprintln!("[typst-editor][compile] {}", message.as_ref());
    }
}

/// Shared compiler state, owned by the host app (Tauri or GPUI).
pub struct CompilerState {
    pub(crate) inner: Arc<RwLock<InnerState>>,
}

pub(crate) struct InnerState {
    pub preview_entry: Option<PathBuf>,
    pub pinned_preview_entry: Option<PathBuf>,
    pub overlays: HashMap<PathBuf, String>,
    pub overlay_hashes: HashMap<PathBuf, u64>, // Track content hashes for incremental compilation
    pub route_table: HashMap<PathBuf, PathBuf>,
    pub world: Option<TypstWorld>,
    pub document: Option<Arc<PagedDocument>>,
    pub rendered_pages: Vec<Option<String>>,
    pub page_hashes: Vec<u64>,
    pub last_compile_ok: bool,
    pub current_request_id: u64, // Track current compile request for deduplication
    /// What the cached `document` was compiled from. The fast path may only
    /// reuse it when all of these still match.
    pub doc_key: Option<DocKey>,
    /// Bumped by every successful compile; identifies a document snapshot.
    pub revision: u64,
    /// Bumped by `reset_session`; a compile that started in an older session
    /// must not store its results.
    pub session: u64,
    /// Warnings (and their structured form) of the cached document, returned
    /// again when the cache is reused so squiggles do not disappear.
    pub doc_warnings: Vec<String>,
    pub doc_diagnostics: Vec<DiagnosticInfo>,
}

/// Identity of a compiled document: its entry, root, the in-memory overlays
/// it saw and the on-disk files it read.
#[derive(Clone, PartialEq)]
pub(crate) struct DocKey {
    entry: PathBuf,
    root: PathBuf,
    overlays: u64,
    deps: Vec<(PathBuf, Option<FileStamp>)>,
}

/// Cheap change detector for a file on disk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FileStamp {
    modified: Option<std::time::SystemTime>,
    len: u64,
}

impl FileStamp {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// Options for a single compile request.
#[derive(Clone, Copy, Default, Debug)]
pub struct CompileOptions {
    /// Skip the cached document and recompile from scratch (F5).
    pub force: bool,
}

impl CompilerState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(InnerState {
                preview_entry: None,
                pinned_preview_entry: None,
                overlays: HashMap::new(),
                overlay_hashes: HashMap::new(),
                route_table: HashMap::new(),
                world: None,
                document: None,
                rendered_pages: Vec::new(),
                page_hashes: Vec::new(),
                last_compile_ok: false,
                current_request_id: 0,
                doc_key: None,
                revision: 0,
                session: 0,
                doc_warnings: Vec::new(),
                doc_diagnostics: Vec::new(),
            })),
        }
    }

    /// Acquire the global compilation lock so only one compile mutates state at a time.
    pub fn compile_lock(&self) -> MutexGuard<'static, ()> {
        COMPILE_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Record the latest compile request ID.
    pub fn note_compile_request(&self, request_id: u64) {
        let mut state = self.inner.write();
        state.current_request_id = state.current_request_id.max(request_id);
    }

    /// Whether a newer compile request has superseded `request_id`.
    pub fn is_stale_request(&self, request_id: u64) -> bool {
        request_id < self.inner.read().current_request_id
    }

    /// Snapshot of the last compilation, without recompiling.
    pub fn current_result(&self) -> CompileResult {
        let state = self.inner.read();
        let pinned_entry_path = state
            .pinned_preview_entry
            .as_ref()
            .map(|p| p.display().to_string());
        match &state.document {
            Some(doc) => CompileResult {
                success: state.last_compile_ok,
                page_count: doc.pages.len(),
                page_hashes: state.page_hashes.clone(),
                outline: vec![],
                errors: vec![],
                warnings: vec![],
                diagnostics: vec![],
                duration_ms: 0,
                entry_path: state
                    .preview_entry
                    .as_ref()
                    .map(|p| p.display().to_string()),
                pinned_entry_path,
                revision: state.revision,
            },
            None => CompileResult {
                pinned_entry_path,
                ..CompileResult::error(vec!["No document available".into()])
            },
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
        state.overlay_hashes.clear();
        state.route_table.clear();
        state.world = None;
        state.document = None;
        state.rendered_pages.clear();
        state.page_hashes.clear();
        state.last_compile_ok = false;
        state.current_request_id = 0;
        state.doc_key = None;
        state.doc_warnings.clear();
        state.doc_diagnostics.clear();
        state.session += 1;
    }

    /// Update any file content and compile with request ID tracking for deduplication.
    pub fn update_file_and_compile_with_id(
        &self,
        _request_id: u64,
        file_path: &str,
        content: &str,
    ) -> CompileResult {
        self.compile_file(file_path, content, CompileOptions::default())
    }

    /// Update any file content and compile the resolved entry for it.
    pub fn update_file_and_compile(&self, file_path: &str, content: &str) -> CompileResult {
        self.compile_file(file_path, content, CompileOptions::default())
    }

    /// Record `content` as the editor text of `file_path`, then compile the
    /// entry that file belongs to.
    ///
    /// The cached document is reused only when it was compiled from the same
    /// entry and root, the same in-memory overlays, and dependency files that
    /// are unchanged on disk. The typst compile itself runs without holding
    /// the state lock, so readers (preview, IDE queries) never wait for it;
    /// callers serialize compiles with [`Self::compile_lock`].
    pub fn compile_file(
        &self,
        file_path: &str,
        content: &str,
        options: CompileOptions,
    ) -> CompileResult {
        let abs = match absolutize_editor_path(file_path) {
            Ok(path) => path,
            Err(err) => return CompileResult::error(vec![err]),
        };

        // Phase 1: record the overlay, then use the cache or prepare a world.
        let (world, entry, root, overlays, overlay_paths, session) = {
            let mut state = self.inner.write();
            state.overlays.insert(abs.clone(), content.to_string());
            state
                .overlay_hashes
                .insert(abs.clone(), hash_content(content));

            let entry = resolve_entry_for_file(&state, &abs);
            let root = resolve_access_root_for_path(&entry);
            let overlays = overlay_fingerprint(&state.overlay_hashes);
            trace_import(format!(
                "compile request file={} entry={} root={} force={}",
                abs.display(),
                entry.display(),
                root.display(),
                options.force
            ));

            let cache_valid = !options.force
                && state.last_compile_ok
                && state.document.is_some()
                && state.doc_key.as_ref().is_some_and(|key| {
                    key.entry == entry
                        && key.root == root
                        && key.overlays == overlays
                        && key
                            .deps
                            .iter()
                            .all(|(path, stamp)| FileStamp::of(path) == *stamp)
                });
            if cache_valid {
                state.preview_entry = Some(entry.clone());
                state.route_table.insert(abs, entry);
                return Self::cached_result(&state);
            }

            let entry_content = match load_overlay_or_disk(&state, &entry) {
                Ok(content) => content,
                Err(err) => return CompileResult::error(vec![err]),
            };
            let reusable = state
                .world
                .as_ref()
                .is_some_and(|w| w.main_path() == entry.as_path() && w.root() == root.as_path());
            let mut world = if reusable {
                let mut world = state.world.take().expect("checked above");
                world.update_source(&entry_content);
                world.reset_touched_paths();
                world
            } else {
                match TypstWorld::new_with_root(
                    entry.to_string_lossy().as_ref(),
                    &entry_content,
                    &root,
                ) {
                    Ok(world) => world,
                    Err(err) => {
                        return CompileResult::error(vec![format!(
                            "Failed to create TypstWorld: {err}"
                        )])
                    }
                }
            };
            // Every overlay inside the root, not just changed ones: a fresh
            // world has none of them yet. Identical text is skipped cheaply.
            for (path, overlay) in state.overlays.iter() {
                if *path == entry || !path_starts_with_root(path, &root) {
                    continue;
                }
                if let Err(err) =
                    world.update_source_by_path(path.to_string_lossy().as_ref(), overlay)
                {
                    return CompileResult::error(vec![err]);
                }
            }
            let overlay_paths: Vec<PathBuf> = state.overlays.keys().cloned().collect();
            (world, entry, root, overlays, overlay_paths, state.session)
        };

        // Phase 2: compile without holding the state lock.
        let start = std::time::Instant::now();
        let result = typst::compile::<PagedDocument>(&world);
        let duration = start.elapsed();

        let mut compile_result = CompileResult {
            duration_ms: duration.as_millis() as u64,
            entry_path: Some(entry.display().to_string()),
            ..CompileResult::error(vec![])
        };
        for warning in &result.warnings {
            compile_result
                .warnings
                .push(format_diagnostic(warning, &world));
            compile_result
                .diagnostics
                .extend(diagnostic_info(warning, &world));
        }
        let depended_paths = world.depended_paths();
        // Files read from disk, stamped so the cache notices outside edits.
        let deps: Vec<(PathBuf, Option<FileStamp>)> = depended_paths
            .iter()
            .filter(|path| !overlay_paths.contains(path))
            .map(|path| (path.clone(), FileStamp::of(path)))
            .collect();
        let output = match result.output {
            Ok(doc) => Some(doc),
            Err(errors) => {
                for error in errors.iter() {
                    compile_result.errors.push(format_diagnostic(error, &world));
                    compile_result
                        .diagnostics
                        .extend(diagnostic_info(error, &world));
                }
                None
            }
        };

        // Phase 3: store, unless the session was reset meanwhile.
        let mut state = self.inner.write();
        if state.session != session {
            return CompileResult::error(vec!["Session was reset during compilation".into()]);
        }
        compile_result.pinned_entry_path = state
            .pinned_preview_entry
            .as_ref()
            .map(|path| path.display().to_string());
        let previous_doc_entry = state.doc_key.as_ref().map(|k| k.entry.clone());
        state.preview_entry = Some(entry.clone());
        state.world = Some(world);
        for dep in depended_paths {
            state.route_table.insert(dep, entry.clone());
        }
        state.route_table.insert(abs, entry.clone());

        match output {
            Some(doc) => {
                trace_import(format!(
                    "compile success entry={} pages={}",
                    entry.display(),
                    doc.pages.len()
                ));
                // Keep cached SVG strings only for pages whose hash is stable.
                let mut rendered_pages = Vec::with_capacity(doc.pages.len());
                let mut page_hashes = Vec::with_capacity(doc.pages.len());
                for (i, page) in doc.pages.iter().enumerate() {
                    let new_hash = hash_page(page);
                    let cached = match (state.rendered_pages.get(i), state.page_hashes.get(i)) {
                        (Some(svg), Some(old)) if *old == new_hash => svg.clone(),
                        _ => None,
                    };
                    rendered_pages.push(cached);
                    page_hashes.push(new_hash);
                }
                state.revision += 1;
                compile_result.success = true;
                compile_result.page_count = doc.pages.len();
                compile_result.page_hashes = page_hashes.clone();
                compile_result.revision = state.revision;
                state.document = Some(Arc::new(doc));
                state.rendered_pages = rendered_pages;
                state.page_hashes = page_hashes;
                state.last_compile_ok = true;
                state.doc_key = Some(DocKey {
                    entry,
                    root,
                    overlays,
                    deps,
                });
                state.doc_warnings = compile_result.warnings.clone();
                state.doc_diagnostics = compile_result.diagnostics.clone();
            }
            None => {
                trace_import(format!(
                    "compile failure entry={} error_count={}",
                    entry.display(),
                    compile_result.errors.len()
                ));
                // The old document stays on screen only if it is this
                // entry's; another entry's document must not pass as current.
                if previous_doc_entry.as_ref() != Some(&entry) {
                    state.document = None;
                    state.rendered_pages.clear();
                    state.page_hashes.clear();
                }
                state.last_compile_ok = false;
                state.doc_key = state
                    .doc_key
                    .take()
                    .filter(|key| key.entry == entry)
                    .map(|key| DocKey {
                        // Never valid for the fast path: the source changed.
                        overlays: key.overlays.wrapping_add(1),
                        ..key
                    });
            }
        }
        compile_result
    }

    /// The cached document as a compile result (fast path).
    fn cached_result(state: &InnerState) -> CompileResult {
        CompileResult {
            success: true,
            page_count: state.document.as_ref().map_or(0, |d| d.pages.len()),
            page_hashes: state.page_hashes.clone(),
            entry_path: state
                .preview_entry
                .as_ref()
                .map(|p| p.display().to_string()),
            pinned_entry_path: state
                .pinned_preview_entry
                .as_ref()
                .map(|p| p.display().to_string()),
            revision: state.revision,
            warnings: state.doc_warnings.clone(),
            diagnostics: state.doc_diagnostics.clone(),
            ..CompileResult::error(vec![])
        }
    }

    /// Identifies the current session; changes on every `reset_session`.
    /// Background work captures it and gives up if it no longer matches.
    pub fn session_id(&self) -> u64 {
        self.inner.read().session
    }

    /// Forget unsaved editor text for `file_path` and use `disk_text`
    /// instead (the user discarded their edits). Takes effect immediately
    /// for the next compile; does not compile by itself.
    pub fn discard_overlay(&self, file_path: &str, disk_text: &str) {
        let Ok(abs) = absolutize_editor_path(file_path) else {
            return;
        };
        let mut state = self.inner.write();
        if !state.overlays.contains_key(&abs) {
            return;
        }
        state.overlays.insert(abs.clone(), disk_text.to_string());
        state
            .overlay_hashes
            .insert(abs.clone(), hash_content(disk_text));
        // A world currently stored has the discarded text: update it too.
        if let Some(world) = state.world.as_mut() {
            if world.main_path() == abs.as_path() {
                world.update_source(disk_text);
            } else {
                let _ = world.update_source_by_path(abs.to_string_lossy().as_ref(), disk_text);
            }
        }
        state.doc_key = None;
    }

    /// Whether a file the current document read from disk has changed
    /// since it was compiled (an edit made outside the editor).
    pub fn dependencies_changed(&self) -> bool {
        let deps = match self.inner.read().doc_key.as_ref() {
            Some(key) => key.deps.clone(),
            None => return false,
        };
        deps.iter()
            .any(|(path, stamp)| FileStamp::of(path) != *stamp)
    }

    /// The entry the cached document was compiled from.
    pub fn document_entry(&self) -> Option<PathBuf> {
        self.inner.read().doc_key.as_ref().map(|k| k.entry.clone())
    }

    /// Revision of the cached document (0 before the first success).
    pub fn revision(&self) -> u64 {
        self.inner.read().revision
    }

    /// Render a specific page as SVG.
    pub fn render_page_svg(&self, page_index: usize) -> Option<String> {
        {
            let state = self.inner.read();
            if let Some(Some(svg)) = state.rendered_pages.get(page_index) {
                return Some(svg.clone());
            }
        }

        let mut state = self.inner.write();
        if page_index >= state.page_hashes.len() {
            return None;
        }
        let doc = state.document.as_ref()?;
        let page = doc.pages.get(page_index)?;
        let svg = typst_svg::svg(page);
        if let Some(slot) = state.rendered_pages.get_mut(page_index) {
            *slot = Some(svg.clone());
        }
        Some(svg)
    }

    /// Page sizes in pt for the current document.
    pub fn page_sizes(&self) -> Vec<(f64, f64)> {
        let state = self.inner.read();
        state
            .document
            .as_ref()
            .map(|doc| {
                doc.pages
                    .iter()
                    .map(|p| (p.frame.width().to_pt(), p.frame.height().to_pt()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Rasterize a page to premultiplied RGBA8 at `pixel_per_pt`.
    /// Returns `(width, height, rgba)`.
    pub fn render_page_rgba(
        &self,
        page_index: usize,
        pixel_per_pt: f32,
    ) -> Option<(u32, u32, Vec<u8>)> {
        let doc = self.inner.read().document.clone()?;
        let page = doc.pages.get(page_index)?;
        let pixmap = typst_render::render(page, pixel_per_pt);
        Some((pixmap.width(), pixmap.height(), pixmap.take()))
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
        // Never export a document compiled from a different entry than the
        // one the preview now shows.
        let doc_entry = state.doc_key.as_ref().map(|k| &k.entry);
        if doc_entry.is_none() || doc_entry != state.preview_entry.as_ref() {
            return Err(
                "The compiled document does not match the current entry; recompile first.".into(),
            );
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

    /// The entry the user pinned explicitly, if any.
    pub fn pinned_entry(&self) -> Option<PathBuf> {
        self.inner.read().pinned_preview_entry.clone()
    }

    /// The entry a compile of `file_path` would use, without falling back to
    /// the file itself: the pinned entry, or the entry that imported it in a
    /// previous compile. `None` for a file no compiled document depends on.
    pub fn entry_for(&self, file_path: &str) -> Option<PathBuf> {
        let abs = absolutize_editor_path(file_path).ok()?;
        let state = self.inner.read();
        state
            .pinned_preview_entry
            .clone()
            .or_else(|| state.route_table.get(&abs).cloned())
    }

    /// Get the page count of the current document.
    pub fn page_count(&self) -> usize {
        let state = self.inner.read();
        state.document.as_ref().map(|d| d.pages.len()).unwrap_or(0)
    }

    /// Like [`Self::with_document`], but runs even if the last compile failed.
    pub fn with_world<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&TypstWorld, Option<&PagedDocument>) -> R,
    {
        let state = self.inner.read();
        let world = state.world.as_ref()?;
        Some(f(world, state.document.as_deref()))
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
    pub outline: Vec<OutlineItem>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// Errors and warnings with source locations, for editor squiggles.
    pub diagnostics: Vec<DiagnosticInfo>,
    pub duration_ms: u64,
    pub entry_path: Option<String>,
    pub pinned_entry_path: Option<String>,
    /// Revision of the document this result describes (0 when none).
    pub revision: u64,
}

impl CompileResult {
    pub fn error(errors: Vec<String>) -> Self {
        Self {
            success: false,
            page_count: 0,
            page_hashes: vec![],
            outline: vec![],
            errors,
            warnings: vec![],
            diagnostics: vec![],
            duration_ms: 0,
            entry_path: None,
            pinned_entry_path: None,
            revision: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticInfo {
    /// Absolute path of the file the diagnostic points into.
    pub file: String,
    /// Byte range within that file.
    pub start: usize,
    pub end: usize,
    pub message: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OutlineItem {
    pub title: String,
    pub level: usize,
    pub file: Option<String>,
    pub offset: Option<usize>,
    pub position: Option<OutlinePosition>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OutlinePosition {
    pub page: usize,
    pub x: f64,
    pub y: f64,
}

fn resolve_entry_for_file(state: &InnerState, file_path: &Path) -> PathBuf {
    if let Some(entry) = state.pinned_preview_entry.as_ref() {
        return entry.clone();
    }

    if let Some(entry) = state.route_table.get(file_path) {
        return entry.clone();
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

fn resolve_access_root_for_path(path: &Path) -> PathBuf {
    discover_access_root(path)
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

pub(crate) fn hash_page(page: &typst::layout::Page) -> u64 {
    let mut hasher = DefaultHasher::new();
    page.hash(&mut hasher);
    hasher.finish()
}

/// Hash content for incremental compilation detection
/// Order-independent fingerprint of all overlay contents.
fn overlay_fingerprint(hashes: &HashMap<PathBuf, u64>) -> u64 {
    let mut entries: Vec<_> = hashes.iter().collect();
    entries.sort();
    let mut hasher = DefaultHasher::new();
    entries.hash(&mut hasher);
    hasher.finish()
}

fn hash_content(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
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

fn diagnostic_info(
    diag: &typst::diag::SourceDiagnostic,
    world: &TypstWorld,
) -> Option<DiagnosticInfo> {
    let id = diag.span.id()?;
    let source = world.source(id).ok()?;
    let range = source.range(diag.span)?;
    Some(DiagnosticInfo {
        file: world.resolve_absolute_path(id)?.display().to_string(),
        start: range.start,
        end: range.end,
        message: diag.message.to_string(),
        is_error: diag.severity == typst::diag::Severity::Error,
    })
}
