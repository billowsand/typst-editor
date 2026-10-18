//! Paper dialogs (unsaved edits, conflicts, recovery, names, deletion,
//! about), the notice slip, the welcome desk, and the problems and project
//! search panels.

use std::path::Path;

use gpui_kit::component::input::Input;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::views::{MARGIN_W, ROW_H};
use super::*;
use crate::skin::*;
use crate::theme::*;

impl TypstEditorApp {
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
            // Clicks and scrolling must not reach the desk underneath.
            .occlude()
            .child(content)
    }

    fn sheet(&self, width: f32) -> Stateful<Div> {
        v_flex()
            .id("dialog-sheet")
            .track_focus(&self.dialog_focus)
            .relative()
            .w(px(width))
            .max_h(px(560.))
            .rounded(px(3.))
            .bg(rgb(PAPER))
            .border_1()
            .border_color(rgba(0xb4966c4d))
            .shadow(lifted_shadow())
    }

    fn dialog_title(title: impl Into<SharedString>) -> Div {
        h_flex()
            .h(px(56.))
            .flex_none()
            .pl(px(MARGIN_W + 8.))
            .pr_5()
            .text_size(px(15.))
            .font_weight(FontWeight::BOLD)
            .child(title.into())
    }

    fn file_rows(&self, files: &[String], cx: &App) -> Div {
        let root = self.ws.root().map(Path::to_path_buf);
        let mono = fonts(cx).mono.clone();
        v_flex().children(files.iter().map(|path| {
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
    }

    fn primary_button(id: &'static str, glyph: &str, label: &str) -> Stateful<Div> {
        paper_button(id, glyph, label)
            .border_color(rgb(AMBER))
            .text_color(rgb(0x8a5a2b))
            .font_weight(FontWeight::SEMIBOLD)
    }

    pub(super) fn render_pending(&self, action: &Pending, cx: &mut Context<Self>) -> AnyElement {
        match action {
            Pending::Switch { .. } | Pending::CloseTab(_) | Pending::CloseWindow => {
                self.render_unsaved_dialog(action, cx).into_any_element()
            }
            Pending::Conflict { paths, .. } => {
                self.render_conflict_dialog(paths, cx).into_any_element()
            }
            Pending::Recover(drafts) => self.render_recover_dialog(drafts, cx).into_any_element(),
            Pending::Rename { .. } | Pending::NewItem { .. } => {
                self.render_name_dialog(action, cx).into_any_element()
            }
            Pending::Delete { path } => self.render_delete_dialog(path, cx).into_any_element(),
        }
    }

    fn render_unsaved_dialog(&self, action: &Pending, cx: &mut Context<Self>) -> impl IntoElement {
        let files = self.affected_dirty(action);
        let (title, go) = match action {
            Pending::Switch { .. } => ("切换项目前，要保存这些修改吗？", "保存并切换"),
            Pending::CloseTab(_) => ("关闭前，要保存这份文稿吗？", "保存并关闭"),
            _ => ("退出前，要保存这些修改吗？", "保存并退出"),
        };
        Self::modal(
            self.sheet(460.)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => this.resolve_pending(None, window, cx),
                        "enter" => this.resolve_pending(Some(true), window, cx),
                        _ => {}
                    }
                }))
                .child(Self::dialog_title(title))
                .child(paper_divider())
                .child(self.file_rows(&files, cx))
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
                        .child(Self::primary_button("unsaved-save", "disk", go).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.resolve_pending(Some(true), window, cx)
                            }),
                        )),
                ),
        )
    }

    fn render_conflict_dialog(&self, paths: &[String], cx: &mut Context<Self>) -> impl IntoElement {
        Self::modal(
            self.sheet(500.)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.resolve_conflict(None, window, cx);
                    }
                }))
                .child(Self::dialog_title("这些文件已被其他程序修改"))
                .child(
                    div()
                        .pl(px(MARGIN_W + 8.))
                        .pr_5()
                        .pb_3()
                        .text_size(px(13.))
                        .text_color(rgb(INK_SOFT))
                        .child("保存会覆盖磁盘上的新内容。请选择保留哪一份。"),
                )
                .child(paper_divider())
                .child(self.file_rows(paths, cx))
                .child(
                    h_flex()
                        .p_4()
                        .gap_2()
                        .child(
                            paper_button("conflict-reload", "", "放弃我的修改，载入磁盘版本")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.resolve_conflict(Some(false), window, cx)
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            paper_button("conflict-cancel", "", "取消").on_click(cx.listener(
                                |this, _, window, cx| this.resolve_conflict(None, window, cx),
                            )),
                        )
                        .child(
                            Self::primary_button("conflict-overwrite", "disk", "用我的版本覆盖")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.resolve_conflict(Some(true), window, cx)
                                })),
                        ),
                ),
        )
    }

    fn render_recover_dialog(&self, drafts: &[Draft], cx: &mut Context<Self>) -> impl IntoElement {
        let mono = fonts(cx).mono.clone();
        let rows = drafts.iter().map(|d| {
            let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(d.written);
            h_flex()
                .h(px(ROW_H))
                .pl(px(MARGIN_W + 8.))
                .pr_5()
                .gap_3()
                .border_b_1()
                .border_color(rgb(RULE))
                .child(div().size(px(7.)).rounded_full().bg(rgb(0xc8763a)))
                .child(
                    div()
                        .flex_none()
                        .font_family(mono.clone())
                        .text_size(px(13.))
                        .text_color(rgb(INK))
                        .child(file_name(&d.path)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(11.5))
                        .text_color(rgb(PATH_INK))
                        .child(
                            Path::new(&d.path)
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default(),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(FAINT))
                        .child(smartisan_time(when)),
                )
        });
        Self::modal(
            self.sheet(560.)
                .child(Self::dialog_title("发现上次未保存的草稿"))
                .child(
                    div()
                        .pl(px(MARGIN_W + 8.))
                        .pr_5()
                        .pb_3()
                        .text_size(px(13.))
                        .text_color(rgb(INK_SOFT))
                        .child(
                            "字笺上次没有正常退出。恢复后草稿以未保存状态打开，确认无误再保存。",
                        ),
                )
                .child(paper_divider())
                .child(
                    v_flex()
                        .id("draft-rows")
                        .overflow_y_scroll()
                        .max_h(px(260.))
                        .children(rows),
                )
                .child(
                    h_flex()
                        .p_4()
                        .gap_2()
                        .child(paper_button("recover-discard", "", "丢弃草稿").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.resolve_recovery(Some(false), window, cx)
                            }),
                        ))
                        .child(div().flex_1())
                        .child(
                            paper_button("recover-later", "", "稍后").on_click(cx.listener(
                                |this, _, window, cx| this.resolve_recovery(None, window, cx),
                            )),
                        )
                        .child(
                            Self::primary_button("recover-restore", "files", "恢复草稿").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.resolve_recovery(Some(true), window, cx)
                                }),
                            ),
                        ),
                ),
        )
    }

    fn render_name_dialog(&self, action: &Pending, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, go) = match action {
            Pending::Rename { path } => (format!("重命名 {}", file_name(path)), "重命名"),
            Pending::NewItem { folder: true, .. } => ("新建文件夹".to_string(), "创建"),
            _ => ("新建文稿".to_string(), "创建"),
        };
        Self::modal(
            self.sheet(440.)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        this.pending = None;
                        cx.notify();
                    }
                }))
                .child(Self::dialog_title(title))
                .child(
                    div().pl(px(MARGIN_W + 8.)).pr_5().pb_2().child(
                        inset_well().h(px(34.)).px_2().flex().items_center().child(
                            Input::new(&self.name_input)
                                .appearance(false)
                                .small()
                                .text_size(px(14.)),
                        ),
                    ),
                )
                .child(
                    h_flex()
                        .p_4()
                        .gap_2()
                        .justify_end()
                        .child(
                            paper_button("name-cancel", "", "取消").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.pending = None;
                                    cx.notify();
                                },
                            )),
                        )
                        .child(Self::primary_button("name-ok", "", go).on_click(
                            cx.listener(|this, _, window, cx| this.confirm_name(window, cx)),
                        )),
                ),
        )
    }

    fn render_delete_dialog(&self, path: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.ws.paths_under(path);
        let dirty: Vec<String> = open
            .iter()
            .filter(|p| self.ws.buffer(p).is_some_and(|b| b.is_dirty()))
            .cloned()
            .collect();
        let target = path.to_string();
        Self::modal(
            self.sheet(480.)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        this.pending = None;
                        cx.notify();
                    }
                }))
                .child(Self::dialog_title(format!("把 {} 移到回收站？", file_name(path))))
                .child(
                    div()
                        .pl(px(MARGIN_W + 8.))
                        .pr_5()
                        .pb_3()
                        .text_size(px(13.))
                        .text_color(rgb(INK_SOFT))
                        .child(if dirty.is_empty() {
                            "可以从系统回收站还原。".to_string()
                        } else {
                            format!(
                                "其中 {} 个打开的文稿有未保存的修改，这些修改会丢失。可以从系统回收站还原文件。",
                                dirty.len()
                            )
                        }),
                )
                .when(!dirty.is_empty(), |s| s.child(paper_divider()).child(self.file_rows(&dirty, cx)))
                .child(
                    h_flex()
                        .p_4()
                        .gap_2()
                        .justify_end()
                        .child(paper_button("delete-cancel", "", "取消").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.pending = None;
                                cx.notify();
                            },
                        )))
                        .child(
                            paper_button("delete-ok", "", "移到回收站")
                                .border_color(rgb(0xb5402f))
                                .text_color(rgb(0x9a3324))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.delete_path(target.clone(), window, cx)
                                })),
                        ),
                ),
        )
    }

    pub(super) fn render_notice(&self, text: String) -> impl IntoElement {
        // Bottom right, clear of the zoom control at the top of the preview.
        div()
            .absolute()
            .bottom(px(22.))
            .right(px(28.))
            .occlude()
            .child(
                div()
                    .relative()
                    .max_w(px(460.))
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

    pub(super) fn render_about(&self, cx: &App) -> impl IntoElement {
        let rows = [
            ("Ctrl+N", "新建文稿"),
            ("Ctrl+O / Ctrl+Shift+O", "打开文件 / 文件夹"),
            ("Ctrl+S / Ctrl+Shift+S", "保存 / 另存为"),
            ("Ctrl+Alt+S", "全部保存"),
            ("Ctrl+W", "关闭当前文稿"),
            ("Ctrl+F / Ctrl+H", "查找 / 替换"),
            ("Ctrl+Shift+F", "在项目中查找与替换"),
            ("Ctrl+Shift+M", "问题列表"),
            ("F5", "强制重新编译"),
            ("Ctrl+B", "显示 / 隐藏文稿栏"),
            ("Ctrl+Shift+E", "导出 PDF"),
            ("Ctrl+= / Ctrl+-", "放大 / 缩小预览"),
            ("Ctrl+0 / Ctrl+9", "实际大小 / 适合宽度"),
        ];
        let mono = fonts(cx).mono.clone();
        Self::modal(
            v_flex()
                .relative()
                .w(px(460.))
                .rounded(px(3.))
                .bg(rgb(PAPER))
                .border_1()
                .border_color(rgba(0xb4966c4d))
                .shadow(lifted_shadow())
                .child(
                    h_flex()
                        .h(px(60.))
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
                                .child(div().text_size(px(11.5)).text_color(rgb(FAINT)).child(
                                    format!(
                                        "版本 {} · {} · {}",
                                        logging::VERSION,
                                        logging::COMMIT,
                                        logging::BUILD_DATE
                                    ),
                                )),
                        ),
                )
                .child(paper_divider())
                .children(rows.into_iter().map(|(key, what)| {
                    h_flex()
                        .h(px(28.))
                        .px_5()
                        .border_b_1()
                        .border_color(rgb(RULE))
                        .text_size(px(13.))
                        .child(
                            div()
                                .w(px(200.))
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

    pub(super) fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let recent: Vec<String> = self
            .recent_projects()
            .into_iter()
            .filter(|r| Path::new(r).is_dir())
            .take(5)
            .collect();
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
            // Direct listeners: before anything has focus, dispatched
            // actions would not reach this view.
            .child(
                h_flex()
                    .mt_5()
                    .gap_3()
                    .child(
                        paper_button("welcome-new", "compose", "新建项目").on_click(
                            cx.listener(|this, _, window, cx| this.new_project(window, cx)),
                        ),
                    )
                    .child(
                        paper_button("welcome-open", "folder", "打开文件夹").on_click(
                            cx.listener(|this, _, window, cx| this.prompt_open(true, window, cx)),
                        ),
                    )
                    .child(paper_button("welcome-file", "files", "打开文件").on_click(
                        cx.listener(|this, _, window, cx| this.prompt_open(false, window, cx)),
                    ))
                    .child(
                        paper_button("welcome-demo", "doc", "排版示例").on_click(
                            cx.listener(|this, _, window, cx| this.open_demo(window, cx)),
                        ),
                    ),
            )
            .when(!recent.is_empty(), |col| {
                col.child(
                    v_flex()
                        .mt_5()
                        .w(px(420.))
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(12.))
                                .child(letterpress("最近项目", WOOD_INK)),
                        )
                        .children(recent.into_iter().enumerate().map(|(ix, root)| {
                            let name = Path::new(&root)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| root.clone());
                            let open = root.clone();
                            h_flex()
                                .id(("recent", ix))
                                .h(px(34.))
                                .px_3()
                                .gap_3()
                                .rounded(px(3.))
                                .cursor_pointer()
                                .bg(rgba(0xfffaf099))
                                .border_1()
                                .border_color(rgba(0xb4966c33))
                                .hover(|d| d.bg(rgb(0xfffaf0)).shadow(paper_shadow()))
                                .child(paper_glyph("folder", 14., 0xb08850))
                                .child(div().text_size(px(13.5)).text_color(rgb(INK)).child(name))
                                .child(
                                    div()
                                        .flex_1()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_size(px(11.5))
                                        .text_color(rgb(PATH_INK))
                                        .child(root),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_path(open.clone().into(), window, cx)
                                }))
                        })),
                )
            })
    }

    // ── Panels ──────────────────────────────────────────────

    /// A card that slides over the bottom of the desk.
    fn panel_card(&self, title: &str, panel: Panel, cx: &mut Context<Self>) -> Stateful<Div> {
        v_flex()
            .id("panel-card")
            .occlude()
            .absolute()
            .left(px(24.))
            .right(px(24.))
            .bottom(px(18.))
            .h(px(320.))
            .rounded(px(3.))
            .bg(rgb(CARD))
            .border_1()
            .border_color(rgba(0xb4966c4d))
            .shadow(lifted_shadow())
            .child(
                h_flex()
                    .h(px(36.))
                    .flex_none()
                    .pl(px(36.))
                    .pr_2()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title.to_string()),
                    )
                    .child(
                        div()
                            .id("panel-close")
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|d| d.bg(rgba(0x8a6a401a)))
                            .child(paper_glyph("close", 12., INK_SOFT))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.panel == Some(panel) {
                                    this.panel = None;
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(paper_divider())
            .child(
                skin("paperclip_v", 18., 36.)
                    .absolute()
                    .top(px(-14.))
                    .left(px(12.)),
            )
    }

    pub(super) fn render_panel(&self, panel: Panel, cx: &mut Context<Self>) -> AnyElement {
        match panel {
            Panel::Problems => self.render_problems(cx).into_any_element(),
            Panel::Search => self.render_search_panel(cx).into_any_element(),
        }
    }

    fn render_problems(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let errors = self.problems.iter().filter(|p| p.is_error).count();
        let title = format!(
            "问题列表  ·  {errors} 个错误，{} 个警告",
            self.problems.len() - errors
        );
        let root = self.ws.root().map(Path::to_path_buf);
        let mono = fonts(cx).mono.clone();
        let mut rows = v_flex()
            .id("problem-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if self.problems.is_empty() {
            rows = rows.child(
                div()
                    .p_4()
                    .pl(px(36.))
                    .text_size(px(13.))
                    .text_color(rgb(FAINT))
                    .child("没有问题。文稿可以正常排版。"),
            );
        }
        for (ix, p) in self.problems.iter().enumerate() {
            let shown = root
                .as_ref()
                .and_then(|r| Path::new(&p.file).strip_prefix(r).ok())
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| p.file.clone());
            let (file, offset) = (p.file.clone(), p.offset);
            rows = rows.child(
                h_flex()
                    .id(("problem", ix))
                    .min_h(px(ROW_H))
                    .pl(px(36.))
                    .pr_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(RULE))
                    .cursor_pointer()
                    .hover(|d| d.bg(rgba(0xe9cf9f22)))
                    .child(skin(
                        if p.is_error { "led_red" } else { "led_amber" },
                        10.,
                        10.,
                    ))
                    .child(
                        div()
                            .w(px(220.))
                            .flex_none()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family(mono.clone())
                            .text_size(px(12.))
                            .text_color(rgb(PATH_INK))
                            .child(format!("{shown}:{}:{}", p.line, p.col)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.))
                            .text_color(rgb(if p.is_error { 0x9a3324 } else { INK_SOFT }))
                            .child(p.message.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.jump_to(file.clone(), offset, window, cx)
                    })),
            );
        }
        self.panel_card(&title, Panel::Problems, cx).child(rows)
    }

    fn render_search_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = fonts(cx).mono.clone();
        let root = self.ws.root().map(Path::to_path_buf);
        let mut file_count = self
            .search_hits
            .iter()
            .map(|h| h.path.as_str())
            .collect::<Vec<_>>();
        file_count.dedup();
        let summary = if self.search_running {
            "查找中…".to_string()
        } else if self.search_hits.is_empty() {
            "输入后按 Enter 查找；替换只修改编辑器中的文本，保存前可撤销".to_string()
        } else {
            format!(
                "{} 处匹配，{} 个文件",
                self.search_hits.len(),
                file_count.len()
            )
        };
        let mut rows = v_flex()
            .id("search-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        let mut last_file = String::new();
        for (ix, hit) in self.search_hits.iter().enumerate() {
            if hit.path != last_file {
                last_file = hit.path.clone();
                let shown = root
                    .as_ref()
                    .and_then(|r| Path::new(&hit.path).strip_prefix(r).ok())
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| hit.path.clone());
                rows = rows.child(
                    div()
                        .pl(px(36.))
                        .pt_2()
                        .pb_1()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(PATH_INK))
                        .child(shown),
                );
            }
            let (path, range) = (hit.path.clone(), hit.start..hit.end);
            let m = hit.snippet_match.clone();
            let snippet = &hit.snippet;
            let safe = |i: usize| {
                (0..=i.min(snippet.len()))
                    .rev()
                    .find(|j| snippet.is_char_boundary(*j))
                    .unwrap_or(0)
            };
            let (a, b) = (safe(m.start), safe(m.end));
            rows = rows.child(
                h_flex()
                    .id(("hit", ix))
                    .h(px(26.))
                    .pl(px(48.))
                    .pr_3()
                    .gap_3()
                    .cursor_pointer()
                    .hover(|d| d.bg(rgba(0xe9cf9f22)))
                    .child(
                        div()
                            .w(px(44.))
                            .flex_none()
                            .text_size(px(11.5))
                            .text_color(rgb(FAINT))
                            .child(format!("{}", hit.line)),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .font_family(mono.clone())
                            .text_size(px(12.5))
                            .text_color(rgb(INK_SOFT))
                            .child(snippet[..a].to_string())
                            .child(
                                div()
                                    .bg(rgba(0xe9b95a66))
                                    .text_color(rgb(INK))
                                    .child(snippet[a..b].to_string()),
                            )
                            .child(snippet[b..].to_string()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_range(path.clone(), range.clone(), window, cx)
                    })),
            );
        }
        let has_hits = !self.search_hits.is_empty();
        self.panel_card("在项目中查找与替换", Panel::Search, cx)
            .child(
                h_flex()
                    .flex_none()
                    .pl(px(36.))
                    .pr_3()
                    .py_2()
                    .gap_2()
                    .child(
                        inset_well()
                            .flex_1()
                            .h(px(30.))
                            .px_2()
                            .flex()
                            .items_center()
                            .child(
                                Input::new(&self.find_input)
                                    .appearance(false)
                                    .small()
                                    .text_size(px(13.)),
                            ),
                    )
                    .child(
                        inset_well()
                            .flex_1()
                            .h(px(30.))
                            .px_2()
                            .flex()
                            .items_center()
                            .child(
                                Input::new(&self.replace_input)
                                    .appearance(false)
                                    .small()
                                    .text_size(px(13.)),
                            ),
                    )
                    .child(
                        paper_button("search-go", "search", "查找")
                            .h(px(30.))
                            .on_click(cx.listener(|this, _, _, cx| this.run_project_search(cx))),
                    )
                    .child(
                        paper_button("search-replace", "", "全部替换")
                            .h(px(30.))
                            .when(!has_hits, |b| b.opacity(0.5))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if !this.search_hits.is_empty() {
                                    this.replace_in_project(window, cx)
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .pl(px(36.))
                    .pb_1()
                    .text_size(px(11.5))
                    .text_color(rgb(FAINT))
                    .child(summary),
            )
            .child(paper_divider())
            .child(rows)
    }

    /// Open the user guide that ships beside the executable (or in the
    /// source tree during development).
    pub(super) fn open_guide(&mut self, cx: &mut Context<Self>) {
        let guide = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("docs").join("用户指南.md")))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("docs")
                    .join("用户指南.md")
            });
        if guide.is_file() {
            cx.open_url(&format!(
                "file:///{}",
                guide.display().to_string().replace('\\', "/")
            ));
        } else {
            self.show_notice("找不到用户指南文件", cx);
        }
    }
}
