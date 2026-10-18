//! Root view: the editing session and everything that keeps documents safe.
//!
//! - One `EditorState` per open document, so switching tabs keeps each
//!   document's undo history, cursor, selection and scroll position.
//! - Saves are atomic and refuse to silently overwrite files that changed
//!   on disk; outside edits are noticed and offered back to the user.
//! - Unsaved text is journaled shortly after each edit and offered for
//!   recovery after a crash.
//!
//! Views live in `views.rs`, dialogs and panels in `dialogs.rs`.

mod dialogs;
mod preferences;
mod views;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_kit::component::input::{EditorState, InputEvent, InputState, RopeExt, TabSize};
use gpui_kit::component::tree::{TreeItem, TreeState};
use gpui_kit::*;
use typst_core::compiler::CompileOptions;
use typst_core::{CompileResult, CompilerState, OutlineItem};

use crate::files::{self, DiskVersion};
use crate::highlight;
use crate::logging;
use crate::lsp::TypstLsp;
use crate::preview::{PreviewEvent, PreviewPanel};
use crate::recovery::{Draft, Journal};
use crate::settings::{Session, Settings};
use crate::workspace::{Workspace, normalize};

actions!(
    typst_editor,
    [
        NewDocument,
        NewProject,
        OpenFile,
        OpenFolder,
        Save,
        SaveAs,
        SaveAll,
        CloseTab,
        Find,
        FindReplace,
        ProjectSearch,
        ShowProblems,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        FitWidth,
        Recompile,
        ExportPdf,
        ToggleSidebar,
        ToggleOutline,
        ToggleCompact,
        FontBigger,
        FontSmaller,
        FontReset,
        OpenSettings,
        RevealProject,
        Rescan,
        OpenDemo,
        UndoEdit,
        RedoEdit,
        CopyDiagnostics,
        OpenLogFolder,
        OpenGuide,
        About
    ]
);

/// Open a document by path (title and tab dropdowns, problems, search).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
pub(crate) struct OpenDocument {
    pub path: String,
}

/// Open a recent project.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
pub(crate) struct OpenRecent {
    pub root: String,
}

/// File tree context menu actions.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
pub(crate) struct TreeNewFile {
    pub dir: String,
}
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
pub(crate) struct TreeNewFolder {
    pub dir: String,
}
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
pub(crate) struct TreeRename {
    pub path: String,
}
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
pub(crate) struct TreeDelete {
    pub path: String,
}
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
pub(crate) struct TreeReveal {
    pub path: String,
}

/// Toggle the star of the open document (sheet header).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
pub(crate) struct ToggleStarActive {
    pub path: String,
}

/// Debounce between the last keystroke and a compile.
const COMPILE_DEBOUNCE: Duration = Duration::from_millis(120);
/// Debounce between cursor movement and the preview marker update.
const CURSOR_DEBOUNCE: Duration = Duration::from_millis(60);
/// Debounce between the last keystroke and writing the recovery journal.
const JOURNAL_DEBOUNCE: Duration = Duration::from_millis(1200);
/// How often open files are checked for outside edits.
const DISK_POLL: Duration = Duration::from_millis(1500);
/// Directories never scanned.
const IGNORED_DIRS: &[&str] = &["target", "node_modules", ".git"];
/// Extensions opened in the editor; everything else is revealed on disk.
pub(crate) const TEXT_EXTS: &[&str] = &[
    "typ", "bib", "yml", "yaml", "toml", "txt", "csv", "json", "xml", "md", "svg", "tex",
];
const DEFAULT_FONT: f32 = 15.;
/// Project search stops collecting after this many matches.
const SEARCH_LIMIT: usize = 2000;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewDocument, None),
        KeyBinding::new("secondary-o", OpenFile, None),
        KeyBinding::new("secondary-shift-o", OpenFolder, None),
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-shift-s", SaveAs, None),
        KeyBinding::new("secondary-alt-s", SaveAll, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        KeyBinding::new("secondary-h", FindReplace, None),
        KeyBinding::new("secondary-shift-f", ProjectSearch, None),
        KeyBinding::new("secondary-shift-m", ShowProblems, None),
        KeyBinding::new("secondary-=", ZoomIn, None),
        KeyBinding::new("secondary--", ZoomOut, None),
        KeyBinding::new("secondary-0", ZoomReset, None),
        KeyBinding::new("secondary-9", FitWidth, None),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-shift-e", ExportPdf, None),
        KeyBinding::new("f5", Recompile, None),
    ]);
}

/// A `.typ` file shown as a note card.
#[derive(Clone)]
pub(crate) struct DocCard {
    pub path: String,
    /// Path relative to the project root, with `/` separators.
    pub rel: String,
    /// First heading or comment of the file, else its name.
    pub title: String,
    pub modified: Option<SystemTime>,
}

/// Plain (Send) mirror of a file tree entry, built off the UI thread.
#[derive(Clone)]
pub(crate) struct TreeNode {
    pub path: String,
    pub name: String,
    pub children: Option<Vec<TreeNode>>,
}

impl TreeNode {
    /// Convert to tree items; with a query, keep only matches and the
    /// folders leading to them (expanded).
    pub fn to_item(&self, depth: usize, query: &str) -> Option<TreeItem> {
        let matches = query.is_empty() || self.name.to_lowercase().contains(query);
        match &self.children {
            Some(children) => {
                let kids: Vec<_> = children
                    .iter()
                    .filter_map(|c| c.to_item(depth + 1, if matches { "" } else { query }))
                    .collect();
                if !matches && kids.is_empty() {
                    return None;
                }
                Some(
                    TreeItem::new(self.path.clone(), self.name.clone())
                        .expanded(depth == 0 || !query.is_empty())
                        .children(kids),
                )
            }
            None => matches.then(|| TreeItem::new(self.path.clone(), self.name.clone())),
        }
    }

    fn files(&self, out: &mut Vec<String>) {
        match &self.children {
            Some(children) => children.iter().for_each(|c| c.files(out)),
            None => out.push(self.path.clone()),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Nav {
    /// `.typ` files as note cards.
    Drafts,
    /// The whole project as a directory tree.
    Files,
}

/// Something waiting on the user's decision.
#[derive(Clone)]
pub(crate) enum Pending {
    /// Switch to another project, then optionally open a file in it.
    Switch {
        root: PathBuf,
        open: Option<PathBuf>,
    },
    CloseTab(String),
    CloseWindow,
    /// Saving these would overwrite edits made outside the editor.
    Conflict {
        paths: Vec<String>,
        after: Option<Box<Pending>>,
    },
    /// Unsaved drafts left behind by a crash.
    Recover(Vec<Draft>),
    Rename {
        path: String,
    },
    NewItem {
        dir: PathBuf,
        folder: bool,
    },
    Delete {
        path: String,
    },
}

/// A non-modal panel over the desk.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Panel {
    Problems,
    Search,
}

pub(crate) enum CompileStatus {
    Idle,
    Ok { ms: u64 },
    Failed { first: String, count: usize },
}

/// One diagnostic with a resolved line and column.
#[derive(Clone)]
pub(crate) struct Problem {
    pub file: String,
    pub offset: usize,
    pub line: usize,
    pub col: usize,
    pub message: String,
    pub is_error: bool,
}

/// One project search match.
#[derive(Clone)]
pub(crate) struct SearchHit {
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    /// The matched line, trimmed, and the match's byte range within it.
    pub snippet: String,
    pub snippet_match: std::ops::Range<usize>,
}

/// The editor of one open document and its subscriptions.
pub(crate) struct DocEditor {
    pub state: Entity<EditorState>,
    _subscriptions: Vec<Subscription>,
}

pub struct TypstEditorApp {
    pub(crate) compiler: Arc<CompilerState>,
    pub(crate) editors: HashMap<String, DocEditor>,
    pub(crate) preview: Entity<PreviewPanel>,
    pub(crate) search: Entity<InputState>,
    pub(crate) name_input: Entity<InputState>,
    pub(crate) find_input: Entity<InputState>,
    pub(crate) replace_input: Entity<InputState>,
    pub(crate) tree_state: Entity<TreeState>,
    pub(crate) query: String,
    pub(crate) ws: Workspace,
    pub(crate) cards: Vec<DocCard>,
    pub(crate) tree_nodes: Vec<TreeNode>,
    pub(crate) resources: usize,
    pub(crate) starred: HashSet<String>,
    pub(crate) char_count: usize,
    pub(crate) last_cursor: usize,
    pub(crate) compiling: bool,
    pub(crate) status: CompileStatus,
    pub(crate) outline: Vec<OutlineItem>,
    pub(crate) problems: Vec<Problem>,
    pub(crate) nav: Nav,
    pub(crate) show_sidebar: bool,
    pub(crate) outline_open: bool,
    pub(crate) show_about: bool,
    preferences: preferences::Preferences,
    pub(crate) show_settings: bool,
    pub(crate) notice: Option<String>,
    pub(crate) pending: Option<Pending>,
    pub(crate) panel: Option<Panel>,
    pub(crate) dialog_focus: FocusHandle,
    pub(crate) app_focus: FocusHandle,
    pub(crate) tabs_scroll: ScrollHandle,
    /// Compact code mode: line numbers, tighter lines, no ruling.
    pub(crate) compact: bool,
    pub(crate) font_px: f32,
    /// Files whose last save failed, with the reason; cleared on success.
    pub(crate) save_errors: BTreeMap<String, String>,
    /// Open documents changed or removed on disk while they had unsaved
    /// edits; the user decides what to keep.
    pub(crate) outside: BTreeMap<String, Option<DiskVersion>>,
    pub(crate) search_hits: Vec<SearchHit>,
    pub(crate) search_running: bool,
    /// The project is being scanned (no cards yet).
    pub(crate) scanning: bool,
    settings: Settings,
    journal: Journal,
    /// Documents with a draft in the journal.
    journaled: HashSet<String>,
    compile_task: Task<()>,
    cursor_task: Task<()>,
    notice_task: Task<()>,
    journal_task: Task<()>,
    search_task: Task<()>,
    _watch_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl TypstEditorApp {
    pub fn new(settings: Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::theme::apply_fonts(&settings.ui_font, &settings.editor_font, cx);
        let preferences = Self::make_preferences(&settings, window, cx);
        let app_focus = cx.focus_handle();
        app_focus.focus(window, cx);
        let compiler = Arc::new(CompilerState::new());
        let preview = cx.new(|_| PreviewPanel::new(compiler.clone()));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索文稿、文件与标题"));
        let name_input = cx.new(|cx| InputState::new(window, cx));
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("在项目中查找"));
        let replace_input = cx.new(|cx| InputState::new(window, cx).placeholder("替换为"));
        let tree_state = cx.new(|cx| TreeState::new(cx));

        let _subscriptions = vec![
            cx.subscribe(&search, |this, search, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = search.read(cx).value().trim().to_lowercase();
                    this.refresh_tree(cx);
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &name_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.confirm_name(window, cx);
                    }
                },
            ),
            cx.subscribe(&find_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.run_project_search(cx);
                }
            }),
            cx.subscribe_in(
                &preview,
                window,
                |this, _, event: &PreviewEvent, window, cx| match event {
                    PreviewEvent::JumpToSource { file, offset } => {
                        this.jump_to(file.clone(), *offset, window, cx)
                    }
                    PreviewEvent::OpenUrl(url) => {
                        // Links in the user's own document; web and mail only.
                        if ["https://", "http://", "mailto:"]
                            .iter()
                            .any(|p| url.starts_with(p))
                        {
                            cx.open_url(url);
                        }
                    }
                },
            ),
        ];

        // Closing the window with unsaved edits asks first.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.on_close_requested(window, cx))
                .unwrap_or(true)
        });

        // Notice edits made to open files by other programs.
        let _watch_task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(DISK_POLL).await;
                if this
                    .update_in(cx, |this, window, cx| this.check_disk(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });

        let nav = if settings.nav == "files" {
            Nav::Files
        } else {
            Nav::Drafts
        };
        Self {
            compiler,
            editors: HashMap::new(),
            preview,
            search,
            name_input,
            find_input,
            replace_input,
            tree_state,
            query: String::new(),
            ws: Workspace::default(),
            cards: Vec::new(),
            tree_nodes: Vec::new(),
            resources: 0,
            starred: settings.starred.iter().cloned().collect(),
            char_count: 0,
            last_cursor: 0,
            compiling: false,
            status: CompileStatus::Idle,
            outline: Vec::new(),
            problems: Vec::new(),
            nav,
            show_sidebar: settings.show_sidebar,
            outline_open: settings.outline_open,
            show_about: false,
            preferences,
            show_settings: false,
            notice: None,
            pending: None,
            panel: None,
            dialog_focus: cx.focus_handle(),
            app_focus,
            tabs_scroll: ScrollHandle::new(),
            compact: settings.compact,
            font_px: settings.font_px,
            save_errors: BTreeMap::new(),
            outside: BTreeMap::new(),
            search_hits: Vec::new(),
            search_running: false,
            scanning: false,
            settings,
            journal: Journal::default_location(),
            journaled: HashSet::new(),
            compile_task: Task::ready(()),
            cursor_task: Task::ready(()),
            notice_task: Task::ready(()),
            journal_task: Task::ready(()),
            search_task: Task::ready(()),
            _watch_task,
            _subscriptions,
        }
    }

    // ── Accessors ───────────────────────────────────────────

    pub(crate) fn active(&self) -> Option<String> {
        self.ws.active().map(str::to_owned)
    }

    pub(crate) fn is_active(&self, path: &str) -> bool {
        self.ws.is_active(path)
    }

    pub(crate) fn active_editor(&self) -> Option<&Entity<EditorState>> {
        self.ws
            .active()
            .and_then(|p| self.editors.get(p))
            .map(|e| &e.state)
    }

    /// Live text of an open document (its editor), else its buffer.
    fn text_of(&self, path: &str, cx: &App) -> Option<String> {
        let key = normalize(path);
        self.editors
            .get(&key)
            .map(|e| e.state.read(cx).text().to_string())
            .or_else(|| self.ws.buffer(&key).map(|b| b.current.clone()))
    }

    fn editor_buffers(&self, cx: &App) -> Vec<(String, String)> {
        self.ws
            .tabs()
            .filter_map(|(path, _)| self.text_of(path, cx).map(|text| (path.to_string(), text)))
            .collect()
    }

    pub(crate) fn line_height(&self) -> f32 {
        (self.font_px * if self.compact { 1.55 } else { 2.0 }).round()
    }

    pub(crate) fn title_text(&self) -> String {
        self.ws
            .root()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "字笺".into())
    }

    pub(crate) fn recent_projects(&self) -> Vec<String> {
        self.settings.recent_projects.clone()
    }

    // ── Settings and session ────────────────────────────────

    /// Write preferences and the current session to disk.
    pub(crate) fn persist(&mut self) {
        self.settings.font_px = self.font_px;
        self.settings.compact = self.compact;
        self.settings.outline_open = self.outline_open;
        self.settings.show_sidebar = self.show_sidebar;
        self.settings.nav = match self.nav {
            Nav::Drafts => "drafts".into(),
            Nav::Files => "files".into(),
        };
        let mut starred: Vec<String> = self.starred.iter().cloned().collect();
        starred.sort();
        self.settings.starred = starred;
        self.settings.last_session = self.ws.root().map(|root| Session {
            root: root.display().to_string(),
            tabs: self.ws.tabs().map(|(p, _)| p.to_owned()).collect(),
            active: self.active(),
        });
        if let Err(err) = self.settings.save_to(&Settings::path()) {
            logging::warn(&format!("saving settings failed: {err}"));
        }
    }

    /// Reopen the project and tabs from the last run.
    pub fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.settings.last_session.clone() else {
            return;
        };
        let root = PathBuf::from(&session.root);
        if !root.is_dir() {
            return;
        }
        self.switch_project(root, None, window, cx);
        for tab in &session.tabs {
            if Path::new(tab).is_file() {
                self.open_document(tab.clone(), window, cx);
            }
        }
        if let Some(active) = session.active.filter(|a| Path::new(a).is_file()) {
            self.open_document(active, window, cx);
        }
    }

    /// Offer drafts left behind by a crash, if any differ from disk.
    pub fn offer_recovery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let drafts: Vec<Draft> = self
            .journal
            .drafts()
            .into_iter()
            .filter(Journal::differs_from_disk)
            .collect();
        if drafts.is_empty() {
            return;
        }
        logging::info(&format!("{} recovery drafts found", drafts.len()));
        self.pending = Some(Pending::Recover(drafts));
        self.dialog_focus.focus(window, cx);
        cx.notify();
    }

    fn restore_drafts(&mut self, drafts: Vec<Draft>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(first) = drafts.first() else { return };
        let first_path = PathBuf::from(&first.path);
        let root = typst_core::discover_project_root(&first_path)
            .or_else(|| first_path.parent().map(Path::to_path_buf));
        if let Some(root) = root.filter(|r| !self.ws.is_root(r)) {
            self.switch_project(root, None, window, cx);
        }
        let mut restored = 0;
        let mut missing = 0;
        for draft in drafts {
            let path = normalize(&draft.path);
            if !Path::new(&path).exists() {
                missing += 1;
            }
            if self.ws.restore_draft(&path, draft.text.clone()).is_err() {
                continue;
            }
            self.char_count = count_chars(&draft.text);
            self.last_cursor = 0;
            let editor = self.make_editor(&path, draft.text, window, cx);
            editor.state.update(cx, |state, cx| state.focus(window, cx));
            self.editors.insert(path.clone(), editor);
            self.open_document(draft.path.clone(), window, cx);
            self.journaled.insert(path);
            restored += 1;
        }
        self.schedule_journal(cx);
        self.schedule_compile(Duration::ZERO, self.compiler.preview_entry().is_none(), cx);
        self.persist();
        self.show_notice(
            if missing > 0 {
                format!("已恢复 {restored} 份草稿；其中 {missing} 份原文件已丢失，请使用“另存为”选择位置")
            } else {
                format!("已恢复 {restored} 份未保存的草稿，请检查后保存")
            },
            cx,
        );
    }

    // ── Projects and documents ──────────────────────────────

    /// Open a `.typ` file or a project folder (command line, dialogs, demo).
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let (root, file) = if path.is_dir() {
            let main = find_main_file(&path);
            (path, main)
        } else {
            let root = typst_core::discover_project_root(&path)
                .or_else(|| path.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| path.clone());
            (root, Some(path))
        };

        // A file inside the open project, or the open project again: no
        // session reset, just show the file.
        let same_project = self.ws.is_root(&root)
            || file
                .as_ref()
                .is_some_and(|f| self.ws.root().is_some() && self.ws.contains(f));
        if same_project {
            if let Some(file) = file {
                self.open_document(file.display().to_string(), window, cx);
            }
            return;
        }
        self.request(Pending::Switch { root, open: file }, window, cx);
    }

    /// Start a fresh session for `root`. Unsaved edits were resolved by
    /// the caller; everything tied to the old project is dropped, and work
    /// still in flight for it is ignored via the workspace generation.
    fn switch_project(
        &mut self,
        root: PathBuf,
        open: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        logging::info(&format!("open project {}", root.display()));
        for path in self.journaled.drain() {
            self.journal.remove(&path);
        }
        self.ws.reset(Some(root.clone()));
        self.editors.clear();
        self.compiler.reset_session();
        self.compile_task = Task::ready(());
        self.cursor_task = Task::ready(());
        self.journal_task = Task::ready(());
        self.compiling = false;
        self.status = CompileStatus::Idle;
        self.outline.clear();
        self.problems.clear();
        self.cards.clear();
        self.tree_nodes.clear();
        self.resources = 0;
        self.char_count = 0;
        self.save_errors.clear();
        self.outside.clear();
        self.search_hits.clear();
        self.preview.update(cx, |preview, cx| {
            preview.set_document(vec![], vec![], cx);
            preview.set_markers(vec![], cx);
        });
        self.settings.note_recent(&root.display().to_string());
        self.rescan(cx);
        if let Some(file) = open {
            self.open_document(file.display().to_string(), window, cx);
        }
        self.persist();
        cx.notify();
    }

    /// Re-read the project for the cards and the file tree.
    pub(crate) fn rescan(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.ws.root().map(Path::to_path_buf) else {
            return;
        };
        let generation = self.ws.generation();
        self.scanning = self.cards.is_empty();
        cx.spawn(async move |this, cx| {
            let (cards, nodes, resources) = cx
                .background_spawn(async move { scan_project(&root) })
                .await;
            this.update(cx, |this, cx| {
                // A scan for a project that has since been closed.
                if this.ws.generation() != generation {
                    return;
                }
                this.cards = cards;
                this.tree_nodes = nodes;
                this.resources = resources;
                this.scanning = false;
                this.refresh_tree(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        let items: Vec<TreeItem> = self
            .tree_nodes
            .iter()
            .filter_map(|n| n.to_item(0, &self.query))
            .collect();
        self.tree_state
            .update(cx, |state, cx| state.set_items(items, cx));
    }

    /// The editor for `path`, with highlighting, completion and the
    /// subscriptions that keep the session in sync with it.
    fn make_editor(
        &mut self,
        path: &str,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> DocEditor {
        let typst = is_typst(path);
        let compact = self.compact;
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(if typst { highlight::LANGUAGE } else { "text" })
                .line_number(compact)
                .soft_wrap(true)
                .replaceable(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .placeholder("在稿纸上写点什么……")
                .default_value(text)
        });
        let compiler = self.compiler.clone();
        let doc_path = path.to_string();
        state.update(cx, |state, cx| {
            let lsp = Rc::new(TypstLsp {
                compiler,
                active_path: Rc::new(std::cell::RefCell::new(Some(doc_path))),
            });
            state.lsp_mut().completion_provider = Some(lsp.clone());
            state.lsp_mut().hover_provider = Some(lsp);
            state.set_highlighter_factory(highlight::typst_highlighter_factory(), cx);
            cx.notify();
        });

        let change_path = path.to_string();
        let cursor_path = path.to_string();
        let subscriptions = vec![
            cx.subscribe(&state, move |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.on_text_changed(&change_path, cx);
                }
            }),
            // Cursor moves only show up as notifications from the editor.
            // (The editor notifies on every caret blink; only a real move
            // needs the status bar and outline to re-render.)
            cx.observe(&state, move |this, editor, cx| {
                if this.is_active(&cursor_path) {
                    let cursor = editor.read(cx).cursor();
                    if cursor != this.last_cursor {
                        this.last_cursor = cursor;
                        this.schedule_cursor_sync(cx);
                        cx.notify();
                    }
                }
            }),
        ];
        DocEditor {
            state,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn open_document(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = normalize(&path);
        if self.is_active(&path) && self.editors.contains_key(&path) {
            return;
        }
        if let Err(err) = self.ws.open(&path, |p| files::read_text(Path::new(p))) {
            logging::warn(&err);
            self.show_notice(err, cx);
            return;
        }
        if !self.editors.contains_key(&path) {
            let text = self
                .ws
                .buffer(&path)
                .map(|b| b.current.clone())
                .unwrap_or_default();
            let editor = self.make_editor(&path, text, window, cx);
            self.editors.insert(path.clone(), editor);
        }
        let editor = self.editors[&path].state.clone();
        let text = editor.read(cx).text().to_string();
        self.char_count = count_chars(&text);
        self.last_cursor = editor.read(cx).cursor();
        editor.update(cx, |state, cx| state.focus(window, cx));
        if let Some(ix) = self.ws.tab_index(&path) {
            self.tabs_scroll.scroll_to_item(ix);
        }

        if is_typst(&path) {
            // The first document opened becomes the preview entry; later
            // ones route to whichever entry imports them.
            let make_entry = self.compiler.preview_entry().is_none();
            self.schedule_compile(Duration::ZERO, make_entry, cx);
        } else if self.compiler.entry_for(&path).is_some() {
            // A bibliography or data file a document reads: recompile it.
            self.schedule_compile(Duration::ZERO, false, cx);
        }
        self.persist();
        cx.notify();
    }

    pub(crate) fn jump_to(
        &mut self,
        file: String,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_active(&file) {
            self.open_document(file, window, cx);
        }
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |state, cx| {
                state.set_selected_range(offset..offset, cx);
                state.focus(window, cx);
            });
        }
    }

    fn select_range(
        &mut self,
        file: String,
        range: std::ops::Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to(file, range.start, window, cx);
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |state, cx| state.set_selected_range(range, cx));
        }
    }

    /// Click on a file tree row.
    pub(crate) fn open_from_tree(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if is_text_file(&path) {
            self.open_document(path, window, cx);
        } else {
            reveal_in_file_manager(Path::new(&path));
            self.show_notice("已在资源管理器中显示此文件", cx);
        }
    }

    /// Pin `path` as the preview entry, or unpin it when it already is.
    /// Runs off the UI thread: it waits for the compile lock.
    pub(crate) fn toggle_pin(&mut self, path: String, cx: &mut Context<Self>) {
        let path = normalize(&path);
        let unpin = self
            .compiler
            .pinned_entry()
            .is_some_and(|p| normalize(&p.display().to_string()) == path);
        let text = match self.text_of(&path, cx) {
            Some(text) => text,
            None => match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) => {
                    self.show_notice(format!("无法读取该文稿：{err}"), cx);
                    return;
                }
            },
        };
        let compiler = self.compiler.clone();
        let generation = self.ws.generation();
        let session = compiler.session_id();
        self.compiling = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let _guard = compiler.compile_lock();
                    if compiler.session_id() != session {
                        return Err("项目已切换".to_string());
                    }
                    if unpin {
                        compiler.pin_preview_entry(None)
                    } else {
                        compiler
                            .open(&path, &text)
                            .and_then(|_| compiler.pin_preview_entry(Some(&path)))
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                if this.ws.generation() != generation {
                    return;
                }
                match result {
                    Ok(()) => {
                        this.show_notice(
                            if unpin {
                                "已取消固定，预览将自动选择入口"
                            } else {
                                "已固定为预览主文件"
                            },
                            cx,
                        );
                        this.schedule_compile(Duration::ZERO, false, cx);
                    }
                    Err(err) => {
                        this.compiling = false;
                        this.show_notice(format!("无法设为主文件：{err}"), cx);
                    }
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn prompt_open(&mut self, directories: bool, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: !directories,
            directories,
            multiple: false,
            prompt: Some(
                if directories {
                    "打开文件夹"
                } else {
                    "打开 Typst 文件"
                }
                .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| {
            let path = paths.await.ok()?.ok()??.into_iter().next()?;
            this.update_in(cx, |this, window, cx| this.open_path(path, window, cx))
                .ok()
        })
        .detach();
    }

    /// A starting point for an empty document.
    fn template_for(name: &str) -> String {
        if name.ends_with(".typ") {
            let title = name.trim_end_matches(".typ");
            format!("#set page(paper: \"a4\")\n#set text(lang: \"zh\")\n\n= {title}\n\n")
        } else {
            String::new()
        }
    }

    /// New document (Ctrl+N): pick a name in the project, create, open.
    fn new_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self
            .ws
            .root()
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let picker = cx.prompt_for_new_path(&dir, Some("新文稿.typ"));
        cx.spawn_in(window, async move |this, cx| {
            let Some(mut path) = picker.await.ok().and_then(Result::ok).flatten() else {
                return;
            };
            if path.extension().is_none() {
                path.set_extension("typ");
            }
            this.update_in(cx, |this, window, cx| {
                this.create_and_open(path, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// New project: a folder with a `main.typ`.
    fn new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self
            .ws
            .root()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let picker = cx.prompt_for_new_path(&base, Some("新项目"));
        cx.spawn_in(window, async move |this, cx| {
            let Some(dir) = picker.await.ok().and_then(Result::ok).flatten() else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                let main = dir.join("main.typ");
                let made = std::fs::create_dir_all(&dir).and_then(|_| {
                    if main.exists() {
                        Ok(())
                    } else {
                        files::atomic_write(&main, Self::template_for("main.typ").as_bytes())
                            .map(|_| ())
                    }
                });
                match made {
                    Ok(()) => this.open_path(dir, window, cx),
                    Err(err) => {
                        this.show_notice(format!("无法创建项目：{}", files::describe(&err)), cx)
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn create_and_open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if path.exists() {
            self.open_path(path, window, cx);
            return;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let result = path
            .parent()
            .map(std::fs::create_dir_all)
            .unwrap_or(Ok(()))
            .and_then(|_| files::atomic_write(&path, Self::template_for(&name).as_bytes()));
        match result {
            Ok(_) => {
                logging::info(&format!("created {}", path.display()));
                self.rescan(cx);
                self.open_path(path, window, cx);
            }
            Err(err) => self.show_notice(format!("无法创建文件：{}", files::describe(&err)), cx),
        }
    }

    /// Save As (Ctrl+Shift+S): write the active document to a new path and
    /// continue editing it there; the original file is left as it was.
    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active() else { return };
        let current = PathBuf::from(&active);
        let dir = current
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = current
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "副本.typ".into());
        let picker = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Some(target) = picker.await.ok().and_then(Result::ok).flatten() else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                this.save_as_to(active, target, window, cx)
            })
            .ok();
        })
        .detach();
    }

    fn save_as_to(
        &mut self,
        from: String,
        to: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.text_of(&from, cx) else {
            return;
        };
        let target = normalize(&to.display().to_string());
        if target == from {
            self.save_paths(&[from], false, cx);
            return;
        }
        if let Err(err) = files::atomic_write(&to, text.as_bytes()) {
            self.save_errors
                .insert(target.clone(), files::describe(&err));
            self.show_notice(format!("另存为失败：{}", files::describe(&err)), cx);
            return;
        }
        logging::info(&format!("saved as {target}"));
        // The original keeps its on-disk content: drop its unsaved buffer.
        let was_dirty = self.ws.buffer(&from).is_some_and(|b| b.is_dirty());
        self.ws.close(&from);
        self.editors.remove(&from);
        self.forget_draft(&from);
        if was_dirty {
            self.revert_overlay(from, cx);
        }
        self.rescan(cx);
        self.open_path(to, window, cx);
    }

    // ── Unsaved edits and decisions ─────────────────────────

    /// Run `action`, asking first if it would discard unsaved edits.
    pub(crate) fn request(&mut self, action: Pending, window: &mut Window, cx: &mut Context<Self>) {
        if self.affected_dirty(&action).is_empty() {
            self.perform(action, false, window, cx);
        } else {
            self.pending = Some(action);
            self.dialog_focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn affected_dirty(&self, action: &Pending) -> Vec<String> {
        match action {
            Pending::CloseTab(path) => self
                .ws
                .buffer(path)
                .filter(|b| b.is_dirty())
                .map(|_| vec![normalize(path)])
                .unwrap_or_default(),
            Pending::Switch { .. } | Pending::CloseWindow => self.ws.dirty_paths(),
            _ => vec![],
        }
    }

    fn perform(
        &mut self,
        action: Pending,
        discarded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            Pending::Switch { root, open } => self.switch_project(root, open, window, cx),
            Pending::CloseTab(path) => self.close_tab_now(path, discarded, window, cx),
            Pending::CloseWindow => {
                self.persist();
                for path in self.journaled.drain() {
                    self.journal.remove(&path);
                }
                logging::info("exit");
                window.remove_window();
            }
            // Decisions with their own dialogs never reach here.
            _ => {}
        }
    }

    /// The unsaved-edits dialog's answer: save first, discard, or cancel.
    pub(crate) fn resolve_pending(
        &mut self,
        save: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.pending.take() else {
            return;
        };
        match save {
            None => {}
            Some(true) => {
                let paths = self.affected_dirty(&action);
                let outcome = self.save_paths(&paths, false, cx);
                match outcome.continuation(action) {
                    Some(action @ Pending::Conflict { .. }) => {
                        self.pending = Some(action);
                        self.dialog_focus.focus(window, cx);
                    }
                    Some(action) => self.perform(action, false, window, cx),
                    None => {}
                }
                // On failure the edits stay; the banner says what failed.
            }
            Some(false) => self.perform(action, true, window, cx),
        }
        cx.notify();
    }

    /// The save-conflict dialog's answer.
    pub(crate) fn resolve_conflict(
        &mut self,
        choice: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Pending::Conflict { paths, after }) = self.pending.take() else {
            return;
        };
        match choice {
            None => {}
            // Overwrite the outside edits with ours.
            Some(true) => {
                let outcome = self.save_paths(&paths, true, cx);
                if outcome.failed.is_empty() {
                    if let Some(after) = after {
                        self.request(*after, window, cx);
                    }
                }
            }
            // Throw ours away and take what is on disk.
            Some(false) => {
                for path in &paths {
                    self.reload_from_disk(path, window, cx);
                }
                if let Some(after) = after {
                    self.request(*after, window, cx);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn resolve_recovery(
        &mut self,
        restore: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Pending::Recover(drafts)) = self.pending.take() else {
            return;
        };
        match restore {
            Some(true) => self.restore_drafts(drafts, window, cx),
            Some(false) => {
                for draft in &drafts {
                    self.journal.remove(&draft.path);
                }
            }
            // Later: leave them in the journal.
            None => {}
        }
        cx.notify();
    }

    fn on_close_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.ws.dirty_paths().is_empty() {
            self.persist();
            for path in self.journaled.drain() {
                self.journal.remove(&path);
            }
            logging::info("exit");
            return true;
        }
        self.pending = Some(Pending::CloseWindow);
        self.dialog_focus.focus(window, cx);
        cx.notify();
        false
    }

    pub(crate) fn close_tab_now(
        &mut self,
        path: String,
        discarded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = normalize(&path);
        let next = self.ws.close(&path);
        self.editors.remove(&path);
        self.forget_draft(&path);
        self.save_errors.remove(&path);
        self.outside.remove(&path);
        if discarded {
            // The compiler still holds the discarded text; put it back to
            // what is on disk so the preview stops showing it.
            self.revert_overlay(path.clone(), cx);
        }
        if self.active().is_none() {
            match next {
                Some(next) => self.open_document(next, window, cx),
                None => {
                    self.char_count = 0;
                    self.cursor_task = Task::ready(());
                    self.preview
                        .update(cx, |preview, cx| preview.set_markers(vec![], cx));
                }
            }
        }
        // Even a clean closed tab must stop overriding its disk file.
        if self.active().is_some() {
            self.schedule_compile(Duration::ZERO, false, cx);
        }
        self.persist();
        cx.notify();
    }

    /// The user discarded the edits of `path`: give the compiler the disk
    /// text right away (a queued compile could otherwise be cancelled and
    /// leave the discarded text in use), then refresh the preview.
    fn revert_overlay(&mut self, path: String, cx: &mut Context<Self>) {
        let Ok(disk) = std::fs::read_to_string(&path) else {
            return;
        };
        self.compiler.discard_overlay(&path, &disk);
        if self.active().is_some() {
            self.schedule_compile(Duration::ZERO, false, cx);
        } else if is_typst(&path) || self.compiler.entry_for(&path).is_some() {
            self.run_compile(
                path,
                Some(disk),
                Duration::ZERO,
                CompileOptions::default(),
                false,
                cx,
            );
        }
    }

    // ── Editing and saving ──────────────────────────────────

    fn on_text_changed(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(text) = self.text_of(path, cx) else {
            return;
        };
        let count = count_chars(&text);
        if !self.ws.set_current(path, text) {
            return;
        }
        if self.is_active(path) {
            self.char_count = count;
        }
        if is_typst(path) || self.compiler.entry_for(path).is_some() {
            self.schedule_compile(COMPILE_DEBOUNCE, false, cx);
        }
        self.schedule_journal(cx);
        cx.notify();
    }

    /// Write the drafts of unsaved documents a moment after editing stops.
    fn schedule_journal(&mut self, cx: &mut Context<Self>) {
        self.journal_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JOURNAL_DEBOUNCE).await;
            this.update(cx, |this, cx| this.flush_journal(cx)).ok();
        });
    }

    fn flush_journal(&mut self, cx: &mut Context<Self>) {
        let mut writes = Vec::new();
        for path in self.ws.dirty_paths() {
            if let Some(text) = self.text_of(&path, cx) {
                self.journaled.insert(path.clone());
                writes.push(self.journal.prepare_write(&path, &text));
            }
        }
        let clean: Vec<String> = self
            .journaled
            .iter()
            .filter(|p| !self.ws.buffer(p).is_some_and(|b| b.is_dirty()))
            .cloned()
            .collect();
        for path in &clean {
            self.journaled.remove(path);
            self.journal.remove(path);
        }
        cx.background_spawn(async move {
            for write in writes {
                if let Err(err) = write.commit() {
                    logging::warn(&format!("journal write failed: {err}"));
                }
            }
        })
        .detach();
    }

    fn forget_draft(&mut self, path: &str) {
        if self.journaled.remove(path) {
            self.journal.remove(path);
        }
    }

    /// Save `paths`. Files changed on disk since we read or wrote them are
    /// not written unless `force`; they come back as conflicts.
    pub(crate) fn save_paths(
        &mut self,
        paths: &[String],
        force: bool,
        cx: &mut Context<Self>,
    ) -> SaveOutcome {
        let mut outcome = SaveOutcome::default();
        for path in paths {
            let path = normalize(path);
            let Some(text) = self.text_of(&path, cx) else {
                continue;
            };
            let known = self.ws.buffer(&path).and_then(|b| b.disk);
            let now = DiskVersion::of(Path::new(&path));
            if !force && now != known && now.is_some() {
                outcome.conflicts.push(path);
                continue;
            }
            match files::atomic_write(Path::new(&path), text.as_bytes()) {
                Ok(version) => {
                    logging::info(&format!("saved {path}"));
                    self.ws.set_current(&path, text.clone());
                    self.ws.mark_saved(&path, &text, version);
                    self.save_errors.remove(&path);
                    self.outside.remove(&path);
                    self.forget_draft(&path);
                }
                Err(err) => {
                    let reason = files::describe(&err);
                    logging::error(&format!("save failed for {path}: {err}"));
                    self.save_errors.insert(path.clone(), reason);
                    outcome.failed.push(path);
                }
            }
        }
        if !outcome.failed.is_empty() {
            self.show_notice(
                format!(
                    "{} 个文件保存失败，修改仍保留在编辑器中",
                    outcome.failed.len()
                ),
                cx,
            );
        }
        self.rescan(cx);
        cx.notify();
        outcome
    }

    fn save_with_conflict_check(
        &mut self,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let outcome = self.save_paths(&paths, false, cx);
        if !outcome.conflicts.is_empty() {
            self.pending = Some(Pending::Conflict {
                paths: outcome.conflicts,
                after: None,
            });
            self.dialog_focus.focus(window, cx);
            cx.notify();
        }
    }

    /// Replace a document with what is on disk, keeping undo history.
    pub(crate) fn reload_from_disk(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = normalize(path);
        match files::read_text(Path::new(&path)) {
            Ok((text, version)) => {
                self.ws.reload(&path, text.clone(), version);
                if let Some(editor) = self.editors.get(&path) {
                    editor.state.update(cx, |state, cx| {
                        let cursor = state.cursor().min(text.len());
                        state.replace_all(text.clone(), window, cx);
                        let cursor = (0..=cursor)
                            .rev()
                            .find(|i| text.is_char_boundary(*i))
                            .unwrap_or(0);
                        state.set_selected_range(cursor..cursor, cx);
                    });
                }
                self.outside.remove(&path);
                self.forget_draft(&path);
                if self.is_active(&path) {
                    self.char_count = count_chars(
                        &self
                            .ws
                            .buffer(&path)
                            .map(|b| b.current.clone())
                            .unwrap_or_default(),
                    );
                }
                if is_typst(&path) || self.compiler.entry_for(&path).is_some() {
                    self.schedule_compile(Duration::ZERO, false, cx);
                }
            }
            Err(err) => self.show_notice(format!("无法重新载入：{}", files::describe(&err)), cx),
        }
    }

    /// Keep the editor's text after an outside edit; the next save
    /// overwrites the file deliberately.
    pub(crate) fn keep_mine(&mut self, path: &str, cx: &mut Context<Self>) {
        let path = normalize(path);
        let now = self.outside.remove(&path).flatten();
        self.ws.keep_mine(&path, now);
        cx.notify();
    }

    /// Poll for outside edits to open documents and their dependencies.
    fn check_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (path, now) in self.ws.changed_on_disk() {
            if self.outside.get(&path) == Some(&now) {
                continue; // already reported
            }
            let dirty = self.ws.buffer(&path).is_some_and(|b| b.is_dirty());
            if now.is_some() && !dirty {
                logging::info(&format!("reloaded outside edit of {path}"));
                self.reload_from_disk(&path, window, cx);
                self.show_notice(format!("已载入外部修改：{}", file_name(&path)), cx);
            } else {
                logging::warn(&format!(
                    "outside change to unsaved {path} (deleted: {})",
                    now.is_none()
                ));
                self.outside.insert(path, now);
                cx.notify();
            }
        }
        // A file the document reads but nobody has open changed on disk.
        if !self.compiling && self.compiler.dependencies_changed() {
            self.schedule_compile(Duration::ZERO, false, cx);
        }
    }

    pub(crate) fn show_notice(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = Some(text.into());
        self.notice_task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(3500))
                .await;
            this.update(cx, |this, cx| {
                this.notice = None;
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    // ── Compile ─────────────────────────────────────────────

    /// Compile the active document after `delay`.
    pub(crate) fn schedule_compile(
        &mut self,
        delay: Duration,
        make_entry: bool,
        cx: &mut Context<Self>,
    ) {
        self.compile_active(delay, CompileOptions::default(), make_entry, cx);
    }

    fn compile_active(
        &mut self,
        delay: Duration,
        options: CompileOptions,
        make_entry: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.active() else { return };
        // The text is read when the debounce ends, not on every keystroke.
        let text = if delay.is_zero() {
            self.text_of(&path, cx)
        } else {
            None
        };
        self.run_compile(path, text, delay, options, make_entry, cx);
    }

    /// Compile `path` with `text`. Replacing the task cancels a compile that
    /// has not started; results for an older session are dropped.
    fn run_compile(
        &mut self,
        path: String,
        text: Option<String>,
        delay: Duration,
        options: CompileOptions,
        make_entry: bool,
        cx: &mut Context<Self>,
    ) {
        let compiler = self.compiler.clone();
        let generation = self.ws.generation();
        let session = compiler.session_id();
        self.compiling = true;

        self.compile_task = cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let text = match text {
                Some(text) => text,
                None => match this.update(cx, |this, cx| this.text_of(&path, cx)) {
                    Ok(Some(text)) => text,
                    // The document was closed while waiting.
                    _ => return Self::compile_abandoned(&this, generation, cx),
                },
            };
            // Snapshot every live buffer, including inactive unsaved imports.
            let buffers = match this.update(cx, |this, cx| this.editor_buffers(cx)) {
                Ok(buffers) => buffers,
                Err(_) => return,
            };
            let outcome = cx
                .background_spawn(async move {
                    let _guard = compiler.compile_lock();
                    // Queued behind a compile of a project that is now closed.
                    if compiler.session_id() != session {
                        return None;
                    }
                    if let Err(err) = compiler.set_editor_overlays(&buffers) {
                        return Some((CompileResult::error(vec![err]), vec![]));
                    }
                    if make_entry {
                        if let Err(err) = compiler.open(&path, &text) {
                            return Some((CompileResult::error(vec![err]), vec![]));
                        }
                    }
                    let mut result = compiler.compile_file(&path, &text, options);
                    typst_core::ide::populate_outline(&compiler, &mut result);
                    let sizes = compiler.page_sizes();
                    Some((result, sizes))
                })
                .await;
            let Some((result, sizes)) = outcome else {
                return Self::compile_abandoned(&this, generation, cx);
            };
            this.update(cx, |this, cx| {
                if this.ws.generation() == generation {
                    this.apply_compile_result(result, sizes, cx);
                }
            })
            .ok();
        });
    }

    /// A queued compile gave up: clear the indicator it set.
    fn compile_abandoned(this: &WeakEntity<Self>, generation: u64, cx: &mut AsyncApp) {
        this.update(cx, |this, cx| {
            if this.ws.generation() == generation {
                this.compiling = false;
                cx.notify();
            }
        })
        .ok();
    }

    fn apply_compile_result(
        &mut self,
        result: CompileResult,
        sizes: Vec<(f64, f64)>,
        cx: &mut Context<Self>,
    ) {
        self.compiling = false;
        if result.success {
            self.outline = result.outline.clone();
            self.status = CompileStatus::Ok {
                ms: result.duration_ms,
            };
            let hashes = result.page_hashes.clone();
            self.preview
                .update(cx, |preview, cx| preview.set_document(sizes, hashes, cx));
            self.schedule_cursor_sync(cx);
        } else {
            self.status = CompileStatus::Failed {
                first: result.errors.first().cloned().unwrap_or_default(),
                count: result.errors.len(),
            };
            if result.page_count == 0 && self.compiler.page_count() == 0 {
                self.preview
                    .update(cx, |preview, cx| preview.set_document(vec![], vec![], cx));
            }
        }
        self.problems = self.resolve_problems(&result, cx);
        self.show_diagnostics(&result, cx);
        cx.notify();
    }

    /// Diagnostics with line and column, for the problems panel.
    fn resolve_problems(&self, result: &CompileResult, cx: &App) -> Vec<Problem> {
        let mut texts: HashMap<String, Option<String>> = HashMap::new();
        let mut problems: Vec<Problem> = result
            .diagnostics
            .iter()
            .map(|d| {
                let file = normalize(&d.file);
                let text = texts
                    .entry(file.clone())
                    .or_insert_with(|| {
                        self.text_of(&file, cx)
                            .or_else(|| std::fs::read_to_string(&file).ok())
                    })
                    .as_deref()
                    .unwrap_or("");
                let (line, col) = line_col(text, d.start);
                Problem {
                    file,
                    offset: d.start,
                    line,
                    col,
                    message: d.message.clone(),
                    is_error: d.is_error,
                }
            })
            .collect();
        problems.sort_by(|a, b| {
            b.is_error
                .cmp(&a.is_error)
                .then_with(|| a.file.cmp(&b.file))
                .then_with(|| a.offset.cmp(&b.offset))
        });
        problems
            .dedup_by(|a, b| a.file == b.file && a.offset == b.offset && a.message == b.message);
        problems
    }

    /// Squiggles in every open editor (each shows its own file's).
    fn show_diagnostics(&mut self, result: &CompileResult, cx: &mut Context<Self>) {
        for (path, editor) in &self.editors {
            let mine: Vec<_> = result
                .diagnostics
                .iter()
                .filter(|d| normalize(&d.file) == *path)
                .cloned()
                .collect();
            editor.state.update(cx, |state, cx| {
                let rope = state.text().clone();
                let diagnostics: Vec<_> = mine
                    .iter()
                    .map(|d| {
                        let start = rope.offset_to_position(d.start.min(rope.len()));
                        let end = rope.offset_to_position(d.end.min(rope.len()));
                        let severity = if d.is_error {
                            DiagnosticSeverity::Error
                        } else {
                            DiagnosticSeverity::Warning
                        };
                        Diagnostic::new(start..end, d.message.clone()).with_severity(severity)
                    })
                    .collect();
                if let Some(set) = state.diagnostics_mut() {
                    set.clear();
                    set.extend(diagnostics);
                }
                cx.notify();
            });
        }
    }

    // ── Cursor -> preview ───────────────────────────────────

    fn schedule_cursor_sync(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.active() else { return };
        let Some(text) = self.text_of(&path, cx) else {
            return;
        };
        let offset = self.last_cursor;
        let compiler = self.compiler.clone();
        self.cursor_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CURSOR_DEBOUNCE).await;
            let markers = cx
                .background_spawn(async move {
                    typst_core::ide::jump_from_offset_live(&compiler, &path, &text, offset)
                })
                .await;
            this.update(cx, |this, cx| {
                this.preview
                    .update(cx, |preview, cx| preview.set_markers(markers, cx));
            })
            .ok();
        });
    }

    // ── View settings ───────────────────────────────────────

    pub(crate) fn zoom_by(&mut self, factor: Option<f32>, cx: &mut Context<Self>) {
        self.preview.update(cx, |preview, cx| {
            let zoom = factor.map_or(1.0, |f| preview.zoom() * f);
            preview.set_zoom(zoom, cx);
        });
        cx.notify();
    }

    fn set_compact(&mut self, compact: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.compact = compact;
        for editor in self.editors.values() {
            editor
                .state
                .update(cx, |state, cx| state.set_line_number(compact, window, cx));
        }
        self.persist();
        cx.notify();
    }

    pub(crate) fn toggle_star(&mut self, path: String, cx: &mut Context<Self>) {
        let path = normalize(&path);
        if !self.starred.remove(&path) {
            self.starred.insert(path);
        }
        self.persist();
        cx.notify();
    }

    fn open_search(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |state, cx| {
                state.focus(window, cx);
                state.open_search(replace, cx);
            });
        }
    }

    // ── Export ──────────────────────────────────────────────

    /// Export PDF: compile the current text under the compile lock, then
    /// export that very document, so the file always matches the entry.
    fn export_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.compiler.preview_entry() else {
            self.show_notice("还没有可导出的文档", cx);
            return;
        };
        let active = self
            .active()
            .and_then(|p| self.text_of(&p, cx).map(|t| (p, t)));
        let path = PathBuf::from(entry);
        let name = path
            .with_extension("pdf")
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document.pdf".into());
        let picker = cx.prompt_for_new_path(path.parent().unwrap_or(Path::new(".")), Some(&name));
        let compiler = self.compiler.clone();
        let buffers = self.editor_buffers(cx);
        let session = compiler.session_id();
        cx.spawn_in(window, async move |this, cx| {
            let Some(mut target) = picker.await.ok().and_then(Result::ok).flatten() else {
                return;
            };
            if target.extension().is_none() {
                target.set_extension("pdf");
            }
            let shown = target.display().to_string();
            let result = cx
                .background_spawn(async move {
                    let _guard = compiler.compile_lock();
                    if compiler.session_id() != session {
                        return Err("项目已切换，请重新导出".to_string());
                    }
                    compiler.set_editor_overlays(&buffers)?;
                    if let Some((path, text)) = active {
                        let result = compiler.compile_file(&path, &text, CompileOptions::default());
                        if !result.success {
                            return Err(format!(
                                "文档有错误，无法导出：{}",
                                result.errors.first().cloned().unwrap_or_default()
                            ));
                        }
                    }
                    let bytes = compiler.export_pdf()?;
                    files::atomic_write(&target, &bytes)
                        .map(|_| ())
                        .map_err(|e| files::describe(&e))
                })
                .await;
            this.update(cx, |this, cx| {
                let text = match result {
                    Ok(()) => {
                        logging::info(&format!("exported {shown}"));
                        format!("PDF 已导出：{shown}")
                    }
                    Err(err) => {
                        logging::warn(&format!("export failed: {err}"));
                        format!("导出失败：{err}")
                    }
                };
                this.show_notice(text, cx);
            })
            .ok();
        })
        .detach();
    }

    fn open_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Portable builds carry examples beside the executable; fall back to
        // the source tree during development.
        let examples = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("examples")))
            .filter(|dir| dir.join("main.typ").is_file())
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"));
        self.open_path(examples, window, cx);
    }

    // ── Project search and replace ──────────────────────────

    /// Search every text file in the project, using unsaved editor text
    /// for open documents.
    pub(crate) fn run_project_search(&mut self, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value().to_string();
        if query.is_empty() {
            self.search_hits.clear();
            cx.notify();
            return;
        }
        let mut paths = Vec::new();
        for node in &self.tree_nodes {
            node.files(&mut paths);
        }
        let sources: Vec<(String, Option<String>)> = paths
            .into_iter()
            .filter(|p| is_text_file(p))
            .map(|p| {
                let text = self.text_of(&p, cx);
                (p, text)
            })
            .collect();
        self.search_running = true;
        self.search_task = cx.spawn(async move |this, cx| {
            let hits = cx
                .background_spawn(async move { search_sources(&sources, &query) })
                .await;
            this.update(cx, |this, cx| {
                this.search_hits = hits;
                this.search_running = false;
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// Replace every match in the project. Edits go into the editors (with
    /// undo), never straight to disk, so the user reviews and saves them.
    pub(crate) fn replace_in_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.find_input.read(cx).value().to_string();
        let replacement = self.replace_input.read(cx).value().to_string();
        if query.is_empty() {
            return;
        }
        let mut files_hit: Vec<String> = self.search_hits.iter().map(|h| h.path.clone()).collect();
        files_hit.dedup();
        let active = self.active();
        let mut count = 0;
        let mut changed = 0;
        for path in &files_hit {
            self.open_document(path.clone(), window, cx);
            let Some(editor) = self.editors.get(&normalize(path)).map(|e| e.state.clone()) else {
                continue;
            };
            let text = editor.read(cx).text().to_string();
            let n = text.matches(&query).count();
            if n == 0 {
                continue;
            }
            let new_text = text.replace(&query, &replacement);
            editor.update(cx, |state, cx| state.replace_all(new_text, window, cx));
            count += n;
            changed += 1;
        }
        if let Some(active) = active {
            self.open_document(active, window, cx);
        }
        self.show_notice(
            format!("已在 {changed} 个文件中替换 {count} 处（尚未保存，可撤销）"),
            cx,
        );
        self.run_project_search(cx);
    }

    // ── File management (tree context menu) ────────────────

    fn begin_name_dialog(
        &mut self,
        pending: Pending,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let initial = initial.to_string();
        self.name_input.update(cx, |input, cx| {
            input.set_value(initial, window, cx);
            input.focus(window, cx);
        });
        self.pending = Some(pending);
        cx.notify();
    }

    /// Enter in the name dialog, or its confirm button.
    pub(crate) fn confirm_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_input.read(cx).value().trim().to_string();
        let invalid = name.is_empty()
            || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
            || name == "."
            || name == "..";
        if invalid {
            self.show_notice("名称不能为空，也不能包含 \\ / : * ? \" < > |", cx);
            return;
        }
        match self.pending.take() {
            Some(Pending::Rename { path }) => self.rename_path(path, name, window, cx),
            Some(Pending::NewItem { dir, folder }) => {
                let target = dir.join(&name);
                if target.exists() {
                    self.show_notice("同名文件或文件夹已存在", cx);
                    self.pending = Some(Pending::NewItem { dir, folder });
                    return;
                }
                if folder {
                    match std::fs::create_dir_all(&target) {
                        Ok(()) => self.rescan(cx),
                        Err(err) => self
                            .show_notice(format!("无法创建文件夹：{}", files::describe(&err)), cx),
                    }
                } else {
                    self.create_and_open(target, window, cx);
                }
            }
            other => self.pending = other,
        }
        cx.notify();
    }

    fn rename_path(
        &mut self,
        path: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from = PathBuf::from(&path);
        let Some(to) = from.parent().map(|p| p.join(&name)) else {
            return;
        };
        if to.exists() {
            self.show_notice("同名文件或文件夹已存在", cx);
            return;
        }
        if let Err(err) = std::fs::rename(&from, &to) {
            self.show_notice(format!("重命名失败：{}", files::describe(&err)), cx);
            return;
        }
        logging::info(&format!("renamed {} -> {}", from.display(), to.display()));
        // Carry open documents (and their editors) to the new paths.
        let to_s = normalize(&to.display().to_string());
        for old in self.ws.paths_under(&path) {
            let new = normalize(&format!("{to_s}{}", &old[normalize(&path).len()..]));
            self.ws.rename(&old, &new);
            if let Some(editor) = self.editors.remove(&old) {
                // The editor's subscriptions name the old path: rebuild it.
                let text = editor.state.read(cx).text().to_string();
                let rebuilt = self.make_editor(&new, text, window, cx);
                self.editors.insert(new.clone(), rebuilt);
            }
            self.forget_draft(&old);
        }
        // Routes and overlays name old paths: start the compiler afresh.
        let remap = |p: PathBuf| {
            p.strip_prefix(&from)
                .map(|suffix| to.join(suffix))
                .unwrap_or(p)
        };
        let entry = self.compiler.preview_entry().map(remap);
        let pinned = self.compiler.pinned_entry().map(remap);
        self.compiler.reset_session();
        if let Some(pinned) = pinned {
            let _ = self
                .compiler
                .pin_preview_entry(Some(&pinned.display().to_string()));
        }
        if let Some(entry) = entry {
            let entry = normalize(&entry.display().to_string());
            let text = self
                .text_of(&entry, cx)
                .or_else(|| std::fs::read_to_string(&entry).ok());
            if let Some(text) = text {
                self.run_compile(
                    entry,
                    Some(text),
                    Duration::ZERO,
                    CompileOptions::default(),
                    true,
                    cx,
                );
            }
        } else {
            self.schedule_compile(Duration::ZERO, false, cx);
        }
        self.flush_journal(cx);
        self.rescan(cx);
        self.persist();
    }

    /// Move `path` to the recycle bin after confirmation; open documents
    /// inside it are closed.
    pub(crate) fn delete_path(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = None;
        if let Err(err) = trash::delete(&path) {
            logging::error(&format!("move to recycle bin failed for {path}: {err}"));
            self.show_notice(format!("无法移到回收站：{err}"), cx);
            return;
        }
        logging::info(&format!("moved to recycle bin: {path}"));
        for open in self.ws.paths_under(&path) {
            self.close_tab_now(open, false, window, cx);
        }
        self.rescan(cx);
        self.show_notice("已移到回收站，可从回收站还原", cx);
    }

    fn copy_diagnostics(&mut self, cx: &mut Context<Self>) {
        let fonts = crate::theme::fonts(cx);
        let text = logging::diagnostics(&fonts.serif, &fonts.mono);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_notice("诊断信息已复制（不含文稿内容）", cx);
    }
}

/// Result of saving several files.
#[derive(Default)]
pub(crate) struct SaveOutcome {
    pub failed: Vec<String>,
    pub conflicts: Vec<String>,
}

impl SaveOutcome {
    /// Never carry a destructive continuation across a failed save, even
    /// when other files also require a conflict decision.
    fn continuation(self, action: Pending) -> Option<Pending> {
        if !self.failed.is_empty() {
            None
        } else if !self.conflicts.is_empty() {
            Some(Pending::Conflict {
                paths: self.conflicts,
                after: Some(Box::new(action)),
            })
        } else {
            Some(action)
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────

pub(crate) fn count_chars(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

pub(crate) fn is_typst(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("typ"))
}

pub(crate) fn is_text_file(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    TEXT_EXTS.contains(&ext.as_str())
}

pub(crate) fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// 1-based line and column (in chars) of byte `offset` in `text`.
pub(crate) fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let offset = (0..=offset.min(text.len()))
        .rev()
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(0);
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

/// Literal, case-sensitive search over `(path, open text)` pairs; files
/// without open text are read from disk (non-UTF-8 and huge files skipped).
fn search_sources(sources: &[(String, Option<String>)], query: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for (path, open) in sources {
        let text = match open {
            Some(text) => text.clone(),
            None => match std::fs::metadata(path) {
                Ok(meta) if meta.len() <= 4 * 1024 * 1024 => match std::fs::read_to_string(path) {
                    Ok(text) => text,
                    Err(_) => continue,
                },
                _ => continue,
            },
        };
        for (start, m) in text.match_indices(query) {
            let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
            let line_end = text[start..].find('\n').map_or(text.len(), |i| start + i);
            let raw = &text[line_start..line_end];
            let lead = raw.len() - raw.trim_start().len();
            let snippet = raw.trim().to_string();
            let match_start = (start - line_start).saturating_sub(lead);
            hits.push(SearchHit {
                path: path.clone(),
                start,
                end: start + m.len(),
                line: text[..start].matches('\n').count() + 1,
                snippet_match: match_start..(match_start + m.len()).min(snippet.len()),
                snippet,
            });
            if hits.len() >= SEARCH_LIMIT {
                return hits;
            }
        }
    }
    hits
}

/// Smartisan-style relative stamp: "今天 08:12", "昨天 23:19",
/// "13天前 10/03 12:29", or a full date for older notes.
pub(crate) fn smartisan_time(time: SystemTime) -> String {
    use chrono::{DateTime, Local};
    let then: DateTime<Local> = time.into();
    let now = Local::now();
    let days = (now.date_naive() - then.date_naive()).num_days();
    match days {
        ..=0 => format!("今天 {}", then.format("%H:%M")),
        1 => format!("昨天 {}", then.format("%H:%M")),
        2..=30 => format!("{days}天前 {}", then.format("%m/%d %H:%M")),
        _ => then.format("%Y/%m/%d").to_string(),
    }
}

pub(crate) fn full_time(time: SystemTime) -> String {
    let then: chrono::DateTime<chrono::Local> = time.into();
    then.format("%Y/%m/%d %H:%M").to_string()
}

/// Show `path` in the platform file manager.
pub(crate) fn reveal_in_file_manager(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let arg = if path.is_dir() {
            format!("\"{}\"", path.display())
        } else {
            format!("/select,\"{}\"", path.display())
        };
        let _ = std::process::Command::new("explorer").raw_arg(arg).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

/// Scan `root`: `.typ` files as cards, the full tree, and how many other
/// files there are.
pub(crate) fn scan_project(root: &Path) -> (Vec<DocCard>, Vec<TreeNode>, usize) {
    fn walk(
        root: &Path,
        dir: &Path,
        depth: usize,
        cards: &mut Vec<DocCard>,
        other: &mut usize,
    ) -> Vec<TreeNode> {
        let Ok(entries) = typst_core::fs::list_dir(&dir.display().to_string()) else {
            return vec![];
        };
        let mut nodes = Vec::new();
        for e in entries {
            let path = Path::new(&e.path);
            if e.is_dir {
                if IGNORED_DIRS.contains(&e.name.as_str()) {
                    continue;
                }
                let children = if depth < 8 {
                    walk(root, path, depth + 1, cards, other)
                } else {
                    vec![]
                };
                nodes.push(TreeNode {
                    path: normalize(&e.path),
                    name: e.name,
                    children: Some(children),
                });
                continue;
            }
            if e.name.ends_with(".zj-tmp") {
                continue; // an atomic save in progress
            }
            if e.name.ends_with(".typ") {
                // Only the head is needed for the title.
                let text = read_head(path, 16 * 1024);
                cards.push(DocCard {
                    rel: path
                        .strip_prefix(root)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .replace('\\', "/"),
                    title: doc_title(&text)
                        .unwrap_or_else(|| e.name.trim_end_matches(".typ").into()),
                    modified: std::fs::metadata(path).and_then(|m| m.modified()).ok(),
                    path: normalize(&e.path),
                });
            } else {
                *other += 1;
            }
            nodes.push(TreeNode {
                path: normalize(&e.path),
                name: e.name,
                children: None,
            });
        }
        nodes
    }
    let mut cards = Vec::new();
    let mut other = 0;
    let nodes = walk(root, root, 0, &mut cards, &mut other);
    // Files at the top level first, then by path.
    cards.sort_by(|a, b| {
        a.rel
            .contains('/')
            .cmp(&b.rel.contains('/'))
            .then_with(|| a.rel.cmp(&b.rel))
    });
    (cards, nodes, other)
}

/// The first `limit` bytes of a file as text (cut at a char boundary).
fn read_head(path: &Path, limit: usize) -> String {
    use std::io::Read;
    let mut buf = Vec::with_capacity(limit);
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(limit as u64).read_to_end(&mut buf);
    }
    match String::from_utf8(buf) {
        Ok(text) => text,
        Err(err) => {
            let valid = err.utf8_error().valid_up_to();
            String::from_utf8_lossy(&err.into_bytes()[..valid]).into_owned()
        }
    }
}

/// A card title for a document: its first heading (`= Title`), else its
/// first comment (`// 基本情况`), else `None` so the caller uses the file
/// name. Code lines are never used; they make poor titles.
fn doc_title(text: &str) -> Option<String> {
    let clean = |s: &str| -> Option<String> {
        let t = s.trim_matches(|c: char| c.is_whitespace() || "=-*#/".contains(c));
        (!t.is_empty()).then(|| t.chars().take(40).collect())
    };
    let heading = text.lines().find_map(|line| {
        let line = line.trim_start();
        let rest = line.trim_start_matches('=');
        (rest.len() < line.len() && rest.starts_with(' '))
            .then(|| clean(rest))
            .flatten()
    });
    heading.or_else(|| {
        text.lines()
            .filter_map(|line| line.trim_start().strip_prefix("//"))
            .find_map(clean)
    })
}

/// Pick the entry file of a folder: `main.typ`, else the first `.typ` file.
fn find_main_file(dir: &Path) -> Option<PathBuf> {
    let main = dir.join("main.typ");
    if main.is_file() {
        return Some(main);
    }
    typst_core::fs::list_dir(&dir.display().to_string())
        .ok()?
        .into_iter()
        .find(|e| !e.is_dir && e.name.ends_with(".typ"))
        .map(|e| PathBuf::from(e.path))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use typst_core::CompilerState;

    #[test]
    fn failed_save_blocks_close_even_when_another_file_has_a_conflict() {
        let outcome = super::SaveOutcome {
            failed: vec!["readonly.typ".into()],
            conflicts: vec!["outside.typ".into()],
        };
        assert!(outcome.continuation(super::Pending::CloseWindow).is_none());
    }

    #[test]
    fn only_successful_saves_can_continue_or_ask_about_conflicts() {
        let outcome = super::SaveOutcome {
            failed: vec![],
            conflicts: vec!["outside.typ".into()],
        };
        assert!(matches!(
            outcome.continuation(super::Pending::CloseWindow),
            Some(super::Pending::Conflict { after: Some(_), .. })
        ));
        assert!(matches!(
            super::SaveOutcome::default().continuation(super::Pending::CloseWindow),
            Some(super::Pending::CloseWindow)
        ));
    }

    #[test]
    fn bundled_example_has_cross_file_outline_and_exports_pdf() {
        let main = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/main.typ");
        let path = main.display().to_string();
        let text = std::fs::read_to_string(&main).unwrap();
        let compiler = CompilerState::new();
        compiler.open(&path, &text).unwrap();
        let mut result = compiler.update_file_and_compile(&path, &text);
        assert!(result.success, "{:?}", result.errors);
        typst_core::ide::populate_outline(&compiler, &mut result);
        assert_eq!(result.outline.len(), 5);
        let included = result
            .outline
            .iter()
            .find(|i| i.title == "文字的力量")
            .unwrap();
        let file = PathBuf::from(included.file.as_ref().unwrap());
        assert!(file.ends_with(Path::new("chapters/design.typ")));
        let source = std::fs::read_to_string(file).unwrap();
        let offset = included.offset.unwrap();
        assert!(offset < source.len() && source.is_char_boundary(offset));
        assert!(compiler.export_pdf().unwrap().starts_with(b"%PDF-"));
    }

    #[test]
    fn titles_come_from_first_heading() {
        assert_eq!(
            super::doc_title("#set page()\n= 排版札记\n正文").as_deref(),
            Some("排版札记")
        );
        assert_eq!(
            super::doc_title("#let x = 1\n// ===== 基本情况 =====\n").as_deref(),
            Some("基本情况")
        );
        // Code lines never become titles; the caller falls back to the name.
        assert_eq!(super::doc_title("#show: x\nversion: \"business\",\n"), None);
    }

    #[test]
    fn project_scan_lists_cards_tree_and_resources() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
        let (cards, nodes, _) = super::scan_project(&root);
        assert!(cards.iter().any(|c| c.rel == "main.typ"));
        assert!(cards.iter().any(|c| c.rel == "chapters/design.typ"));
        let chapters = nodes.iter().find(|n| n.name == "chapters").unwrap();
        assert!(
            chapters
                .children
                .as_ref()
                .unwrap()
                .iter()
                .any(|n| n.name == "design.typ")
        );
        // Filtering keeps the folder leading to a match.
        assert!(chapters.to_item(0, "design").is_some());
        assert!(chapters.to_item(0, "nothing-matches").is_none());
    }

    #[test]
    fn line_and_column_count_chars() {
        let text = "第一行\nab中文cd";
        let off = text.find("cd").unwrap();
        assert_eq!(super::line_col(text, off), (2, 5));
        assert_eq!(super::line_col(text, 0), (1, 1));
        // Offsets inside a multi-byte char snap to its start.
        assert_eq!(super::line_col(text, 1), (1, 1));
    }

    #[test]
    fn project_search_finds_matches_in_open_and_disk_text() {
        let sources = vec![
            (
                "a.typ".to_string(),
                Some("= 标题\n  正文 关键词 结尾\n".to_string()),
            ),
            ("b.typ".to_string(), Some("关键词关键词".to_string())),
        ];
        let hits = super::search_sources(&sources, "关键词");
        assert_eq!(hits.len(), 3);
        let first = &hits[0];
        assert_eq!(first.line, 2);
        assert_eq!(&first.snippet[first.snippet_match.clone()], "关键词");
        assert_eq!(first.snippet, "正文 关键词 结尾");
    }

    #[test]
    fn heads_are_cut_at_char_boundaries() {
        let dir = std::env::temp_dir().join(format!("zj-head-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("h.typ");
        std::fs::write(&file, "中文字符").unwrap();
        // 4 bytes cuts the second character in half.
        assert_eq!(super::read_head(&file, 4), "中");
        let _ = std::fs::remove_dir_all(dir);
    }
}
