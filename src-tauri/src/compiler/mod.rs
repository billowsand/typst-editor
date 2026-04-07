pub(crate) mod world;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
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
    pub world: Option<TypstWorld>,
    pub document: Option<Arc<PagedDocument>>,
    pub rendered_pages: Vec<String>,
    pub page_hashes: Vec<u64>,
    /// Whether the *last* compilation succeeded.
    pub last_compile_ok: bool,
    /// Cached project root to avoid repeated filesystem walks.
    pub cached_root: Option<PathBuf>,
}

impl CompilerState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(InnerState {
                world: None,
                document: None,
                rendered_pages: Vec::new(),
                page_hashes: Vec::new(),
                last_compile_ok: false,
                cached_root: None,
            })),
        }
    }

    /// Open a .typ file (path on disk) and initialize the world.
    pub fn open(&self, file_path: &str, content: &str) -> Result<(), String> {
        let abs = PathBuf::from(file_path);
        let mut state = self.inner.write();

        // Fast path: reuse cached root only when BOTH paths are absolute and the
        // file genuinely lives inside the cached root.  An empty or relative cached
        // root (left over from opening the virtual "untitled.typ" at startup) must
        // never be used for real absolute paths because Path::starts_with("") is
        // vacuously true for every path, which would set the wrong root and break
        // relative #import / #include resolution.
        let root = state.cached_root.clone().and_then(|cached| {
            if cached.is_absolute() && abs.is_absolute() && abs.starts_with(&cached) {
                Some(cached)
            } else {
                None
            }
        });

        let root = root.or_else(|| {
            // Walk upward to find typst.toml (project root marker).
            discover_project_root(&abs).or_else(|| {
                if abs.is_absolute() {
                    // Use the file's parent directory as root.
                    abs.parent().map(|p| p.to_path_buf())
                } else {
                    // Virtual / untitled file – use the process working directory so
                    // that the world is still functional without breaking anything.
                    std::env::current_dir().ok()
                }
            })
        })
        .unwrap_or_else(|| PathBuf::from("."));

        // Only persist absolute roots; a relative fallback must not poison the cache.
        if root.is_absolute() {
            state.cached_root = Some(root.clone());
        }

        let world = TypstWorld::new_with_root(file_path, content, &root)
            .map_err(|e| format!("Failed to create TypstWorld: {}", e))?;

        state.world = Some(world);
        state.document = None;
        state.rendered_pages.clear();
        state.page_hashes.clear();
        state.last_compile_ok = false;
        Ok(())
    }

    /// Update the source content and compile.
    pub fn update_and_compile(&self, content: &str) -> CompileResult {
        let main_path = {
            let state = self.inner.read();
            state
                .world
                .as_ref()
                .map(|world| world.main().vpath().as_rootless_path().display().to_string())
        };

        match main_path {
            Some(path) => self.update_file_and_compile(&path, content),
            None => CompileResult {
                success: false,
                page_count: 0,
                page_hashes: vec![],
                errors: vec!["No file is open".into()],
                warnings: vec![],
                duration_ms: 0,
            },
        }
    }

    /// Update any file content and recompile the current main document.
    pub fn update_file_and_compile(&self, file_path: &str, content: &str) -> CompileResult {
        let mut state = self.inner.write();
        let Some(world) = &mut state.world else {
            return CompileResult {
                success: false,
                page_count: 0,
                page_hashes: vec![],
                errors: vec!["No file is open".into()],
                warnings: vec![],
                duration_ms: 0,
            };
        };

        if let Err(err) = world.update_source_by_path(file_path, content) {
            return CompileResult {
                success: false,
                page_count: 0,
                page_hashes: vec![],
                errors: vec![err],
                warnings: vec![],
                duration_ms: 0,
            };
        }
        let start = std::time::Instant::now();
        let result = typst::compile::<PagedDocument>(world);
        let duration = start.elapsed();

        let mut compile_result = CompileResult {
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
                .push(format_diagnostic(warning, world));
        }

        match result.output {
            Ok(doc) => {
                let rendered_pages: Vec<String> = doc.pages.iter().map(typst_svg::svg).collect();
                let page_hashes = rendered_pages
                    .iter()
                    .map(|svg| hash_page(svg))
                    .collect::<Vec<_>>();

                compile_result.success = true;
                compile_result.page_count = doc.pages.len();
                compile_result.page_hashes = page_hashes.clone();
                state.document = Some(Arc::new(doc));
                state.rendered_pages = rendered_pages;
                state.page_hashes = page_hashes;
                state.last_compile_ok = true;
            }
            Err(errors) => {
                for error in errors {
                    compile_result.errors.push(format_diagnostic(&error, world));
                }
                // Keep the old document so preview isn't blank,
                // but mark that compilation failed.
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
    /// Returns `Err` if there is no document at all, or if the last compile
    /// failed (stale document).
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

    /// Get the project root directory path.
    pub fn project_root(&self) -> Option<PathBuf> {
        let state = self.inner.read();
        state.world.as_ref().map(|w| w.root().to_path_buf())
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
