//! Root view, dressed as a Smartisan Notes desk: a walnut title bar, a birch
//! desk holding paper cards (documents, outline), a ruled notepad (editor)
//! and the typeset pages (preview), and a walnut status strip.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_kit::component::input::{
    Editor, EditorState, Input, InputEvent, InputState, RopeExt, TabSize,
};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use typst_core::{CompileResult, CompilerState, OutlineItem};

use crate::highlight;
use crate::lsp::{ActivePath, TypstLsp};
use crate::preview::{PreviewEvent, PreviewPanel};
use crate::skin::*;
use crate::theme::*;

actions!(
    typst_editor,
    [
        OpenFile,
        OpenFolder,
        Save,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        Recompile,
        ExportPdf,
        ToggleSidebar,
        ToggleOutline,
        OpenDemo,
        UndoEdit,
        RedoEdit,
        About
    ]
);

/// Debounce between the last keystroke and a compile.
const COMPILE_DEBOUNCE: Duration = Duration::from_millis(120);
/// Debounce between cursor movement and the preview marker update.
const CURSOR_DEBOUNCE: Duration = Duration::from_millis(60);
/// Directories never scanned for documents.
const IGNORED_DIRS: &[&str] = &["target", "node_modules"];

/// Notepad geometry: the tinted margin strip and the writing line pitch.
const MARGIN_W: f32 = 44.;
const LINE_H: f32 = 30.;
/// Top padding the gpui-kit editor puts above its first line.
const EDITOR_TOP_PAD: f32 = 4.;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-o", OpenFile, None),
        KeyBinding::new("secondary-shift-o", OpenFolder, None),
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-=", ZoomIn, None),
        KeyBinding::new("secondary--", ZoomOut, None),
        KeyBinding::new("secondary-0", ZoomReset, None),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("secondary-shift-e", ExportPdf, None),
        KeyBinding::new("f5", Recompile, None),
    ]);
}

struct Buffer {
    saved: String,
    current: String,
}

/// A `.typ` file shown as a note card.
#[derive(Clone)]
struct DocCard {
    path: String,
    /// Path relative to the project root, with `/` separators.
    rel: String,
    /// First heading of the file, else its first line, else its name.
    title: String,
    modified: Option<SystemTime>,
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
    query: String,
    project_root: Option<PathBuf>,
    active_path: ActivePath,
    buffers: HashMap<String, Buffer>,
    cards: Vec<DocCard>,
    resources: usize,
    starred: HashSet<String>,
    dirty: bool,
    char_count: usize,
    last_cursor: usize,
    compiling: bool,
    status: CompileStatus,
    compile_task: Task<()>,
    cursor_task: Task<()>,
    notice_task: Task<()>,
    outline: Vec<OutlineItem>,
    show_sidebar: bool,
    show_outline: bool,
    show_about: bool,
    notice: Option<String>,
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
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索文稿与标题"));

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

        Self {
            compiler,
            editor,
            preview,
            search,
            query: String::new(),
            project_root: None,
            active_path,
            buffers: HashMap::new(),
            cards: Vec::new(),
            resources: 0,
            starred: HashSet::new(),
            dirty: false,
            char_count: 0,
            last_cursor: 0,
            compiling: false,
            status: CompileStatus::Idle,
            compile_task: Task::ready(()),
            cursor_task: Task::ready(()),
            notice_task: Task::ready(()),
            outline: Vec::new(),
            show_sidebar: true,
            show_outline: true,
            show_about: false,
            notice: None,
            _subscriptions,
        }
    }

    fn active(&self) -> Option<String> {
        self.active_path.borrow().clone()
    }

    fn is_active(&self, path: &str) -> bool {
        self.active()
            .is_some_and(|active| Path::new(&active) == Path::new(path))
    }

    fn is_entry(&self, path: &str) -> bool {
        self.compiler
            .preview_entry()
            .is_some_and(|entry| entry == Path::new(path))
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
        self.outline.clear();
        self.cards.clear();
        self.project_root = Some(root);
        self.rescan(cx);
    }

    /// Re-read the project's documents for the card list.
    fn rescan(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.project_root.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let (cards, resources) = cx.background_spawn(async move { scan_docs(&root) }).await;
            this.update(cx, |this, cx| {
                this.cards = cards;
                this.resources = resources;
                cx.notify();
            })
            .ok();
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
                    self.show_notice(format!("无法读取 {path}: {err}"), cx);
                    return;
                }
            }
        }
        let buffer = &self.buffers[&path];
        let text = buffer.current.clone();
        self.dirty = buffer.current != buffer.saved;
        self.char_count = count_chars(&text);

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

    /// Pin `path` as the document the preview compiles (the card's pushpin).
    fn pin_entry(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let text = match self.buffers.get(&path) {
            Some(buffer) => buffer.current.clone(),
            None => std::fs::read_to_string(&path).unwrap_or_default(),
        };
        let compiler = self.compiler.clone();
        let pinned = {
            let _guard = compiler.compile_lock();
            compiler.open(&path, &text).is_ok() && compiler.pin_preview_entry(Some(&path)).is_ok()
        };
        if pinned {
            self.open_document(path, window, cx);
            self.schedule_compile(Duration::ZERO, false, cx);
            self.show_notice("已将此文稿设为预览主文件", cx);
        }
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
        let Some(buffer) = self.buffers.get_mut(&path) else {
            return;
        };
        if buffer.current == text {
            return;
        }
        self.char_count = count_chars(&text);
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
                self.rescan(cx);
            }
            Err(err) => self.show_notice(format!("保存失败：{err}"), cx),
        }
        cx.notify();
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

    // ── Zoom, export, demo ──────────────────────────────────

    fn zoom_by(&mut self, factor: Option<f32>, cx: &mut Context<Self>) {
        self.preview.update(cx, |preview, cx| {
            let zoom = factor.map_or(1.0, |f| preview.zoom() * f);
            preview.set_zoom(zoom, cx);
        });
        cx.notify();
    }

    fn on_export_pdf(&mut self, _: &ExportPdf, window: &mut Window, cx: &mut Context<Self>) {
        if self.compiling {
            self.show_notice("正在编译，请稍后导出", cx);
            return;
        }
        let Some(entry) = self.compiler.preview_entry() else {
            return;
        };
        let path = PathBuf::from(entry);
        let name = path
            .with_extension("pdf")
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
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
        self.project_root = None;
        self.open_path(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
            window,
            cx,
        );
    }

    // ── Title bar ───────────────────────────────────────────

    /// Like Smartisan's folder title ("全部便签"): the project folder; the
    /// dropdown beside it switches between its documents.
    fn title_text(&self) -> String {
        self.project_root
            .as_ref()
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
        h_flex()
            .gap_0p5()
            .child(menu("menu-file", "文件").dropdown_menu(|m, _, _| {
                m.menu("打开文件…", Box::new(OpenFile))
                    .menu("打开文件夹…", Box::new(OpenFolder))
                    .separator()
                    .menu("保存", Box::new(Save))
                    .menu("导出 PDF…", Box::new(ExportPdf))
            }))
            .child(menu("menu-edit", "编辑").dropdown_menu(|m, _, _| {
                m.menu("撤销", Box::new(UndoEdit))
                    .menu("重做", Box::new(RedoEdit))
            }))
            .child(menu("menu-view", "视图").dropdown_menu(|m, _, _| {
                m.menu("显示 / 隐藏文稿栏", Box::new(ToggleSidebar))
                    .menu("显示 / 隐藏目录卡", Box::new(ToggleOutline))
                    .separator()
                    .menu("放大预览", Box::new(ZoomIn))
                    .menu("缩小预览", Box::new(ZoomOut))
                    .menu("实际大小", Box::new(ZoomReset))
            }))
            .child(menu("menu-doc", "文档").dropdown_menu(|m, _, _| {
                m.menu("重新编译", Box::new(Recompile))
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
                m.separator().menu("打开文件…", Box::new(OpenFile))
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

    // ── Sidebar: search, document cards, outline card ───────

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
                        .text_size(px(13.)),
                ),
            )
    }

    fn render_card(&self, card: &DocCard, ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.is_active(&card.path);
        let entry = self.is_entry(&card.path);
        let starred = self.starred.contains(&card.path);
        let dirty = self
            .buffers
            .get(&card.path)
            .is_some_and(|b| b.current != b.saved);
        let path = card.path.clone();
        let pin_path = card.path.clone();
        let star_path = card.path.clone();

        div()
            .id(("card", ix))
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
            .shadow(if active {
                lifted_shadow()
            } else {
                paper_shadow()
            })
            .cursor_pointer()
            .on_click(
                cx.listener(move |this, _, window, cx| {
                    this.open_document(path.clone(), window, cx)
                }),
            )
            // Binding strip with its double edge.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .h_full()
                    .w(px(26.))
                    .bg(rgb(if active { 0xf3e6cd } else { BAND }))
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
                    ),
            )
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
                            .child(
                                div()
                                    .id(("pin", ix))
                                    .p_0p5()
                                    .rounded_sm()
                                    .hover(|d| d.bg(rgba(0x8a6a4014)))
                                    .tooltip(|w, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new("设为预览主文件")
                                            .build(w, cx)
                                    })
                                    .child(skin(if entry { "pin_on" } else { "pin" }, 16., 16.))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.pin_entry(pin_path.clone(), window, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id(("star", ix))
                                    .p_0p5()
                                    .rounded_sm()
                                    .hover(|d| d.bg(rgba(0x8a6a4014)))
                                    .child(skin(if starred { "star_on" } else { "star" }, 16., 16.))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        if !this.starred.remove(&star_path) {
                                            this.starred.insert(star_path.clone());
                                        }
                                        cx.notify();
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
                                    .text_size(px(11.5))
                                    .text_color(rgb(FAINT))
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

    fn render_outline_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = v_flex().pl(px(36.)).pr(px(10.));
        let matches =
            |title: &str| self.query.is_empty() || title.to_lowercase().contains(&self.query);
        let visible: Vec<_> = self
            .outline
            .iter()
            .enumerate()
            .filter(|(_, item)| matches(&item.title))
            .collect();
        if visible.is_empty() {
            rows = rows.child(
                div()
                    .h(px(LINE_H))
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
                    .h(px(LINE_H))
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

        div()
            .relative()
            .flex_none()
            .ml(px(10.))
            .mr(px(2.))
            .rounded(px(2.))
            .bg(rgb(CARD))
            .border_1()
            .border_color(rgba(0xb4966c40))
            .shadow(paper_shadow())
            .pb_2()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .h_full()
                    .w(px(26.))
                    .bg(rgb(BAND))
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
                    ),
            )
            .child(
                h_flex()
                    .relative()
                    .h(px(30.))
                    .pl(px(36.))
                    .pr(px(10.))
                    .gap_2()
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
                    ),
            )
            .child(div().relative().pl(px(26.)).child(paper_divider()))
            .child(rows.relative())
            .child(
                skin("paperclip", 36., 18.)
                    .absolute()
                    .left(px(-15.))
                    .top(px(12.)),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let matches = |c: &&DocCard| {
            self.query.is_empty()
                || c.title.to_lowercase().contains(&self.query)
                || c.rel.to_lowercase().contains(&self.query)
        };
        let mut cards: Vec<&DocCard> = self.cards.iter().filter(matches).collect();
        // Starred first, then the order on disk.
        cards.sort_by_key(|c| !self.starred.contains(&c.path));

        let mut list = v_flex()
            .id("doc-cards")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pt(px(10.))
            .pb(px(16.))
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
                        row.child(letterpress(
                            format!("另有 {} 个资源", self.resources),
                            WOOD_INK,
                        ))
                    }),
            );
        for (ix, card) in cards.into_iter().enumerate() {
            list = list.child(self.render_card(card, ix, cx));
        }
        if self.show_outline {
            list = list
                .child(div().h(px(4.)))
                .child(self.render_outline_card(cx));
        }

        v_flex().size_full().child(self.render_search()).child(list)
    }

    // ── Notepad: index tabs, sheet header, ruled paper ──────

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut paths: Vec<_> = self.buffers.keys().cloned().collect();
        paths.sort();
        let mut tabs = h_flex().h(px(30.)).pl(px(18.)).gap(px(3.)).items_end();
        for (ix, path) in paths.into_iter().enumerate() {
            let selected = self.is_active(&path);
            let buffer = &self.buffers[&path];
            let name = Path::new(&path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let dirty = buffer.current != buffer.saved;
            tabs = tabs.child(
                h_flex()
                    .id(("tab", ix))
                    .relative()
                    .h(px(if selected { 30. } else { 26. }))
                    .px(px(14.))
                    .gap_1p5()
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
                    })
                    .text_size(px(12.5))
                    .text_color(rgb(if selected { INK } else { INK_SOFT }))
                    .when(selected, |t| t.font_weight(FontWeight::SEMIBOLD))
                    .cursor_pointer()
                    .when(dirty, |t| {
                        t.child(div().size(px(6.)).rounded_full().bg(rgb(0xc8763a)))
                    })
                    .child(name)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_document(path.clone(), window, cx)
                    })),
            );
        }
        tabs
    }

    fn render_sheet_header(&self) -> impl IntoElement {
        let project = self
            .project_root
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "全部文稿".into());
        let modified = self.active().and_then(|path| {
            self.cards
                .iter()
                .find(|c| Path::new(&c.path) == Path::new(&path))
                .and_then(|c| c.modified)
        });
        let stamp = if self.dirty {
            "尚未保存".to_string()
        } else {
            modified.map(full_time).unwrap_or_default()
        };
        let starred = self.active().is_some_and(|p| self.starred.contains(&p));

        h_flex()
            .h(px(46.))
            .flex_none()
            .pl(px(MARGIN_W + 14.))
            .pr(px(14.))
            .gap_3()
            .child(
                inset_well()
                    .id("folder-well")
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .cursor_pointer()
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
                    .child(paper_glyph("triangle", 12., 0xb5aa9c))
                    .on_click(|_, w, cx| w.dispatch_action(Box::new(ToggleSidebar), cx)),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.5))
                    .text_color(rgb(0xb9afa5))
                    .child(format!("{stamp}  |  {}", self.char_count)),
            )
            .child(skin(if starred { "star_on" } else { "star" }, 18., 18.))
            .child(
                h_flex()
                    .gap_1()
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
                    .child("TYP"),
            )
    }

    /// Margin strip, its double edge and the writing rules, kept in step
    /// with the editor's scroll position.
    fn render_ruled_paper(&self) -> impl IntoElement {
        let editor = self.editor.clone();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let state = editor.read(cx);
                let line_h = f32::from(state.line_height().unwrap_or(px(LINE_H)));
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
                            .rounded_tl(px(0.))
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
                                    .child(self.render_sheet_header()),
                            )
                            .child(div().relative().child(paper_divider()))
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_h_0()
                                    .child(self.render_ruled_paper())
                                    .child(
                                        div().size_full().pl(px(MARGIN_W - 2.)).child(
                                            Editor::new(&self.editor)
                                                .bordered(false)
                                                .p_0()
                                                .size_full()
                                                .font_family(MONO)
                                                .text_size(px(15.))
                                                .line_height(px(LINE_H)),
                                        ),
                                    ),
                            ),
                    ),
            )
    }

    // ── Preview column, status strip, overlays ──────────────

    fn render_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let zoom = (self.preview.read(cx).zoom() * 100.).round() as i32;
        let zoom_button = |id: &'static str, glyph: &'static str, action: Box<dyn Action>| {
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
                    .child(zoom_button("zoom-out", "min", Box::new(ZoomOut)))
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
                            .child(format!("{zoom}%"))
                            .on_click(|_, w, cx| w.dispatch_action(Box::new(ZoomReset), cx)),
                    )
                    .child(zoom_button("zoom-in", "plus", Box::new(ZoomIn))),
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
                    bar.child(item(if self.dirty { "未保存" } else { "已保存" }.into()))
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
        div().absolute().top(px(16.)).right(px(24.)).child(
            div()
                .relative()
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

    fn render_about(&self) -> impl IntoElement {
        let rows = [
            ("Ctrl+O", "打开文件"),
            ("Ctrl+Shift+O", "打开文件夹"),
            ("Ctrl+S", "保存"),
            ("F5", "重新编译"),
            ("Ctrl+B", "显示 / 隐藏文稿栏"),
            ("Ctrl+Shift+E", "导出 PDF"),
            ("Ctrl+= / Ctrl+-", "放大 / 缩小预览"),
            ("Ctrl+0", "实际大小"),
        ];
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x3c28141f))
            .child(
                v_flex()
                    .relative()
                    .w(px(420.))
                    .rounded(px(3.))
                    .bg(rgb(PAPER))
                    .border_1()
                    .border_color(rgba(0xb4966c4d))
                    .shadow(lifted_shadow())
                    .child(
                        h_flex()
                            .h(px(54.))
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
                            .h(px(LINE_H))
                            .px_5()
                            .border_b_1()
                            .border_color(rgb(RULE))
                            .text_size(px(13.))
                            .child(
                                div()
                                    .w(px(150.))
                                    .font_family(MONO)
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

/// Open a document from the title dropdown.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = typst_editor, no_json)]
struct OpenDocument {
    path: String,
}

impl Render for TypstEditorApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = if self.project_root.is_none() && self.active().is_none() {
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
                        .size(px(286.))
                        .size_range(px(220.)..px(460.))
                        .child(self.render_sidebar(cx)),
                )
            })
            .child(
                gpui_kit::base::resizable_panel()
                    .size(px(600.))
                    .size_range(px(320.)..px(2400.))
                    .child(self.render_notepad(cx)),
            )
            .child(
                gpui_kit::base::resizable_panel()
                    .size_range(px(300.)..px(3000.))
                    .child(self.render_preview(cx)),
            )
            .into_any_element()
        };

        v_flex()
            .id("typst-editor")
            .size_full()
            .font_family(SERIF)
            .text_size(px(14.))
            .text_color(rgb(INK))
            .bg(rgb(0xf0ddbc))
            .on_action(cx.listener(Self::on_open_file))
            .on_action(cx.listener(Self::on_open_folder))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_export_pdf))
            .on_action(cx.listener(|this, action: &OpenDocument, window, cx| {
                this.open_document(action.path.clone(), window, cx)
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
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.show_sidebar = !this.show_sidebar;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleOutline, _, cx| {
                this.show_sidebar = true;
                this.show_outline = !this.show_outline;
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
                    .when(self.show_about, |desk| desk.child(self.render_about())),
            )
            .child(self.render_status_bar(cx))
    }
}

// ── Helpers ─────────────────────────────────────────────────

fn count_chars(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

/// Smartisan-style relative stamp: "今天 08:12", "昨天 23:19",
/// "13天前 10/03 12:29", or a full date for older notes.
fn smartisan_time(time: SystemTime) -> String {
    use chrono::{DateTime, Local};
    let then: DateTime<Local> = time.into();
    let now = Local::now();
    let days = (now.date_naive() - then.date_naive()).num_days();
    match days {
        0 => format!("今天 {}", then.format("%H:%M")),
        1 => format!("昨天 {}", then.format("%H:%M")),
        2..=30 => format!("{days}天前 {}", then.format("%m/%d %H:%M")),
        _ => then.format("%Y/%m/%d").to_string(),
    }
}

fn full_time(time: SystemTime) -> String {
    let then: chrono::DateTime<chrono::Local> = time.into();
    then.format("%Y/%m/%d %H:%M").to_string()
}

/// Every `.typ` under `root` as a card, plus a count of other files.
fn scan_docs(root: &Path) -> (Vec<DocCard>, usize) {
    fn walk(root: &Path, dir: &Path, depth: usize, cards: &mut Vec<DocCard>, other: &mut usize) {
        let Ok(entries) = typst_core::fs::list_dir(&dir.display().to_string()) else {
            return;
        };
        for e in entries {
            let path = Path::new(&e.path);
            if e.is_dir {
                if depth < 8 && !IGNORED_DIRS.contains(&e.name.as_str()) {
                    walk(root, path, depth + 1, cards, other);
                }
            } else if e.name.ends_with(".typ") {
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
                    path: e.path,
                });
            } else {
                *other += 1;
            }
        }
    }
    let mut cards = Vec::new();
    let mut other = 0;
    walk(root, root, 0, &mut cards, &mut other);
    // Files at the top level first, then by path.
    cards.sort_by(|a, b| {
        a.rel
            .contains('/')
            .cmp(&b.rel.contains('/'))
            .then_with(|| a.rel.cmp(&b.rel))
    });
    (cards, other)
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
}
