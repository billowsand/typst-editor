//! Root view, dressed as a Smartisan Notes desk: a walnut title bar, a birch
//! desk holding paper cards (documents or the file tree, and the outline),
//! a ruled notepad (editor), the typeset pages (preview) and a walnut status
//! strip.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_kit::component::input::{
    Editor, EditorState, Input, InputEvent, InputState, RopeExt, TabSize,
};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::tree::{TreeItem, TreeState, tree};
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use typst_core::{CompileResult, CompilerState, OutlineItem};

use crate::highlight;
use crate::lsp::{ActivePath, TypstLsp};
use crate::preview::{PreviewEvent, PreviewPanel};
use crate::skin::*;
use crate::theme::*;
use crate::workspace::{Workspace, normalize};

actions!(
    typst_editor,
    [
        OpenFile,
        OpenFolder,
        Save,
        CloseTab,
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
        RevealProject,
        Rescan,
        OpenDemo,
        UndoEdit,
        RedoEdit,
        About
    ]
);

/// Open a document by path (title and tab dropdowns).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
struct OpenDocument {
    path: String,
}

/// Debounce between the last keystroke and a compile.
const COMPILE_DEBOUNCE: Duration = Duration::from_millis(120);
/// Debounce between cursor movement and the preview marker update.
const CURSOR_DEBOUNCE: Duration = Duration::from_millis(60);
/// Directories never scanned.
const IGNORED_DIRS: &[&str] = &["target", "node_modules"];
/// Extensions opened in the editor; everything else is revealed on disk.
const TEXT_EXTS: &[&str] = &[
    "typ", "bib", "yml", "yaml", "toml", "txt", "csv", "json", "xml", "md", "svg", "tex",
];

/// Notepad geometry: the tinted margin strip, and the top padding the
/// gpui-kit editor puts above its first line (the rules align to it).
const MARGIN_W: f32 = 44.;
const EDITOR_TOP_PAD: f32 = 4.;
const DEFAULT_FONT: f32 = 15.;
/// Height of a row on lined cards (outline, file tree, dialogs).
const ROW_H: f32 = 30.;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-o", OpenFile, None),
        KeyBinding::new("secondary-shift-o", OpenFolder, None),
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        KeyBinding::new("secondary-=", ZoomIn, None),
        KeyBinding::new("secondary--", ZoomOut, None),
        KeyBinding::new("secondary-0", ZoomReset, None),
        KeyBinding::new("secondary-9", FitWidth, None),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("secondary-shift-e", ExportPdf, None),
        KeyBinding::new("f5", Recompile, None),
    ]);
}

/// A `.typ` file shown as a note card.
#[derive(Clone)]
struct DocCard {
    path: String,
    /// Path relative to the project root, with `/` separators.
    rel: String,
    /// First heading or comment of the file, else its name.
    title: String,
    modified: Option<SystemTime>,
}

/// Plain (Send) mirror of a file tree entry, built off the UI thread.
#[derive(Clone)]
struct TreeNode {
    path: String,
    name: String,
    children: Option<Vec<TreeNode>>,
}

impl TreeNode {
    /// Convert to tree items; with a query, keep only matches and the
    /// folders leading to them (expanded).
    fn to_item(&self, depth: usize, query: &str) -> Option<TreeItem> {
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
}

#[derive(Clone, Copy, PartialEq)]
enum Nav {
    /// `.typ` files as note cards.
    Drafts,
    /// The whole project as a directory tree.
    Files,
}

/// Something waiting on a decision about unsaved edits.
#[derive(Clone)]
enum Pending {
    /// Switch to another project, then optionally open a file in it.
    Switch {
        root: PathBuf,
        open: Option<PathBuf>,
    },
    CloseTab(String),
    CloseWindow,
}

enum CompileStatus {
    Idle,
    Ok { ms: u64 },
    Failed { first: String, count: usize },
}

pub struct TypstEditorApp {
    compiler: Arc<CompilerState>,
    editor: Entity<EditorState>,
    preview: Entity<PreviewPanel>,
    search: Entity<InputState>,
    tree_state: Entity<TreeState>,
    query: String,
    ws: Workspace,
    /// Mirror of `ws.active()` shared with the completion provider.
    active_path: ActivePath,
    cards: Vec<DocCard>,
    tree_nodes: Vec<TreeNode>,
    resources: usize,
    starred: HashSet<String>,
    char_count: usize,
    last_cursor: usize,
    compiling: bool,
    status: CompileStatus,
    compile_task: Task<()>,
    cursor_task: Task<()>,
    notice_task: Task<()>,
    outline: Vec<OutlineItem>,
    nav: Nav,
    show_sidebar: bool,
    outline_open: bool,
    show_about: bool,
    notice: Option<String>,
    pending: Option<Pending>,
    dialog_focus: FocusHandle,
    tabs_scroll: ScrollHandle,
    /// Compact code mode: line numbers, tighter lines, no ruling.
    compact: bool,
    font_px: f32,
    _subscriptions: Vec<Subscription>,
}

impl TypstEditorApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let compiler = Arc::new(CompilerState::new());
        let active_path: ActivePath = Rc::default();

        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(highlight::LANGUAGE)
                .line_number(false)
                .soft_wrap(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .placeholder("在稿纸上写点什么……")
        });
        editor.update(cx, |state, cx| {
            let lsp = Rc::new(TypstLsp {
                compiler: compiler.clone(),
                active_path: active_path.clone(),
            });
            state.lsp_mut().completion_provider = Some(lsp.clone());
            state.lsp_mut().hover_provider = Some(lsp);
            state.set_highlighter_factory(highlight::typst_highlighter_factory(), cx);
            cx.notify();
        });

        let preview = cx.new(|_| PreviewPanel::new(compiler.clone()));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索文稿、文件与标题"));
        let tree_state = cx.new(|cx| TreeState::new(cx));

        let _subscriptions = vec![
            cx.subscribe(&editor, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.on_text_changed(cx);
                }
            }),
            // Cursor moves only show up as notifications from the editor.
            cx.observe(&editor, |this, editor, cx| {
                let cursor = editor.read(cx).cursor();
                if cursor != this.last_cursor {
                    this.last_cursor = cursor;
                    this.schedule_cursor_sync(cx);
                }
                cx.notify();
            }),
            cx.subscribe(&search, |this, search, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = search.read(cx).value().trim().to_lowercase();
                    this.refresh_tree(cx);
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &preview,
                window,
                |this, _, event: &PreviewEvent, window, cx| {
                    let PreviewEvent::JumpToSource { file, offset } = event;
                    this.jump_to(file.clone(), *offset, window, cx);
                },
            ),
        ];

        // Closing the window with unsaved edits asks first.
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.on_close_requested(window, cx))
                .unwrap_or(true)
        });

        Self {
            compiler,
            editor,
            preview,
            search,
            tree_state,
            query: String::new(),
            ws: Workspace::default(),
            active_path,
            cards: Vec::new(),
            tree_nodes: Vec::new(),
            resources: 0,
            starred: HashSet::new(),
            char_count: 0,
            last_cursor: 0,
            compiling: false,
            status: CompileStatus::Idle,
            compile_task: Task::ready(()),
            cursor_task: Task::ready(()),
            notice_task: Task::ready(()),
            outline: Vec::new(),
            nav: Nav::Drafts,
            show_sidebar: true,
            outline_open: true,
            show_about: false,
            notice: None,
            pending: None,
            dialog_focus: cx.focus_handle(),
            tabs_scroll: ScrollHandle::new(),
            compact: false,
            font_px: DEFAULT_FONT,
            _subscriptions,
        }
    }

    fn active(&self) -> Option<String> {
        self.ws.active().map(str::to_owned)
    }

    fn is_active(&self, path: &str) -> bool {
        self.ws.is_active(path)
    }

    fn line_height(&self) -> f32 {
        (self.font_px * if self.compact { 1.55 } else { 2.0 }).round()
    }

    fn sync_active_path(&self) {
        *self.active_path.borrow_mut() = self.active();
    }

    /// Write the editor's text into the active buffer.
    fn stash_editor(&mut self, cx: &App) {
        if let Some(active) = self.active() {
            let text = self.editor.read(cx).text().to_string();
            self.ws.set_current(&active, text);
        }
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
        self.ws.reset(Some(root));
        self.sync_active_path();
        self.compiler.reset_session();
        self.compile_task = Task::ready(());
        self.cursor_task = Task::ready(());
        self.compiling = false;
        self.status = CompileStatus::Idle;
        self.outline.clear();
        self.cards.clear();
        self.tree_nodes.clear();
        self.resources = 0;
        self.char_count = 0;
        self.preview.update(cx, |preview, cx| {
            preview.set_document(vec![], vec![], cx);
            preview.set_markers(vec![], cx);
        });
        self.editor.update(cx, |state, cx| {
            state.set_value("", window, cx);
            if let Some(set) = state.diagnostics_mut() {
                set.clear();
            }
        });
        self.rescan(cx);
        if let Some(file) = open {
            self.open_document(file.display().to_string(), window, cx);
        }
        cx.notify();
    }

    /// Re-read the project for the cards and the file tree.
    fn rescan(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.ws.root().map(Path::to_path_buf) else {
            return;
        };
        let generation = self.ws.generation();
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

    fn open_document(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let path = normalize(&path);
        if self.is_active(&path) && self.ws.buffer(&path).is_some() {
            return;
        }
        self.stash_editor(cx);
        if let Err(err) = self.ws.open(&path, |p| std::fs::read_to_string(p)) {
            self.show_notice(err, cx);
            return;
        }
        self.sync_active_path();
        let text = self
            .ws
            .active_buffer()
            .map(|b| b.current.clone())
            .unwrap_or_default();
        self.char_count = count_chars(&text);
        self.last_cursor = 0;
        let typst = is_typst(&path);
        self.editor.update(cx, |state, cx| {
            state.set_highlighter(if typst { highlight::LANGUAGE } else { "text" }, cx);
            state.set_value(text.clone(), window, cx);
            if let Some(set) = state.diagnostics_mut() {
                set.clear();
            }
            state.focus(window, cx);
        });
        if let Some(ix) = self.ws.tab_index(&path) {
            self.tabs_scroll.scroll_to_item(ix);
        }

        if typst {
            // The first document opened becomes the preview entry; later
            // ones route to whichever entry imports them.
            let make_entry = self.compiler.preview_entry().is_none();
            self.schedule_compile(Duration::ZERO, make_entry, cx);
        } else if self.compiler.entry_for(&path).is_some() {
            // A bibliography or data file a document reads: recompile it.
            self.schedule_compile(Duration::ZERO, false, cx);
        }
        cx.notify();
    }

    fn jump_to(
        &mut self,
        file: String,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_active(&file) {
            self.open_document(file, window, cx);
        }
        self.editor.update(cx, |state, cx| {
            state.set_selected_range(offset..offset, cx);
            state.focus(window, cx);
        });
    }

    /// Click on a file tree row.
    fn open_from_tree(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let ext = Path::new(&path)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if TEXT_EXTS.contains(&ext.as_str()) {
            self.open_document(path, window, cx);
        } else {
            reveal_in_file_manager(Path::new(&path));
            self.show_notice("已在资源管理器中显示此文件", cx);
        }
    }

    /// Pin `path` as the preview entry, or unpin it when it already is.
    /// Runs off the UI thread: it waits for the compile lock.
    fn toggle_pin(&mut self, path: String, cx: &mut Context<Self>) {
        let path = normalize(&path);
        let unpin = self
            .compiler
            .pinned_entry()
            .is_some_and(|p| normalize(&p.display().to_string()) == path);
        let text = match self.ws.buffer(&path) {
            Some(buffer) => buffer.current.clone(),
            None => std::fs::read_to_string(&path).unwrap_or_default(),
        };
        let compiler = self.compiler.clone();
        let generation = self.ws.generation();
        self.compiling = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let _guard = compiler.compile_lock();
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

    fn on_open_file(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_open(false, window, cx);
    }

    fn on_open_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_open(true, window, cx);
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

    // ── Unsaved edits ───────────────────────────────────────

    /// Run `action`, asking first if it would discard unsaved edits.
    fn request(&mut self, action: Pending, window: &mut Window, cx: &mut Context<Self>) {
        self.stash_editor(cx);
        if self.affected_dirty(&action).is_empty() {
            self.perform(action, false, window, cx);
        } else {
            self.pending = Some(action);
            self.dialog_focus.focus(window, cx);
            cx.notify();
        }
    }

    fn affected_dirty(&self, action: &Pending) -> Vec<String> {
        match action {
            Pending::CloseTab(path) => self
                .ws
                .buffer(path)
                .filter(|b| b.is_dirty())
                .map(|_| vec![normalize(path)])
                .unwrap_or_default(),
            Pending::Switch { .. } | Pending::CloseWindow => self.ws.dirty_paths(),
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
            Pending::CloseWindow => window.remove_window(),
        }
    }

    /// The unsaved-edits dialog's answer: save first, discard, or cancel.
    fn resolve_pending(&mut self, save: Option<bool>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.pending.take() else {
            return;
        };
        match save {
            None => {}
            Some(true) => {
                let paths = self.affected_dirty(&action);
                if self.save_paths(&paths, cx) {
                    self.perform(action, false, window, cx);
                } else {
                    // Keep the edits; the notice says what failed.
                    self.pending = None;
                }
            }
            Some(false) => self.perform(action, true, window, cx),
        }
        cx.notify();
    }

    fn on_close_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.stash_editor(cx);
        if self.ws.dirty_paths().is_empty() {
            return true;
        }
        self.pending = Some(Pending::CloseWindow);
        self.dialog_focus.focus(window, cx);
        cx.notify();
        false
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(active) = self.active() {
            self.request(Pending::CloseTab(active), window, cx);
        }
    }

    fn close_tab_now(
        &mut self,
        path: String,
        discarded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = self.ws.close(&path);
        if discarded {
            // The compiler still holds the discarded text; put it back to
            // what is on disk so the preview stops showing it.
            self.revert_overlay(path.clone(), cx);
        }
        if self.active().is_none() {
            self.sync_active_path();
            match next {
                Some(next) => self.open_document(next, window, cx),
                None => {
                    self.char_count = 0;
                    self.cursor_task = Task::ready(());
                    self.editor
                        .update(cx, |state, cx| state.set_value("", window, cx));
                    self.preview
                        .update(cx, |preview, cx| preview.set_markers(vec![], cx));
                }
            }
        }
        cx.notify();
    }

    fn revert_overlay(&mut self, path: String, cx: &mut Context<Self>) {
        if !is_typst(&path) && self.compiler.entry_for(&path).is_none() {
            return;
        }
        let Ok(disk) = std::fs::read_to_string(&path) else {
            return;
        };
        self.run_compile(path, disk, Duration::ZERO, false, cx);
    }

    // ── Editing, saving ─────────────────────────────────────

    fn on_text_changed(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.active() else { return };
        let text = self.editor.read(cx).text().to_string();
        let count = count_chars(&text);
        if !self.ws.set_current(&path, text) {
            return;
        }
        self.char_count = count;
        if is_typst(&path) || self.compiler.entry_for(&path).is_some() {
            self.schedule_compile(COMPILE_DEBOUNCE, false, cx);
        }
        cx.notify();
    }

    /// Write buffers to disk. Returns whether all succeeded.
    fn save_paths(&mut self, paths: &[String], cx: &mut Context<Self>) -> bool {
        self.stash_editor(cx);
        let mut ok = true;
        for path in paths {
            let Some(text) = self.ws.buffer(path).map(|b| b.current.clone()) else {
                continue;
            };
            match std::fs::write(path, &text) {
                Ok(()) => self.ws.mark_saved(path),
                Err(err) => {
                    ok = false;
                    self.show_notice(format!("保存失败：{err}"), cx);
                }
            }
        }
        self.rescan(cx);
        cx.notify();
        ok
    }

    fn on_save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(active) = self.active() {
            self.save_paths(&[active], cx);
        }
    }

    fn show_notice(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
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
    fn schedule_compile(&mut self, delay: Duration, make_entry: bool, cx: &mut Context<Self>) {
        let Some(path) = self.active() else { return };
        let text = self.editor.read(cx).text().to_string();
        self.run_compile(path, text, delay, make_entry, cx);
    }

    /// Compile `path` with `text`. Replacing the task cancels a compile that
    /// has not started; results for an older session are dropped.
    fn run_compile(
        &mut self,
        path: String,
        text: String,
        delay: Duration,
        make_entry: bool,
        cx: &mut Context<Self>,
    ) {
        let compiler = self.compiler.clone();
        let generation = self.ws.generation();
        self.compiling = true;

        self.compile_task = cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let (result, sizes) = cx
                .background_spawn(async move {
                    let _guard = compiler.compile_lock();
                    if make_entry {
                        if let Err(err) = compiler.open(&path, &text) {
                            return (CompileResult::error(vec![err]), vec![]);
                        }
                    }
                    let mut result = compiler.update_file_and_compile(&path, &text);
                    typst_core::ide::populate_outline(&compiler, &mut result);
                    let sizes = compiler.page_sizes();
                    (result, sizes)
                })
                .await;
            this.update(cx, |this, cx| {
                if this.ws.generation() == generation {
                    this.apply_compile_result(result, sizes, cx);
                }
            })
            .ok();
        });
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
        }
        self.show_diagnostics(&result, cx);
        cx.notify();
    }

    fn show_diagnostics(&mut self, result: &CompileResult, cx: &mut Context<Self>) {
        let Some(active) = self.active() else { return };
        self.editor.update(cx, |state, cx| {
            let rope = state.text().clone();
            let diagnostics: Vec<_> = result
                .diagnostics
                .iter()
                .filter(|d| normalize(&d.file) == active)
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

    // ── Cursor -> preview ───────────────────────────────────

    fn schedule_cursor_sync(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.active() else { return };
        let offset = self.last_cursor;
        let compiler = self.compiler.clone();
        self.cursor_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CURSOR_DEBOUNCE).await;
            let markers = cx
                .background_spawn(async move {
                    typst_core::ide::jump_from_offset(&compiler, &path, offset)
                })
                .await;
            this.update(cx, |this, cx| {
                this.preview
                    .update(cx, |preview, cx| preview.set_markers(markers, cx));
            })
            .ok();
        });
    }

    // ── View settings, export, demo ─────────────────────────

    fn zoom_by(&mut self, factor: Option<f32>, cx: &mut Context<Self>) {
        self.preview.update(cx, |preview, cx| {
            let zoom = factor.map_or(1.0, |f| preview.zoom() * f);
            preview.set_zoom(zoom, cx);
        });
        cx.notify();
    }

    fn set_compact(&mut self, compact: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.compact = compact;
        self.editor
            .update(cx, |state, cx| state.set_line_number(compact, window, cx));
        cx.notify();
    }

    fn on_export_pdf(&mut self, _: &ExportPdf, window: &mut Window, cx: &mut Context<Self>) {
        if self.compiling {
            self.show_notice("正在编译，请稍后导出", cx);
            return;
        }
        let Some(entry) = self.compiler.preview_entry() else {
            self.show_notice("还没有可导出的文档", cx);
            return;
        };
        let path = PathBuf::from(entry);
        let name = path
            .with_extension("pdf")
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "document.pdf".into());
        let picker = cx.prompt_for_new_path(path.parent().unwrap_or(Path::new(".")), Some(&name));
        let compiler = self.compiler.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Some(mut path) = picker.await.ok().and_then(Result::ok).flatten() else {
                return;
            };
            if path.extension().is_none() {
                path.set_extension("pdf");
            }
            let result = cx
                .background_spawn(async move {
                    let _guard = compiler.compile_lock();
                    compiler
                        .export_pdf()
                        .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()))
                })
                .await;
            this.update(cx, |this, cx| {
                let text = match result {
                    Ok(()) => "PDF 已导出".to_string(),
                    Err(err) => format!("导出失败：{err}"),
                };
                this.show_notice(text, cx);
            })
            .ok();
        })
        .detach();
    }

    fn open_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_path(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            window,
            cx,
        );
    }

    fn toggle_star(&mut self, path: String, cx: &mut Context<Self>) {
        let path = normalize(&path);
        if !self.starred.remove(&path) {
            self.starred.insert(path);
        }
        cx.notify();
    }

    // ── Title bar ───────────────────────────────────────────

    /// Like Smartisan's folder title ("全部便签"): the project folder; the
    /// dropdown beside it switches between its documents.
    fn title_text(&self) -> String {
        self.ws
            .root()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "字笺".into())
    }

    fn render_menus(&self, cx: &App) -> impl IntoElement {
        let menu = |id: &'static str, label: &'static str| {
            bar_button(id, cx)
                .compact()
                .px_2()
                .h(px(28.))
                .child(div().text_size(px(13.)).child(embossed(label)))
        };
        let compact = self.compact;
        let outline_open = self.outline_open;
        h_flex()
            .gap_0p5()
            .child(menu("menu-file", "文件").dropdown_menu(|m, _, _| {
                m.menu("打开文件…", Box::new(OpenFile))
                    .menu("打开文件夹…", Box::new(OpenFolder))
                    .separator()
                    .menu("保存", Box::new(Save))
                    .menu("关闭当前文稿", Box::new(CloseTab))
                    .separator()
                    .menu("导出 PDF…", Box::new(ExportPdf))
            }))
            .child(menu("menu-edit", "编辑").dropdown_menu(|m, _, _| {
                m.menu("撤销", Box::new(UndoEdit))
                    .menu("重做", Box::new(RedoEdit))
            }))
            .child(menu("menu-view", "视图").dropdown_menu(move |m, _, _| {
                m.menu("显示 / 隐藏文稿栏", Box::new(ToggleSidebar))
                    .menu_with_check("目录卡", outline_open, Box::new(ToggleOutline))
                    .separator()
                    .menu_with_check("紧凑代码模式（行号）", compact, Box::new(ToggleCompact))
                    .menu("增大字号", Box::new(FontBigger))
                    .menu("减小字号", Box::new(FontSmaller))
                    .menu("默认字号", Box::new(FontReset))
                    .separator()
                    .menu("预览适合宽度", Box::new(FitWidth))
                    .menu("预览实际大小", Box::new(ZoomReset))
                    .menu("放大预览", Box::new(ZoomIn))
                    .menu("缩小预览", Box::new(ZoomOut))
            }))
            .child(menu("menu-doc", "文档").dropdown_menu(|m, _, _| {
                m.menu("重新编译", Box::new(Recompile))
                    .menu("重新扫描项目", Box::new(Rescan))
                    .menu("在资源管理器中显示项目", Box::new(RevealProject))
                    .separator()
                    .menu("打开排版示例", Box::new(OpenDemo))
            }))
            .child(
                menu("menu-help", "帮助")
                    .dropdown_menu(|m, _, _| m.menu("关于字笺 / 快捷键", Box::new(About))),
            )
    }

    fn render_title(&self, cx: &App) -> impl IntoElement {
        let docs: Vec<(String, String)> = self
            .cards
            .iter()
            .map(|c| (c.path.clone(), format!("{}  ·  {}", c.title, c.rel)))
            .collect();
        bar_button("title-menu", cx)
            .compact()
            .px_3()
            .h(px(36.))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .max_w(px(360.))
                            .overflow_hidden()
                            .text_size(px(19.))
                            .font_weight(FontWeight::BOLD)
                            .child(embossed(self.title_text())),
                    )
                    // Smartisan's recessed folder triangle.
                    .child(
                        div()
                            .relative()
                            .size(px(14.))
                            .child(
                                svg()
                                    .path("skin/g_triangle.svg")
                                    .absolute()
                                    .top(px(1.))
                                    .size(px(14.))
                                    .text_color(rgba(0xfff0dc33)),
                            )
                            .child(
                                svg()
                                    .path("skin/g_triangle.svg")
                                    .absolute()
                                    .size(px(14.))
                                    .text_color(rgb(0x51423a)),
                            ),
                    ),
            )
            .dropdown_menu(move |mut m, _, _| {
                for (path, label) in &docs {
                    m = m.menu(label.clone(), Box::new(OpenDocument { path: path.clone() }));
                }
                if !docs.is_empty() {
                    m = m.separator();
                }
                m.menu("打开文件夹…", Box::new(OpenFolder))
                    .menu("打开文件…", Box::new(OpenFile))
            })
    }

    fn render_titlebar(&self, cx: &App) -> impl IntoElement {
        let tool =
            |id: &'static str, glyph: &'static str, tip: &'static str, action: Box<dyn Action>| {
                bar_button(id, cx)
                    .compact()
                    .size(px(34.))
                    .tooltip(tip)
                    .child(bar_glyph(glyph, 19.))
                    .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            };
        let window_button = |id: &'static str, glyph: &'static str, area: WindowControlArea| {
            div()
                .id(id)
                .w(px(40.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .window_control_area(area)
                .hover(move |d| {
                    d.bg(if id == "window-close" {
                        rgba(0xa54e40d9)
                    } else {
                        rgba(0x0000001f)
                    })
                })
                .child(bar_glyph(glyph, 14.))
        };

        walnut_bar(54.).child(
            h_flex()
                .relative()
                .size_full()
                .child(
                    h_flex()
                        .flex_1()
                        .h_full()
                        .pl_3()
                        .gap_2()
                        .child(skin("notebook", 26., 26.))
                        .child(
                            div()
                                .mr_2()
                                .text_size(px(15.))
                                .font_weight(FontWeight::BOLD)
                                .child(embossed("字笺")),
                        )
                        .child(self.render_menus(cx))
                        .child(
                            div()
                                .flex_1()
                                .h_full()
                                .window_control_area(WindowControlArea::Drag),
                        ),
                )
                .child(self.render_title(cx))
                .child(
                    h_flex()
                        .flex_1()
                        .h_full()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .h_full()
                                .window_control_area(WindowControlArea::Drag),
                        )
                        .child(tool(
                            "tool-open",
                            "folder",
                            "打开文件夹  Ctrl+Shift+O",
                            Box::new(OpenFolder),
                        ))
                        .child(tool("tool-save", "disk", "保存  Ctrl+S", Box::new(Save)))
                        .child(tool(
                            "tool-compile",
                            "compile",
                            "重新编译  F5",
                            Box::new(Recompile),
                        ))
                        .child(tool(
                            "tool-export",
                            "plane",
                            "导出 PDF  Ctrl+Shift+E",
                            Box::new(ExportPdf),
                        ))
                        .child(groove_v(24.).mx_1())
                        .child(tool(
                            "tool-sidebar",
                            "sidebar",
                            "文稿栏  Ctrl+B",
                            Box::new(ToggleSidebar),
                        ))
                        .when(cfg!(target_os = "windows"), |bar| {
                            bar.child(div().w(px(8.)))
                                .child(window_button("window-min", "min", WindowControlArea::Min))
                                .child(window_button("window-max", "max", WindowControlArea::Max))
                                .child(window_button(
                                    "window-close",
                                    "close",
                                    WindowControlArea::Close,
                                ))
                        })
                        .when(!cfg!(target_os = "windows"), |bar| {
                            bar.child(div().w(px(10.)))
                        }),
                ),
        )
    }

    // ── Sidebar ─────────────────────────────────────────────

    fn render_search(&self) -> impl IntoElement {
        inset_pill()
            .h(px(34.))
            .mx(px(14.))
            .mt(px(14.))
            .flex_none()
            .flex()
            .items_center()
            .pl_3()
            .child(paper_glyph("search", 16., 0xb5aa9c))
            .child(
                div().flex_1().child(
                    Input::new(&self.search)
                        .appearance(false)
                        .small()
                        .cleanable(true)
                        .text_size(px(13.)),
                ),
            )
    }

    /// 文稿 | 文件: a recessed track with a raised paper knob.
    fn render_nav_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let segment = |id: &'static str, nav: Nav, glyph: &'static str, label: &'static str| {
            let on = self.nav == nav;
            h_flex()
                .id(id)
                .flex_1()
                .h_full()
                .justify_center()
                .gap_1p5()
                .rounded(px(5.))
                .cursor_pointer()
                .text_size(px(12.5))
                .when(on, |s| {
                    s.bg(linear_gradient(
                        180.,
                        linear_color_stop(rgb(0xfffdf8), 0.),
                        linear_color_stop(rgb(0xf3ecdf), 1.),
                    ))
                    .border_1()
                    .border_color(rgb(0xd6c8b2))
                    .shadow(vec![
                        shadow(0., 1., 1.5, 0., 0x5a3c1e40),
                        inset(0., 1., 0., 0xffffffcc),
                    ])
                    .text_color(rgb(INK))
                    .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!on, |s| {
                    s.text_color(rgb(INK_SOFT))
                        .hover(|s| s.bg(rgba(0xffffff40)))
                })
                .child(paper_glyph(
                    glyph,
                    13.,
                    if on { INK_SOFT } else { 0xa89a88 },
                ))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.nav = nav;
                    cx.notify();
                }))
        };
        h_flex()
            .flex_none()
            .h(px(32.))
            .mx(px(14.))
            .mt(px(10.))
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(7.))
            .bg(rgba(0x8a6a402e))
            .shadow(vec![
                inset(0., 1., 2.5, 0x5a3c1e40),
                shadow(0., 1., 0., 0., 0xffffff80),
            ])
            .child(segment("nav-drafts", Nav::Drafts, "files", "文稿"))
            .child(segment("nav-files", Nav::Files, "folder", "文件"))
    }

    /// Binding strip down the left edge of a card, with its double edge.
    fn binding_strip(tint: u32) -> Div {
        div()
            .absolute()
            .top_0()
            .left_0()
            .h_full()
            .w(px(26.))
            .bg(rgb(tint))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right(px(1.))
                    .h_full()
                    .w(px(1.))
                    .bg(rgb(0xebe2d4)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .h_full()
                    .w(px(1.))
                    .bg(rgba(0xffffffcc)),
            )
    }

    fn card_frame(id: impl Into<ElementId>, lifted: bool) -> Stateful<Div> {
        div()
            .id(id.into())
            .relative()
            .flex_none()
            .ml(px(10.))
            .mr(px(2.))
            .rounded(px(2.))
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgb(CARD_HEAD), 0.),
                linear_color_stop(rgb(CARD), 1.),
            ))
            .border_1()
            .border_color(rgba(0xb4966c40))
            .shadow(if lifted {
                lifted_shadow()
            } else {
                paper_shadow()
            })
    }

    fn render_card(&self, card: &DocCard, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.is_active(&card.path);
        let pinned = self
            .compiler
            .pinned_entry()
            .is_some_and(|p| normalize(&p.display().to_string()) == card.path);
        let auto_entry = !pinned
            && self
                .compiler
                .preview_entry()
                .is_some_and(|p| normalize(&p.display().to_string()) == card.path);
        let starred = self.starred.contains(&card.path);
        let dirty = self.ws.buffer(&card.path).is_some_and(|b| b.is_dirty());
        let path = card.path.clone();
        let pin_path = card.path.clone();
        let star_path = card.path.clone();
        let icon_well = |d: Stateful<Div>| d.p_0p5().rounded_sm().hover(|d| d.bg(rgba(0x8a6a4014)));

        Self::card_frame(("card", ix), active)
            .cursor_pointer()
            .hover(|d| d.shadow(lifted_shadow()))
            .on_click(
                cx.listener(move |this, _, window, cx| {
                    this.open_document(path.clone(), window, cx)
                }),
            )
            .child(Self::binding_strip(if active { 0xf3e6cd } else { BAND }))
            .child(
                v_flex()
                    .relative()
                    .pl(px(36.))
                    .pr(px(10.))
                    .child(
                        h_flex()
                            .h(px(28.))
                            .pr(px(18.))
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(11.5))
                                    .text_color(rgb(FAINT))
                                    .child(match card.modified {
                                        Some(t) => smartisan_time(t),
                                        None => "刚刚".into(),
                                    }),
                            )
                            .when(auto_entry, |row| {
                                row.child(
                                    div()
                                        .px_1()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(rgba(0xb98c5780))
                                        .text_size(px(10.))
                                        .text_color(rgb(0xa8763f))
                                        .child("入口"),
                                )
                            })
                            .child(
                                icon_well(div().id(("pin", ix)))
                                    .tooltip(move |w, cx| {
                                        Tooltip::new(if pinned {
                                            "取消固定（恢复自动选择入口）"
                                        } else {
                                            "固定为预览主文件"
                                        })
                                        .build(w, cx)
                                    })
                                    .child(skin(if pinned { "pin_on" } else { "pin" }, 16., 16.))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_pin(pin_path.clone(), cx);
                                    })),
                            )
                            .child(
                                icon_well(div().id(("star", ix)))
                                    .tooltip(move |w, cx| {
                                        Tooltip::new(if starred {
                                            "取消置顶"
                                        } else {
                                            "置顶"
                                        })
                                        .build(w, cx)
                                    })
                                    .child(skin(if starred { "star_on" } else { "star" }, 16., 16.))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_star(star_path.clone(), cx);
                                    })),
                            ),
                    )
                    .child(paper_divider())
                    .child(
                        v_flex()
                            .py(px(9.))
                            .gap(px(2.))
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .when(dirty, |row| {
                                        row.child(
                                            div().size(px(6.)).rounded_full().bg(rgb(0xc8763a)),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(INK))
                                            .child(card.title.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(12.))
                                    .text_color(rgb(PATH_INK))
                                    .child(card.rel.clone()),
                            ),
                    ),
            )
            // The metal clip holding the card, poking out past its left edge.
            .child(
                skin("paperclip", 36., 18.)
                    .absolute()
                    .left(px(-15.))
                    .top(px(30.)),
            )
            // Silk bookmark on the open document.
            .when(active, |card| {
                card.child(
                    skin("ribbon", 12., 26.)
                        .absolute()
                        .top(px(-3.))
                        .right(px(8.)),
                )
            })
    }

    fn render_drafts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = &self.query;
        let mut cards: Vec<&DocCard> = self
            .cards
            .iter()
            .filter(|c| {
                query.is_empty()
                    || c.title.to_lowercase().contains(query)
                    || c.rel.to_lowercase().contains(query)
            })
            .collect();
        // Starred first, then the order on disk.
        cards.sort_by_key(|c| !self.starred.contains(&c.path));

        let mut list = v_flex()
            .id("doc-cards")
            .size_full()
            .overflow_y_scroll()
            .pt(px(8.))
            .pb(px(14.))
            .pl(px(8.))
            .pr(px(12.))
            .gap(px(10.))
            .child(
                h_flex()
                    .pl(px(12.))
                    .text_size(px(12.))
                    .justify_between()
                    .child(letterpress(format!("文稿 · {}", cards.len()), WOOD_INK))
                    .when(self.resources > 0, |row| {
                        row.child(
                            div()
                                .id("resources-hint")
                                .cursor_pointer()
                                .child(letterpress(
                                    format!("另有 {} 个资源 ›", self.resources),
                                    WOOD_INK,
                                ))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.nav = Nav::Files;
                                    cx.notify();
                                })),
                        )
                    }),
            );
        if cards.is_empty() {
            list = list.child(
                div()
                    .pl(px(12.))
                    .pt_4()
                    .text_size(px(12.5))
                    .child(letterpress(
                        if self.cards.is_empty() {
                            "这个文件夹里还没有 .typ 文稿"
                        } else {
                            "没有匹配的文稿"
                        },
                        WOOD_INK,
                    )),
            );
        }
        for (ix, card) in cards.into_iter().enumerate() {
            list = list.child(self.render_card(card, ix, cx));
        }
        list
    }

    fn render_files(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let active = self.active();
        let tree_view = tree(
            &self.tree_state,
            move |ix, entry, _selected, _window, _cx| {
                let item = entry.item();
                let path = item.id.to_string();
                let folder = entry.is_folder();
                let ext = Path::new(&path)
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                let (icon, tint) = if folder {
                    (
                        if entry.is_expanded() {
                            "skin/g_folder_open.svg"
                        } else {
                            "skin/g_folder.svg"
                        },
                        0xb08850,
                    )
                } else {
                    match ext.as_str() {
                        "typ" => ("skin/g_doc.svg", INK_SOFT),
                        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" => {
                            ("skin/g_image.svg", 0x6f8a8c)
                        }
                        "bib" => ("skin/g_book.svg", 0x8a6a9c),
                        "toml" | "yml" | "yaml" | "json" | "csv" | "xml" => {
                            ("skin/g_braces.svg", 0x7a8a50)
                        }
                        _ => ("skin/g_file.svg", 0xa89a88),
                    }
                };
                let is_active = active.as_deref() == Some(normalize(&path).as_str());
                let view = view.clone();
                ListItem::new(ix)
                    .h(px(ROW_H))
                    .px_2()
                    .pl(px(14.) * entry.depth() + px(8.))
                    .rounded(px(0.))
                    .border_b_1()
                    .border_color(rgb(RULE))
                    .when(is_active, |row| row.bg(rgba(0xe9cf9f40)))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(svg().path(icon).size(px(14.)).text_color(rgb(tint)))
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(13.))
                                    .text_color(rgb(if is_active { 0x8a3f22 } else { INK }))
                                    .when(is_active, |d| d.font_weight(FontWeight::SEMIBOLD))
                                    .child(item.label.clone()),
                            ),
                    )
                    .on_click(move |_, window, cx| {
                        if !folder {
                            view.update(cx, |this, cx| {
                                this.open_from_tree(path.clone(), window, cx)
                            });
                        }
                    })
            },
        );

        // The card stretches across the column; `size_full` plus its side
        // margins would overflow it.
        v_flex()
            .size_full()
            .pt(px(8.))
            .pb(px(14.))
            .pl(px(8.))
            .pr(px(12.))
            .child(
                Self::card_frame("files-card", false)
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(Self::binding_strip(BAND))
                    .child(
                        v_flex()
                            .relative()
                            .size_full()
                            .min_w_0()
                            .overflow_hidden()
                            .child(
                                h_flex()
                                    .h(px(30.))
                                    .flex_none()
                                    .pl(px(36.))
                                    .pr(px(10.))
                                    .gap_2()
                                    .child(paper_glyph("folder", 14., 0xb5aa9c))
                                    .child(
                                        div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_size(px(12.5))
                                            .text_color(rgb(FAINT))
                                            .child(self.title_text()),
                                    ),
                            )
                            .child(div().pl(px(26.)).child(paper_divider()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .pl(px(28.))
                                    .pr(px(4.))
                                    .child(tree_view.size_full()),
                            ),
                    )
                    .child(
                        skin("paperclip", 36., 18.)
                            .absolute()
                            .left(px(-15.))
                            .top(px(12.)),
                    ),
            )
    }

    fn render_outline_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.outline_open;
        let header = h_flex()
            .id("outline-header")
            .h(px(30.))
            .flex_none()
            .pl(px(36.))
            .pr(px(10.))
            .gap_2()
            .cursor_pointer()
            .child(paper_glyph("outline", 14., 0xb5aa9c))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.5))
                    .text_color(rgb(FAINT))
                    .child("目录"),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(FAINT))
                    .child(format!("{} 个标题", self.outline.len())),
            )
            .child(
                svg()
                    .path(if open {
                        "skin/g_chevron_down.svg"
                    } else {
                        "skin/g_chevron_up.svg"
                    })
                    .size(px(13.))
                    .text_color(rgb(0xa89a88)),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.outline_open = !this.outline_open;
                cx.notify();
            }));

        let mut card = Self::card_frame("outline-card", false)
            .child(Self::binding_strip(BAND))
            .child(
                skin("paperclip", 36., 18.)
                    .absolute()
                    .left(px(-15.))
                    .top(px(7.)),
            );
        if !open {
            return card.child(div().relative().child(header));
        }

        let matches =
            |title: &str| self.query.is_empty() || title.to_lowercase().contains(&self.query);
        let visible: Vec<_> = self
            .outline
            .iter()
            .enumerate()
            .filter(|(_, item)| matches(&item.title))
            .collect();
        let mut rows = v_flex()
            .id("outline-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pl(px(36.))
            .pr(px(10.))
            .pb_2();
        if visible.is_empty() {
            rows = rows.child(
                div()
                    .h(px(ROW_H))
                    .flex()
                    .items_center()
                    .text_size(px(12.5))
                    .text_color(rgb(FAINT))
                    .child(if self.outline.is_empty() {
                        "标题会在编译后出现在这里"
                    } else {
                        "没有匹配的标题"
                    }),
            );
        }
        for (ix, item) in visible {
            let target = item.file.clone().zip(item.offset);
            let current = target.as_ref().is_some_and(|(file, offset)| {
                self.is_active(file)
                    && *offset <= self.last_cursor
                    && !self.outline.iter().skip(ix + 1).any(|next| {
                        next.file.as_deref().is_some_and(|f| self.is_active(f))
                            && next.offset.is_some_and(|n| n <= self.last_cursor)
                    })
            });
            let indent = item.level.saturating_sub(1).min(5) as f32 * 14.;
            rows = rows.child(
                h_flex()
                    .id(("outline", ix))
                    .flex_none()
                    .h(px(ROW_H))
                    .pl(px(indent))
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(RULE))
                    .cursor_pointer()
                    .hover(|d| d.bg(rgba(0xe9cf9f22)))
                    .child(
                        div()
                            .w(px(4.))
                            .h(px(14.))
                            .rounded_sm()
                            .when(current, |d| d.bg(rgb(0xc0533d))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(if item.level == 1 { 13.5 } else { 12.5 }))
                            .when(item.level == 1, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .text_color(rgb(if current { 0x8a3f22 } else { INK_SOFT }))
                            .child(item.title.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some((file, offset)) = &target {
                            this.jump_to(file.clone(), *offset, window, cx);
                        }
                    })),
            );
        }
        card = card.flex_1().min_h_0().child(
            v_flex()
                .relative()
                .size_full()
                .child(header)
                .child(div().pl(px(26.)).child(paper_divider()))
                .child(rows),
        );
        card
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let list: AnyElement = match self.nav {
            Nav::Drafts => self.render_drafts(cx).into_any_element(),
            Nav::Files => self.render_files(cx).into_any_element(),
        };
        let outline = v_flex()
            .size_full()
            .pt(px(4.))
            .pb(px(14.))
            .pl(px(8.))
            .pr(px(12.))
            .child(self.render_outline_card(cx));

        let body: AnyElement = if self.outline_open {
            // Files and outline scroll on their own; drag between them.
            gpui_kit::base::v_resizable("sidebar-split")
                .child(
                    gpui_kit::base::resizable_panel()
                        .size_range(px(140.)..px(3000.))
                        .child(list),
                )
                .child(
                    gpui_kit::base::resizable_panel()
                        .size(px(300.))
                        .size_range(px(120.)..px(3000.))
                        .child(outline),
                )
                .into_any_element()
        } else {
            v_flex()
                .size_full()
                .child(div().flex_1().min_h_0().child(list))
                .child(
                    div()
                        .flex_none()
                        .pl(px(8.))
                        .pr(px(12.))
                        .pb(px(14.))
                        .child(self.render_outline_card(cx)),
                )
                .into_any_element()
        };

        v_flex()
            .size_full()
            .child(self.render_search())
            .child(self.render_nav_switch(cx))
            .child(div().flex_1().min_h_0().child(body))
    }

    // ── Notepad: index tabs, sheet header, ruled paper ──────

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<(String, bool)> = self
            .ws
            .tabs()
            .map(|(p, b)| (p.to_owned(), b.is_dirty()))
            .collect();
        let menu_items: Vec<(String, String)> = tabs
            .iter()
            .map(|(p, dirty)| {
                let name = file_name(p);
                (p.clone(), if *dirty { format!("● {name}") } else { name })
            })
            .collect();

        let mut strip = h_flex()
            .id("tab-strip")
            .flex_1()
            .min_w_0()
            .h_full()
            .items_end()
            .gap(px(3.))
            .overflow_x_scroll()
            .track_scroll(&self.tabs_scroll);
        for (ix, (path, dirty)) in tabs.into_iter().enumerate() {
            let selected = self.is_active(&path);
            let group: SharedString = format!("tab-{ix}").into();
            let open_path = path.clone();
            let close_path = path.clone();
            strip = strip.child(
                h_flex()
                    .id(("tab", ix))
                    .group(group.clone())
                    .relative()
                    .flex_none()
                    .max_w(px(200.))
                    .h(px(if selected { 30. } else { 26. }))
                    .pl(px(12.))
                    .pr(px(6.))
                    .gap_1()
                    .rounded_t(px(6.))
                    .border_1()
                    .border_b_0()
                    .border_color(rgba(0xb4966c4d))
                    .bg(if selected {
                        rgb(CARD_HEAD)
                    } else {
                        rgb(0xeee5d5)
                    })
                    .when(!selected, |t| {
                        t.shadow(vec![inset(0., -2., 3., 0x5a3c1e1a)])
                            .hover(|t| t.bg(rgb(0xf3ebdd)))
                    })
                    .cursor_pointer()
                    .tooltip({
                        let path = path.clone();
                        move |w, cx| Tooltip::new(path.clone()).build(w, cx)
                    })
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.5))
                            .text_color(rgb(if selected { INK } else { INK_SOFT }))
                            .when(selected, |t| t.font_weight(FontWeight::SEMIBOLD))
                            .child(file_name(&path)),
                    )
                    // Close button; an unsaved tab shows an ink dot until hovered.
                    .child(
                        div()
                            .id(("tab-close", ix))
                            .relative()
                            .flex_none()
                            .size(px(18.))
                            .rounded(px(3.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|d| d.bg(rgba(0x8a6a4026)))
                            .when(dirty, |d| {
                                d.child(
                                    div()
                                        .size(px(7.))
                                        .rounded_full()
                                        .bg(rgb(0xc8763a))
                                        .group_hover(group.clone(), |s| s.invisible()),
                                )
                            })
                            .child(
                                svg()
                                    .path("skin/g_close.svg")
                                    .absolute()
                                    .top(px(3.))
                                    .left(px(3.))
                                    .size(px(12.))
                                    .text_color(rgb(INK_SOFT))
                                    .when(dirty || !selected, |s| s.invisible())
                                    .group_hover(group, |s| s.visible()),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.request(Pending::CloseTab(close_path.clone()), window, cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_document(open_path.clone(), window, cx)
                    })),
            );
        }

        h_flex()
            .h(px(30.))
            .flex_none()
            .pl(px(18.))
            .pr(px(6.))
            .gap_1()
            .child(strip)
            .when(menu_items.len() > 1, |row| {
                row.child(
                    bar_button("tab-overflow", cx)
                        .compact()
                        .size(px(24.))
                        .tooltip("所有打开的文稿")
                        .child(paper_glyph("triangle", 12., 0x9c8f7e))
                        .dropdown_menu(move |mut m, _, _| {
                            for (path, label) in &menu_items {
                                m = m.menu(
                                    label.clone(),
                                    Box::new(OpenDocument { path: path.clone() }),
                                );
                            }
                            m
                        }),
                )
            })
    }

    fn render_sheet_header(&self, cx: &App) -> impl IntoElement {
        let project = self.title_text();
        let active = self.active();
        let dirty = self.ws.active_buffer().is_some_and(|b| b.is_dirty());
        let modified = active.as_ref().and_then(|path| {
            self.cards
                .iter()
                .find(|c| c.path == *path)
                .and_then(|c| c.modified)
                .or_else(|| std::fs::metadata(path).and_then(|m| m.modified()).ok())
        });
        let stamp = if dirty {
            "尚未保存".to_string()
        } else {
            modified.map(full_time).unwrap_or_default()
        };
        let starred = active.as_ref().is_some_and(|p| self.starred.contains(p));
        let ext = active
            .as_deref()
            .and_then(|p| Path::new(p).extension())
            .map(|e| e.to_string_lossy().to_uppercase())
            .unwrap_or_else(|| "TYP".into());

        h_flex()
            .h(px(46.))
            .flex_none()
            .pl(px(MARGIN_W + 14.))
            .pr(px(14.))
            .gap_3()
            // The project well is a real menu, like Smartisan's folder picker.
            .child(
                bar_button("project-well", cx)
                    .compact()
                    .h(px(28.))
                    .px_2()
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(rgb(0xe6dfd3), 0.),
                        linear_color_stop(rgb(0xf1ebe1), 1.),
                    ))
                    .border_1()
                    .border_color(rgb(0xddd3c5))
                    .rounded(px(4.))
                    .shadow(vec![
                        inset(0., 1., 2., 0x6a4a2a33),
                        shadow(0., 1., 0., 0., 0xffffffcc),
                    ])
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(paper_glyph("files", 14., 0x9c8f7e))
                            .child(
                                div()
                                    .max_w(px(160.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(12.))
                                    .text_color(rgb(INK_SOFT))
                                    .child(project),
                            )
                            .child(paper_glyph("triangle", 12., 0xb5aa9c)),
                    )
                    .dropdown_menu(|m, _, _| {
                        m.menu("打开文件夹…", Box::new(OpenFolder))
                            .menu("打开文件…", Box::new(OpenFile))
                            .separator()
                            .menu("在资源管理器中显示项目", Box::new(RevealProject))
                            .menu("重新扫描项目", Box::new(Rescan))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.5))
                    .text_color(rgb(0xb0a598))
                    .when(active.is_some(), |d| {
                        d.child(format!("{stamp}  |  {} 字", self.char_count))
                    }),
            )
            .when(active.is_some(), |header| {
                header
                    .child(
                        div()
                            .id("sheet-star")
                            .p_0p5()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|d| d.bg(rgba(0x8a6a4014)))
                            .tooltip(move |w, cx| {
                                Tooltip::new(if starred {
                                    "取消置顶"
                                } else {
                                    "置顶此文稿"
                                })
                                .build(w, cx)
                            })
                            .child(skin(if starred { "star_on" } else { "star" }, 18., 18.))
                            .when_some(active.clone(), |star, path| {
                                star.on_click(move |_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(ToggleStarActive { path: path.clone() }),
                                        cx,
                                    )
                                })
                            }),
                    )
                    .child(
                        h_flex()
                            .px(px(6.))
                            .h(px(20.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(rgb(AMBER))
                            .bg(linear_gradient(
                                180.,
                                linear_color_stop(rgb(0xf7e6c6), 0.),
                                linear_color_stop(rgb(0xead3ab), 1.),
                            ))
                            .shadow(vec![shadow(0., 1., 0., 0., 0xffffffcc)])
                            .text_size(px(10.5))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xa8763f))
                            .child(ext),
                    )
            })
    }

    /// Margin strip, its double edge and the writing rules, kept in step
    /// with the editor's scroll position. Compact mode is plain paper.
    fn render_ruled_paper(&self) -> impl IntoElement {
        let editor = self.editor.clone();
        let compact = self.compact;
        let fallback_line = self.line_height();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                if compact {
                    return;
                }
                let state = editor.read(cx);
                let line_h = f32::from(state.line_height().unwrap_or(px(fallback_line)));
                let scroll_y = f32::from(state.scroll_offset().y);
                let top = bounds.origin.y;
                let left = bounds.origin.x;
                let height = bounds.size.height;

                window.paint_quad(fill(
                    Bounds::new(point(left, top), size(px(MARGIN_W), height)),
                    rgb(MARGIN),
                ));
                window.paint_quad(fill(
                    Bounds::new(point(left + px(MARGIN_W - 1.), top), size(px(1.), height)),
                    rgb(0xe0d7c9),
                ));
                window.paint_quad(fill(
                    Bounds::new(point(left + px(MARGIN_W), top), size(px(1.), height)),
                    rgba(0xffffffb3),
                ));
                // Each rule sits under a line of text.
                let phase = (EDITOR_TOP_PAD + scroll_y).rem_euclid(line_h);
                let mut y = phase - 1.;
                while y < f32::from(height) {
                    if y >= 0. {
                        window.paint_quad(fill(
                            Bounds::new(point(left, top + px(y)), size(bounds.size.width, px(1.))),
                            rgb(RULE),
                        ));
                    }
                    y += line_h;
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    fn render_notepad(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Two sheets peeking out under the top one make it read as a pad.
        let under = |inset: f32, drop: f32, color: u32| {
            div()
                .absolute()
                .left(px(inset))
                .right(px(inset))
                .top(px(drop))
                .bottom(px(-drop))
                .rounded(px(3.))
                .bg(rgb(color))
                .border_1()
                .border_color(rgba(0xb4966c40))
                .shadow(paper_shadow())
        };
        let mono = fonts(cx).mono.clone();
        let empty = self.active().is_none();

        v_flex()
            .size_full()
            .pt(px(8.))
            .pb(px(16.))
            .px(px(10.))
            .child(self.render_tabs(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(under(6., 7., 0xefe7d8))
                    .child(under(3., 3.5, 0xf5eee2))
                    .child(
                        v_flex()
                            .relative()
                            .size_full()
                            .rounded(px(3.))
                            .when(!empty, |s| s.rounded_tl(px(0.)))
                            .overflow_hidden()
                            .bg(rgb(PAPER))
                            .border_1()
                            .border_color(rgba(0xb4966c4d))
                            .shadow(lifted_shadow())
                            .child(
                                img("textures/paper.jpg")
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .size_full()
                                    .object_fit(ObjectFit::Cover),
                            )
                            .child(
                                div()
                                    .relative()
                                    .bg(linear_gradient(
                                        180.,
                                        linear_color_stop(rgba(0xfffdf8e6), 0.),
                                        linear_color_stop(rgba(0xfdfaf2cc), 1.),
                                    ))
                                    .child(self.render_sheet_header(cx)),
                            )
                            .child(div().relative().child(paper_divider()))
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_h_0()
                                    .child(self.render_ruled_paper())
                                    .when(!empty, |area| {
                                        area.child(
                                            div()
                                                .size_full()
                                                .pl(px(if self.compact {
                                                    4.
                                                } else {
                                                    MARGIN_W - 2.
                                                }))
                                                .child(
                                                    Editor::new(&self.editor)
                                                        .bordered(false)
                                                        .p_0()
                                                        .size_full()
                                                        .font_family(mono)
                                                        .text_size(px(self.font_px))
                                                        .line_height(px(self.line_height())),
                                                ),
                                        )
                                    })
                                    .when(empty, |area| {
                                        area.child(
                                            div()
                                                .absolute()
                                                .top_0()
                                                .left_0()
                                                .size_full()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .text_size(px(14.))
                                                .child(letterpress(
                                                    "从左侧选一份文稿，或按 Ctrl+O 打开文件",
                                                    0xb0a598,
                                                )),
                                        )
                                    }),
                            ),
                    ),
            )
    }

    // ── Preview column, status strip, overlays ──────────────

    fn render_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let preview = self.preview.read(cx);
        let zoom = (preview.zoom() * 100.).round() as i32;
        let fit = preview.fit_width();
        let zoom_button =
            |id: &'static str, glyph: &'static str, tip: &'static str, action: Box<dyn Action>| {
                div()
                    .id(id)
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|d| d.bg(rgba(0x8a6a401a)))
                    .active(|d| d.bg(rgba(0x8a6a402e)))
                    .tooltip(move |w, cx| Tooltip::new(tip).build(w, cx))
                    .child(paper_glyph(glyph, 13., INK_SOFT))
                    .on_click(move |_, w, cx| w.dispatch_action(action.boxed_clone(), cx))
            };
        div()
            .relative()
            .size_full()
            .child(self.preview.clone())
            // A small cream control resting on the desk.
            .child(
                h_flex()
                    .absolute()
                    .top(px(12.))
                    .right(px(16.))
                    .p(px(3.))
                    .gap(px(2.))
                    .rounded(px(6.))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(rgb(0xfffdf8), 0.),
                        linear_color_stop(rgb(0xf1e9da), 1.),
                    ))
                    .border_1()
                    .border_color(rgb(0xd4c5ae))
                    .shadow(vec![
                        shadow(0., 1., 2., 0., 0x5a3c1e4d),
                        inset(0., 1., 0., 0xffffffcc),
                    ])
                    .child(zoom_button(
                        "zoom-out",
                        "min",
                        "缩小  Ctrl+-",
                        Box::new(ZoomOut),
                    ))
                    .child(
                        inset_well()
                            .id("zoom-reset")
                            .h(px(22.))
                            .w(px(52.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_size(px(11.5))
                            .text_color(rgb(INK_SOFT))
                            .tooltip(|w, cx| Tooltip::new("实际大小  Ctrl+0").build(w, cx))
                            .child(format!("{zoom}%"))
                            .on_click(|_, w, cx| w.dispatch_action(Box::new(ZoomReset), cx)),
                    )
                    .child(zoom_button(
                        "zoom-in",
                        "plus",
                        "放大  Ctrl+=",
                        Box::new(ZoomIn),
                    ))
                    .child(
                        div()
                            .id("zoom-fit")
                            .h(px(26.))
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_size(px(11.5))
                            .text_color(rgb(if fit { 0xa8763f } else { INK_SOFT }))
                            .when(fit, |d| {
                                d.bg(rgba(0xe9cf9f55))
                                    .shadow(vec![inset(0., 1., 2., 0x6a4a2a33)])
                            })
                            .when(!fit, |d| d.hover(|d| d.bg(rgba(0x8a6a401a))))
                            .tooltip(|w, cx| Tooltip::new("适合宽度  Ctrl+9").build(w, cx))
                            .child("适宽")
                            .on_click(|_, w, cx| w.dispatch_action(Box::new(FitWidth), cx)),
                    ),
            )
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cursor = self.editor.read(cx).cursor_position();
        let (led, status_text) = match &self.status {
            _ if self.compiling => ("led_amber", "排版中…".to_string()),
            CompileStatus::Idle => ("led_amber", "就绪".to_string()),
            CompileStatus::Ok { ms } => ("led_green", format!("排版完成  {ms} ms")),
            CompileStatus::Failed { first, count } => {
                ("led_red", format!("{count} 处错误：{first}"))
            }
        };
        let pages = self.preview.read(cx).page_count();
        let dirty = self.ws.active_buffer().is_some_and(|b| b.is_dirty());
        let item = |text: String| div().child(embossed(text));

        walnut_bar(30.).child(
            h_flex()
                .relative()
                .size_full()
                .px_4()
                .gap_4()
                .text_size(px(12.))
                .child(skin(led, 12., 12.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(embossed(status_text)),
                )
                .when(self.active().is_some(), |bar| {
                    bar.child(item(if dirty { "未保存" } else { "已保存" }.into()))
                        .child(groove_v(14.))
                        .child(item(format!(
                            "行 {}  列 {}",
                            cursor.line + 1,
                            cursor.character + 1
                        )))
                        .child(groove_v(14.))
                        .child(item(format!("{} 字", self.char_count)))
                        .child(groove_v(14.))
                        .child(item(format!("{pages} 页")))
                }),
        )
    }

    fn render_notice(&self, text: String) -> impl IntoElement {
        // Bottom right, clear of the zoom control at the top of the preview.
        div().absolute().bottom(px(22.)).right(px(28.)).child(
            div()
                .relative()
                .max_w(px(420.))
                .pl(px(22.))
                .pr(px(16.))
                .py(px(10.))
                .rounded(px(2.))
                .bg(rgb(0xfff6d6))
                .border_1()
                .border_color(rgba(0xc8a96a66))
                .shadow(lifted_shadow())
                .text_size(px(13.))
                .text_color(rgb(INK))
                .child(text)
                .child(
                    skin("paperclip_v", 18., 36.)
                        .absolute()
                        .top(px(-14.))
                        .left(px(6.)),
                ),
        )
    }

    /// A modal sheet of paper over a dimmed desk.
    fn modal(content: impl IntoElement) -> Div {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x3c281433))
            .child(content)
    }

    fn render_unsaved_dialog(&self, action: &Pending, cx: &mut Context<Self>) -> impl IntoElement {
        let files = self.affected_dirty(action);
        let root = self.ws.root().map(Path::to_path_buf);
        let (title, go) = match action {
            Pending::Switch { .. } => ("切换项目前，要保存这些修改吗？", "保存并切换"),
            Pending::CloseTab(_) => ("关闭前，要保存这份文稿吗？", "保存并关闭"),
            Pending::CloseWindow => ("退出前，要保存这些修改吗？", "保存并退出"),
        };
        let mono = fonts(cx).mono.clone();
        Self::modal(
            v_flex()
                .id("unsaved-dialog")
                .track_focus(&self.dialog_focus)
                .relative()
                .w(px(460.))
                .rounded(px(3.))
                .bg(rgb(PAPER))
                .border_1()
                .border_color(rgba(0xb4966c4d))
                .shadow(lifted_shadow())
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => this.resolve_pending(None, window, cx),
                        "enter" => this.resolve_pending(Some(true), window, cx),
                        _ => {}
                    }
                }))
                .child(
                    h_flex()
                        .h(px(56.))
                        .pl(px(MARGIN_W + 8.))
                        .pr_5()
                        .text_size(px(15.))
                        .font_weight(FontWeight::BOLD)
                        .child(title),
                )
                .child(paper_divider())
                .children(files.iter().map(|path| {
                    let shown = root
                        .as_ref()
                        .and_then(|r| Path::new(path).strip_prefix(r).ok())
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| path.clone());
                    h_flex()
                        .h(px(ROW_H))
                        .pl(px(MARGIN_W + 8.))
                        .pr_5()
                        .gap_2()
                        .border_b_1()
                        .border_color(rgb(RULE))
                        .child(div().size(px(7.)).rounded_full().bg(rgb(0xc8763a)))
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_family(mono.clone())
                                .text_size(px(13.))
                                .text_color(rgb(INK_SOFT))
                                .child(shown),
                        )
                }))
                .child(
                    h_flex()
                        .p_4()
                        .gap_2()
                        .child(
                            paper_button("unsaved-discard", "", "不保存").on_click(cx.listener(
                                |this, _, window, cx| this.resolve_pending(Some(false), window, cx),
                            )),
                        )
                        .child(div().flex_1())
                        .child(
                            paper_button("unsaved-cancel", "", "取消").on_click(cx.listener(
                                |this, _, window, cx| this.resolve_pending(None, window, cx),
                            )),
                        )
                        .child(
                            paper_button("unsaved-save", "disk", go)
                                .border_color(rgb(AMBER))
                                .text_color(rgb(0x8a5a2b))
                                .font_weight(FontWeight::SEMIBOLD)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.resolve_pending(Some(true), window, cx)
                                })),
                        ),
                ),
        )
    }

    fn render_about(&self, cx: &App) -> impl IntoElement {
        let rows = [
            ("Ctrl+O / Ctrl+Shift+O", "打开文件 / 文件夹"),
            ("Ctrl+S", "保存"),
            ("Ctrl+W", "关闭当前文稿"),
            ("F5", "重新编译"),
            ("Ctrl+B", "显示 / 隐藏文稿栏"),
            ("Ctrl+Shift+E", "导出 PDF"),
            ("Ctrl+= / Ctrl+-", "放大 / 缩小预览"),
            ("Ctrl+0 / Ctrl+9", "实际大小 / 适合宽度"),
        ];
        let mono = fonts(cx).mono.clone();
        Self::modal(
            v_flex()
                .relative()
                .w(px(440.))
                .rounded(px(3.))
                .bg(rgb(PAPER))
                .border_1()
                .border_color(rgba(0xb4966c4d))
                .shadow(lifted_shadow())
                .child(
                    h_flex()
                        .h(px(56.))
                        .px_5()
                        .gap_3()
                        .child(skin("notebook", 30., 30.))
                        .child(
                            v_flex()
                                .child(
                                    div()
                                        .text_size(px(17.))
                                        .font_weight(FontWeight::BOLD)
                                        .child("字笺 · Typst"),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .text_color(rgb(FAINT))
                                        .child("一张稿纸，一桌木纹，把想法排成作品。"),
                                ),
                        ),
                )
                .child(paper_divider())
                .children(rows.into_iter().map(|(key, what)| {
                    h_flex()
                        .h(px(ROW_H))
                        .px_5()
                        .border_b_1()
                        .border_color(rgb(RULE))
                        .text_size(px(13.))
                        .child(
                            div()
                                .w(px(190.))
                                .font_family(mono.clone())
                                .text_color(rgb(0x9a4f2c))
                                .child(key),
                        )
                        .child(div().text_color(rgb(INK_SOFT)).child(what))
                }))
                .child(
                    h_flex().p_4().justify_end().child(
                        paper_button("close-about", "", "收起")
                            .on_click(|_, w, cx| w.dispatch_action(Box::new(About), cx)),
                    ),
                ),
        )
    }

    fn render_empty_state(&self) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(skin("notebook", 112., 112.))
            .child(
                div()
                    .mt_2()
                    .text_size(px(36.))
                    .font_weight(FontWeight::BOLD)
                    .child(letterpress("字笺", 0x6e5340)),
            )
            .child(div().text_size(px(14.)).child(letterpress(
                "一张稿纸，一桌木纹，把想法排成作品。",
                WOOD_INK,
            )))
            .child(
                h_flex()
                    .mt_5()
                    .gap_3()
                    .child(
                        paper_button("open-file", "compose", "打开文件")
                            .on_click(|_, w, cx| w.dispatch_action(Box::new(OpenFile), cx)),
                    )
                    .child(
                        paper_button("open-folder", "folder", "打开文件夹")
                            .on_click(|_, w, cx| w.dispatch_action(Box::new(OpenFolder), cx)),
                    )
                    .child(
                        paper_button("open-demo", "files", "排版示例")
                            .on_click(|_, w, cx| w.dispatch_action(Box::new(OpenDemo), cx)),
                    ),
            )
    }
}

/// Toggle the star of the open document (sheet header).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
struct ToggleStarActive {
    path: String,
}

impl Render for TypstEditorApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = if self.ws.root().is_none() && self.active().is_none() {
            self.render_empty_state().into_any_element()
        } else {
            gpui_kit::base::h_resizable(if self.show_sidebar {
                "desk-with-cards"
            } else {
                "desk-focus"
            })
            .when(self.show_sidebar, |group| {
                group.child(
                    gpui_kit::base::resizable_panel()
                        .size(px(276.))
                        .size_range(px(220.)..px(460.))
                        .child(self.render_sidebar(cx)),
                )
            })
            .child(
                gpui_kit::base::resizable_panel()
                    .size(px(560.))
                    .size_range(px(320.)..px(2400.))
                    .child(self.render_notepad(cx)),
            )
            .child(
                gpui_kit::base::resizable_panel()
                    .size_range(px(280.)..px(3000.))
                    .child(self.render_preview(cx)),
            )
            .into_any_element()
        };
        let serif = fonts(cx).serif.clone();
        let pending = self.pending.clone();

        v_flex()
            .id("typst-editor")
            .size_full()
            .font_family(serif)
            .text_size(px(14.))
            .text_color(rgb(INK))
            .bg(rgb(0xf0ddbc))
            .on_action(cx.listener(Self::on_open_file))
            .on_action(cx.listener(Self::on_open_folder))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_export_pdf))
            .on_action(cx.listener(|this, action: &OpenDocument, window, cx| {
                this.open_document(action.path.clone(), window, cx)
            }))
            .on_action(cx.listener(|this, action: &ToggleStarActive, _, cx| {
                this.toggle_star(action.path.clone(), cx)
            }))
            .on_action(cx.listener(|this, _: &UndoEdit, window, cx| {
                this.editor.update(cx, |s, cx| s.focus(window, cx));
                window.dispatch_action(Box::new(gpui_kit::component::input::Undo), cx);
            }))
            .on_action(cx.listener(|this, _: &RedoEdit, window, cx| {
                this.editor.update(cx, |s, cx| s.focus(window, cx));
                window.dispatch_action(Box::new(gpui_kit::component::input::Redo), cx);
            }))
            .on_action(cx.listener(|this, _: &Recompile, _, cx| {
                this.schedule_compile(Duration::ZERO, false, cx)
            }))
            .on_action(cx.listener(|this, _: &Rescan, _, cx| this.rescan(cx)))
            .on_action(cx.listener(|this, _: &RevealProject, _, _| {
                if let Some(root) = this.ws.root() {
                    reveal_in_file_manager(root);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.show_sidebar = !this.show_sidebar;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleOutline, _, cx| {
                this.show_sidebar = true;
                this.outline_open = !this.outline_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleCompact, window, cx| {
                this.set_compact(!this.compact, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FontBigger, _, cx| {
                this.font_px = (this.font_px + 1.).min(24.);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FontSmaller, _, cx| {
                this.font_px = (this.font_px - 1.).max(11.);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FontReset, _, cx| {
                this.font_px = DEFAULT_FONT;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenDemo, window, cx| this.open_demo(window, cx)))
            .on_action(cx.listener(|this, _: &About, _, cx| {
                this.show_about = !this.show_about;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(Some(1.1), cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(Some(1. / 1.1), cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.zoom_by(None, cx)))
            .on_action(cx.listener(|this, _: &FitWidth, _, cx| {
                this.preview.update(cx, |p, cx| p.set_fit_width(cx));
                cx.notify();
            }))
            .child(self.render_titlebar(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(desk())
                    .child(div().relative().size_full().child(body))
                    .child(bar_cast_shadow(true).top_0())
                    .child(bar_cast_shadow(false).bottom_0())
                    .when_some(self.notice.clone(), |desk, text| {
                        desk.child(self.render_notice(text))
                    })
                    .when(self.show_about, |desk| desk.child(self.render_about(cx)))
                    .when_some(pending, |desk, action| {
                        desk.child(self.render_unsaved_dialog(&action, cx))
                    }),
            )
            .child(self.render_status_bar(cx))
    }
}

// ── Helpers ─────────────────────────────────────────────────

fn count_chars(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

fn is_typst(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("typ"))
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// Smartisan-style relative stamp: "今天 08:12", "昨天 23:19",
/// "13天前 10/03 12:29", or a full date for older notes.
fn smartisan_time(time: SystemTime) -> String {
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

fn full_time(time: SystemTime) -> String {
    let then: chrono::DateTime<chrono::Local> = time.into();
    then.format("%Y/%m/%d %H:%M").to_string()
}

/// Show `path` in the platform file manager.
fn reveal_in_file_manager(path: &Path) {
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
fn scan_project(root: &Path) -> (Vec<DocCard>, Vec<TreeNode>, usize) {
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
            if e.name.ends_with(".typ") {
                let text = std::fs::read_to_string(path).unwrap_or_default();
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
        let item = chapters.to_item(0, "design");
        assert!(item.is_some());
        assert!(chapters.to_item(0, "nothing-matches").is_none());
    }
}
