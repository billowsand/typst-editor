use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use typst::diag::FileError;
use typst::foundations::{Bytes, Datetime};
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::Library;
use typst::LibraryExt;
use typst::World;

use typst_kit::fonts::{FontSearcher, Fonts};
use typst_kit::package::{default_package_cache_path, default_package_path};

/// Global font cache – searched once, reused forever.
static FONTS: OnceLock<Fonts> = OnceLock::new();

/// Initialize (if needed) and return the global font set.
/// Called by both the compiler and the `list_system_fonts` command.
pub fn get_or_init_fonts() -> &'static Fonts {
    FONTS.get_or_init(|| FontSearcher::new().search())
}

/// Implements `typst::World` for the editor.
///
/// Resolves files relative to an access root directory. When a `typst.toml`
/// is present, that project root is used. Otherwise the access root falls back
/// to the filesystem root for the opened file so standalone documents can still
/// import shared files from parent directories.
pub struct TypstWorld {
    /// Typst access root used for file sandboxing.
    root: PathBuf,
    /// Absolute path to the main source file.
    main_path: PathBuf,
    /// The main source file.
    main_source: Source,
    /// In-memory overlays for edited non-main text sources.
    overlays: HashMap<FileId, Source>,
    /// Absolute-path overlays keyed by file path for convenience.
    overlay_paths: HashMap<PathBuf, FileId>,
    /// Accessed files during compilation, used to build a route table.
    touched_paths: Mutex<HashSet<PathBuf>>,
    /// Fonts (reference to the global cache).
    fonts: &'static Fonts,
    book: LazyHash<FontBook>,
    library: LazyHash<Library>,
}

impl TypstWorld {
    /// Create a new world.
    ///
    /// `file_path` – absolute or relative path to the main `.typ` file.
    /// `content`   – the text content of that file.
    pub fn new(file_path: &str, content: &str) -> Result<Self, String> {
        let abs = PathBuf::from(file_path);
        let root = discover_access_root(&abs)
            .unwrap_or_else(|| PathBuf::from("."));

        Self::new_with_root_impl(file_path, content, root)
    }

    /// Create a new world with a pre-discovered root (avoids filesystem walk).
    pub fn new_with_root(file_path: &str, content: &str, root: &Path) -> Result<Self, String> {
        Self::new_with_root_impl(file_path, content, root.to_path_buf())
    }

    fn new_with_root_impl(file_path: &str, content: &str, root: PathBuf) -> Result<Self, String> {
        let abs = absolutize_with_root(Path::new(file_path), &root);

        let rel = VirtualPath::within_root(&abs, &root).ok_or_else(|| {
            format!(
                "Main file is outside current project root: {}",
                abs.display()
            )
        })?;
        let id = FileId::new(None, rel);
        let main_source = Source::new(id, content.to_string());

        let fonts = FONTS.get_or_init(|| FontSearcher::new().search());
        let book = LazyHash::new(fonts.book.clone());
        let library = LazyHash::new(Library::builder().build());

        Ok(Self {
            root,
            main_path: abs.clone(),
            main_source,
            overlays: HashMap::new(),
            overlay_paths: HashMap::new(),
            touched_paths: Mutex::new(HashSet::from([abs])),
            fonts,
            book,
            library,
        })
    }

    /// Replace the main source text in-place (enables incremental parsing).
    pub fn update_source(&mut self, content: &str) {
        self.main_source.replace(content);
    }

    /// Replace the text of any source file in-place.
    pub fn update_source_by_path(&mut self, file_path: &str, content: &str) -> Result<(), String> {
        let abs = absolutize_with_root(Path::new(file_path), &self.root);
        let id = self
            .file_id_for_path(&abs)
            .ok_or_else(|| format!("File is outside current project root: {}", file_path))?;

        if id == self.main_source.id() {
            self.main_source.replace(content);
        } else {
            self.overlays
                .insert(id, Source::new(id, content.to_string()));
            self.overlay_paths.insert(abs.clone(), id);
        }
        self.touched_paths.lock().insert(abs);

        Ok(())
    }

    /// Get a reference to the main source.
    pub fn source_ref(&self) -> &Source {
        &self.main_source
    }

    /// Get the project root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn main_path(&self) -> &Path {
        &self.main_path
    }

    pub fn reset_touched_paths(&mut self) {
        let mut touched = self.touched_paths.lock();
        touched.clear();
        touched.insert(self.main_path.clone());
    }

    pub fn source_by_path(&self, file_path: &str) -> Option<Source> {
        let abs = absolutize_with_root(Path::new(file_path), &self.root);
        let id = self.file_id_for_path(&abs)?;
        self.source(id).ok()
    }

    pub fn resolve_absolute_path(&self, id: FileId) -> Option<PathBuf> {
        if id == self.main_source.id() {
            return Some(self.main_path.clone());
        }
        if let Some(path) = self
            .overlay_paths
            .iter()
            .find_map(|(path, overlay_id)| (*overlay_id == id).then(|| path.clone()))
        {
            return Some(path);
        }
        self.resolve_path(id).ok()
    }

    pub fn depended_paths(&self) -> Vec<PathBuf> {
        let mut deps = self
            .touched_paths
            .lock()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        deps.sort();
        deps
    }

    fn file_id_for_path(&self, abs: &Path) -> Option<FileId> {
        let rel = VirtualPath::within_root(abs, &self.root)?;

        Some(FileId::new(None, rel))
    }

    /// Resolve a `FileId` to an absolute path on disk.
    fn resolve_path(&self, id: FileId) -> Result<PathBuf, FileError> {
        let root = if let Some(spec) = id.package() {
            resolve_package_root(spec)
                .ok_or_else(|| FileError::NotFound(PathBuf::from("typst.toml")))?
        } else {
            self.root.clone()
        };

        id.vpath()
            .resolve(&root)
            .ok_or_else(|| FileError::AccessDenied)
    }

    /// Read a source file from disk and parse it.
    fn read_source(&self, id: FileId) -> Result<Source, FileError> {
        let path = self.resolve_path(id)?;
        self.touched_paths.lock().insert(path.clone());
        let text = std::fs::read_to_string(&path).map_err(|_| FileError::NotFound(path.clone()))?;
        Ok(Source::new(id, text))
    }

    /// Read a binary file from disk.
    fn read_file(&self, id: FileId) -> Result<Bytes, FileError> {
        let path = self.resolve_path(id)?;
        self.touched_paths.lock().insert(path.clone());
        let data = std::fs::read(&path).map_err(|_| FileError::NotFound(path.clone()))?;
        Ok(Bytes::new(data))
    }
}

pub(crate) fn discover_project_root(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() { path } else { path.parent()? };
    for dir in start.ancestors() {
        if dir.join("typst.toml").is_file() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

pub(crate) fn discover_access_root(path: &Path) -> Option<PathBuf> {
    discover_project_root(path).or_else(|| discover_filesystem_root(path))
}

fn discover_filesystem_root(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };

    start
        .ancestors()
        .last()
        .map(Path::to_path_buf)
        .filter(|root| !root.as_os_str().is_empty())
}

fn resolve_package_root(spec: &typst::syntax::package::PackageSpec) -> Option<PathBuf> {
    let rel = PathBuf::from(spec.namespace.as_str())
        .join(spec.name.as_str())
        .join(spec.version.to_string());

    default_package_path()
        .into_iter()
        .chain(default_package_cache_path())
        .map(|base| base.join(&rel))
        .find(|dir| dir.exists())
}

impl typst::World for TypstWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main_source.id()
    }

    fn source(&self, id: FileId) -> typst::diag::FileResult<Source> {
        // Fast path: the main file.
        if id == self.main_source.id() {
            return Ok(self.main_source.clone());
        }
        if let Some(source) = self.overlays.get(&id) {
            return Ok(source.clone());
        }
        self.read_source(id)
    }

    fn file(&self, id: FileId) -> typst::diag::FileResult<Bytes> {
        // The main source can also be requested as a raw file.
        if id == self.main_source.id() {
            return Ok(Bytes::from_string(self.main_source.text().to_string()));
        }
        if let Some(source) = self.overlays.get(&id) {
            return Ok(Bytes::from_string(source.text().to_string()));
        }
        self.read_file(id)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.fonts.get(index).and_then(|slot| slot.get())
    }

    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        use chrono::Datelike;
        let now = chrono::Local::now();
        Some(Datetime::from_ymd(
            now.year(),
            now.month() as u8,
            now.day() as u8,
        )?)
    }
}

impl typst_ide::IdeWorld for TypstWorld {
    fn upcast(&self) -> &dyn typst::World {
        self
    }
}

fn absolutize_with_root(path: &Path, root: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}
