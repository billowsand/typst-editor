//! Root view: file tree | editor | preview, plus status bar.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_kit::component::input::{Editor, EditorState, InputEvent, RopeExt, TabSize};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tree::{TreeItem, TreeState, tree};
use gpui_kit::component::{ActiveTheme, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use typst_core::{CompileResult, CompilerState};

use crate::highlight;
use crate::lsp::{ActivePath, TypstLsp};
use crate::preview::{PreviewEvent, PreviewPanel};

actions!(typst_editor, [OpenFile, OpenFolder, Save, ZoomIn, ZoomOut, ZoomReset]);

/// Debounce between the last keystroke and a compile.
const COMPILE_DEBOUNCE: Duration = Duration::from_millis(120);
/// Debounce between cursor movement and the preview marker update.
const CURSOR_DEBOUNCE: Duration = Duration::from_millis(60);
/// Directories never shown in the file tree.
const IGNORED_DIRS: &[&str] = &["target", "node_modules"];

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-o", OpenFile, None),
        KeyBinding::new("secondary-shift-o", OpenFolder, None),
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-=", ZoomIn, None),
        KeyBinding::new("secondary--", ZoomOut, None),
        KeyBinding::new("secondary-0", ZoomReset, None),
    ]);
}

struct Buffer {
    saved: String,
    current: String,
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
    tree_state: Entity<TreeState>,
    project_root: Option<PathBuf>,
    active_path: ActivePath,
    buffers: HashMap<String, Buffer>,
    dirty: bool,
    last_cursor: usize,
    compiling: bool,
    status: CompileStatus,
    compile_task: Task<()>,
    cursor_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl TypstEditorApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let compiler = Arc::new(CompilerState::new());
        let active_path: ActivePath = Rc::default();

        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(highlight::LANGUAGE)
                .line_number(true)
                .soft_wrap(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    hard_tabs: false,
                })
                .placeholder("打开一个 .typ 文件开始编辑")
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
            cx.subscribe_in(&preview, window, |this, _, event: &PreviewEvent, window, cx| {
                let PreviewEvent::JumpToSource { file, offset } = event;
                this.jump_to(file.clone(), *offset, window, cx);
            }),
        ];

        Self {
            compiler,
            editor,
            preview,
            tree_state,
            project_root: None,
            active_path,
            buffers: HashMap::new(),
            dirty: false,
            last_cursor: 0,
            compiling: false,
            status: CompileStatus::Idle,
            compile_task: Task::ready(()),
            cursor_task: Task::ready(()),
            _subscriptions,
        }
    }

    fn active(&self) -> Option<String> {
        self.active_path.borrow().clone()
    }

    // ── Opening files ───────────────────────────────────────

    /// Open a `.typ` file or a project folder.
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        if path.is_dir() {
            self.set_project_root(path.clone(), cx);
            if let Some(main) = find_main_file(&path) {
                self.open_document(main.display().to_string(), window, cx);
            }
        } else {
            if self.project_root.is_none() {
                let root = typst_core::discover_project_root(&path)
                    .or_else(|| path.parent().map(Path::to_path_buf));
                if let Some(root) = root {
                    self.set_project_root(root, cx);
                }
            }
            self.open_document(path.display().to_string(), window, cx);
        }
    }

    fn set_project_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.compiler.reset_session();
        self.buffers.clear();
        self.project_root = Some(root.clone());
        let tree_state = self.tree_state.clone();
        cx.spawn(async move |_, cx| {
            // TreeItem is !Send, so scan off-thread and convert here.
            let nodes = cx
                .background_spawn(async move { scan_dir(&root, 0) })
                .await;
            let items: Vec<TreeItem> = nodes.into_iter().map(TreeNode::into_item).collect();
            tree_state.update(cx, |state, cx| state.set_items(items, cx));
        })
        .detach();
    }

    fn open_document(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.active().as_deref() == Some(path.as_str()) {
            return;
        }
        // Keep unsaved edits of the document we are leaving.
        if let Some(previous) = self.active() {
            let text = self.editor.read(cx).text().to_string();
            if let Some(buffer) = self.buffers.get_mut(&previous) {
                buffer.current = text;
            }
        }

        if !self.buffers.contains_key(&path) {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    self.buffers.insert(
                        path.clone(),
                        Buffer {
                            saved: text.clone(),
                            current: text,
                        },
                    );
                }
                Err(err) => {
                    self.status = CompileStatus::Failed {
                        first: format!("无法读取 {path}: {err}"),
                        count: 1,
                    };
                    cx.notify();
                    return;
                }
            }
        }
        let buffer = &self.buffers[&path];
        let text = buffer.current.clone();
        self.dirty = buffer.current != buffer.saved;

        *self.active_path.borrow_mut() = Some(path.clone());
        self.last_cursor = 0;
        self.editor.update(cx, |state, cx| {
            state.set_value(text, window, cx);
            state.focus(window, cx);
        });

        // The first document opened becomes the preview entry; later ones
        // route to whichever entry imports them.
        let make_entry = self.compiler.preview_entry().is_none();
        self.schedule_compile(Duration::ZERO, make_entry, cx);
        cx.notify();
    }

    fn jump_to(&mut self, file: String, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        let same = self
            .active()
            .is_some_and(|active| Path::new(&active) == Path::new(&file));
        if !same {
            self.open_document(file, window, cx);
        }
        self.editor.update(cx, |state, cx| {
            state.set_selected_range(offset..offset, cx);
            state.focus(window, cx);
        });
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
            prompt: Some(if directories { "打开文件夹" } else { "打开 Typst 文件" }.into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let path = paths.await.ok()?.ok()??.into_iter().next()?;
            this.update_in(cx, |this, window, cx| {
                if directories {
                    // Opening a folder starts a fresh session.
                    *this.active_path.borrow_mut() = None;
                } else {
                    this.project_root = None;
                }
                this.open_path(path, window, cx);
            })
            .ok()
        })
        .detach();
    }

    // ── Editing, saving ─────────────────────────────────────

    fn on_text_changed(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.active() else { return };
        let text = self.editor.read(cx).text().to_string();
        let Some(buffer) = self.buffers.get_mut(&path) else { return };
        if buffer.current == text {
            return;
        }
        buffer.current = text;
        self.dirty = buffer.current != buffer.saved;
        self.schedule_compile(COMPILE_DEBOUNCE, false, cx);
        cx.notify();
    }

    fn on_save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.active() else { return };
        let text = self.editor.read(cx).text().to_string();
        match std::fs::write(&path, &text) {
            Ok(()) => {
                if let Some(buffer) = self.buffers.get_mut(&path) {
                    buffer.saved = text.clone();
                    buffer.current = text;
                }
                self.dirty = false;
            }
            Err(err) => {
                self.status = CompileStatus::Failed {
                    first: format!("保存失败: {err}"),
                    count: 1,
                };
            }
        }
        cx.notify();
    }

    // ── Compile ─────────────────────────────────────────────

    /// Compile the active document after `delay`. Replacing the task cancels
    /// a compile that has not started yet, so only the latest text wins.
    fn schedule_compile(&mut self, delay: Duration, make_entry: bool, cx: &mut Context<Self>) {
        let Some(path) = self.active() else { return };
        let text = self.editor.read(cx).text().to_string();
        let compiler = self.compiler.clone();
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
            this.update(cx, |this, cx| this.apply_compile_result(result, sizes, cx))
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
                .filter(|d| Path::new(&d.file) == Path::new(&active))
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

    // ── Zoom ────────────────────────────────────────────────

    fn zoom_by(&mut self, factor: Option<f32>, cx: &mut Context<Self>) {
        self.preview.update(cx, |preview, cx| {
            let zoom = factor.map_or(1.0, |f| preview.zoom() * f);
            preview.set_zoom(zoom, cx);
        });
        cx.notify();
    }

    // ── Rendering ───────────────────────────────────────────

    fn render_file_tree(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let active = self.active();
        tree(&self.tree_state, move |ix, entry, _selected, _window, cx| {
            let item = entry.item();
            let icon = if !entry.is_folder() {
                IconName::File
            } else if entry.is_expanded() {
                IconName::FolderOpen
            } else {
                IconName::Folder
            };
            let is_active = active.as_deref() == Some(item.id.as_ref());
            let path = item.id.clone();
            let is_typ = !item.is_folder() && path.ends_with(".typ");
            let view = view.clone();

            ListItem::new(ix)
                .w_full()
                .rounded(cx.theme().radius)
                .py_0p5()
                .px_2()
                .pl(px(14.) * entry.depth() + px(8.))
                .when(is_active, |item| item.bg(cx.theme().accent))
                .when(!item.is_folder() && !is_typ, |item| {
                    item.text_color(cx.theme().muted_foreground)
                })
                .child(h_flex().gap_2().child(icon).child(item.label.clone()))
                .on_click(move |_, window, cx| {
                    if is_typ {
                        view.update(cx, |this, cx| {
                            this.open_document(path.to_string(), window, cx);
                        });
                    }
                })
        })
        .text_sm()
        .p_1()
        .size_full()
        .bg(cx.theme().sidebar)
        .text_color(cx.theme().sidebar_foreground)
    }

    fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(cx.theme().background)
            .child(
                div()
                    .text_xl()
                    .text_color(cx.theme().muted_foreground)
                    .child("Typst Editor · GPUI 原型"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("open-file")
                            .primary()
                            .icon(IconName::File)
                            .label("打开文件 (Ctrl+O)")
                            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenFile), cx)),
                    )
                    .child(
                        Button::new("open-folder")
                            .icon(IconName::FolderOpen)
                            .label("打开文件夹 (Ctrl+Shift+O)")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenFolder), cx)
                            }),
                    ),
            )
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let file_label = match self.active() {
            Some(path) => {
                let name = Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(path);
                if self.dirty { format!("● {name}") } else { name }
            }
            None => "未打开文件".into(),
        };
        let cursor = self.editor.read(cx).cursor_position();
        let (status_text, status_color) = match &self.status {
            _ if self.compiling => ("编译中…".to_string(), cx.theme().muted_foreground),
            CompileStatus::Idle => ("就绪".to_string(), cx.theme().muted_foreground),
            CompileStatus::Ok { ms } => (format!("✓ 编译 {ms} ms"), cx.theme().success),
            CompileStatus::Failed { first, count } => {
                (format!("✗ {count} 个错误: {first}"), cx.theme().danger)
            }
        };
        let preview = self.preview.read(cx);
        let pages = preview.page_count();
        let zoom = (preview.zoom() * 100.).round() as i32;

        StatusBar::new()
            .left(div().child(file_label))
            .left(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("行 {}, 列 {}", cursor.line + 1, cursor.character + 1)),
            )
            .left(
                div()
                    .max_w(px(640.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(status_color)
                    .child(status_text),
            )
            .right(div().child(format!("{pages} 页")))
            .right(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("zoom-out")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Minus)
                            .on_click(cx.listener(|this, _, _, cx| this.zoom_by(Some(1. / 1.1), cx))),
                    )
                    .child(
                        Button::new("zoom-reset")
                            .ghost()
                            .xsmall()
                            .label(format!("{zoom}%"))
                            .on_click(cx.listener(|this, _, _, cx| this.zoom_by(None, cx))),
                    )
                    .child(
                        Button::new("zoom-in")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _, _, cx| this.zoom_by(Some(1.1), cx))),
                    ),
            )
    }
}

impl Render for TypstEditorApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = if self.project_root.is_none() && self.active().is_none() {
            self.render_empty_state(cx).into_any_element()
        } else {
            h_resizable("main")
                .child(
                    resizable_panel()
                        .size(px(220.))
                        .child(self.render_file_tree(cx)),
                )
                .child(
                    resizable_panel().child(
                        Editor::new(&self.editor)
                            .bordered(false)
                            .p_0()
                            .size_full()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(cx.theme().mono_font_size),
                    ),
                )
                .child(resizable_panel().child(self.preview.clone()))
                .into_any_element()
        };

        v_flex()
            .id("typst-editor")
            .size_full()
            .font_family(cx.theme().font_family.clone())
            .text_size(cx.theme().font_size)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(Self::on_open_file))
            .on_action(cx.listener(Self::on_open_folder))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(Some(1.1), cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(Some(1. / 1.1), cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.zoom_by(None, cx)))
            .child(div().flex_1().min_h_0().child(body))
            .child(self.render_status_bar(cx))
    }
}

// ── Helpers ─────────────────────────────────────────────────

/// Plain (Send) mirror of a file tree entry, built on a background thread.
struct TreeNode {
    path: String,
    name: String,
    children: Option<Vec<TreeNode>>,
}

impl TreeNode {
    fn into_item(self) -> TreeItem {
        let item = TreeItem::new(self.path, self.name);
        match self.children {
            Some(children) => item.children(children.into_iter().map(TreeNode::into_item)),
            None => item,
        }
    }
}

/// Recursively list `dir` for the file tree (directories first).
fn scan_dir(dir: &Path, depth: usize) -> Vec<TreeNode> {
    const MAX_DEPTH: usize = 8;
    let Ok(entries) = typst_core::fs::list_dir(&dir.display().to_string()) else {
        return vec![];
    };
    entries
        .into_iter()
        .filter(|e| !(e.is_dir && IGNORED_DIRS.contains(&e.name.as_str())))
        .map(|e| TreeNode {
            children: (e.is_dir && depth < MAX_DEPTH)
                .then(|| scan_dir(Path::new(&e.path), depth + 1)),
            path: e.path,
            name: e.name,
        })
        .collect()
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
