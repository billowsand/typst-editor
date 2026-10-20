//! The desk: walnut title bar, sidebar (search, cards or file tree, outline),
//! the notepad with its banners, the preview column and the status strip.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gpui_kit::component::input::{Editor, Input};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::tree::tree;
use gpui_kit::component::{Sizable, h_flex, v_flex};
use gpui_kit::base::InteractiveElementExt;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::*;
use crate::skin::*;
use crate::theme::*;

/// Notepad geometry: the tinted margin strip, and the top padding the
/// gpui-kit editor puts above its first line (the rules align to it).
pub(super) const MARGIN_W: f32 = 44.;
const EDITOR_TOP_PAD: f32 = 4.;
/// Height of a row on lined cards (outline, file tree, dialogs).
pub(super) const ROW_H: f32 = 30.;

/// Bound dynamic menu labels before layout; PopupMenu's outer max width alone
/// does not clip its default text item. Keep the full label available on hover.
fn path_menu_label(label: String, window: &Window) -> impl IntoElement + use<> {
    let width = (f32::from(window.viewport_size().width) - 64.).clamp(160., 440.);
    div()
        .id(SharedString::from(label.clone()))
        .w(px(width))
        .min_w_0()
        .truncate()
        .child(label.clone())
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
}

impl TypstEditorApp {
    // ── Title bar ───────────────────────────────────────────

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
        let recent = self.recent_projects();
        let has_doc = self.active().is_some();
        h_flex()
            .gap_0p5()
            .child(menu("menu-file", "文件").dropdown_menu(move |mut m, _, _| {
                m = m
                    .menu("新建文稿…", Box::new(NewDocument))
                    .menu("新建项目…", Box::new(NewProject))
                    .separator()
                    .menu("打开文件…", Box::new(OpenFile))
                    .menu("打开文件夹…", Box::new(OpenFolder));
                if !recent.is_empty() {
                    m = m.separator().label("最近项目");
                    for root in &recent {
                        let label = Path::new(root)
                            .file_name()
                            .map(|n| format!("{}    {root}", n.to_string_lossy()))
                            .unwrap_or_else(|| root.clone());
                        m = m.menu_element(
                            Box::new(OpenRecent { root: root.clone() }),
                            move |window, _| path_menu_label(label.clone(), window),
                        );
                    }
                }
                m.separator()
                    .menu_with_disabled("保存", Box::new(Save), !has_doc)
                    .menu_with_disabled("另存为…", Box::new(SaveAs), !has_doc)
                    .menu("全部保存", Box::new(SaveAll))
                    .menu_with_disabled("关闭当前文稿", Box::new(CloseTab), !has_doc)
                    .separator()
                    .menu("导出 PDF…", Box::new(ExportPdf))
            }))
            .child(menu("menu-edit", "编辑").dropdown_menu(move |m, _, _| {
                m.menu("撤销", Box::new(UndoEdit))
                    .menu("重做", Box::new(RedoEdit))
                    .separator()
                    .menu_with_disabled("查找…", Box::new(Find), !has_doc)
                    .menu_with_disabled("替换…", Box::new(FindReplace), !has_doc)
                    .menu("在项目中查找与替换…", Box::new(ProjectSearch))
            }))
            .child(menu("menu-view", "视图").dropdown_menu(move |m, _, _| {
                m.menu("显示 / 隐藏文稿栏", Box::new(ToggleSidebar))
                    .menu_with_check("目录卡", outline_open, Box::new(ToggleOutline))
                    .menu("问题列表", Box::new(ShowProblems))
                    .separator()
                    .menu_with_check("紧凑代码模式（行号）", compact, Box::new(ToggleCompact))
                    .menu("增大字号", Box::new(FontBigger))
                    .menu("减小字号", Box::new(FontSmaller))
                    .menu("默认字号", Box::new(FontReset))
                    .menu("设置…", Box::new(OpenSettings))
                    .separator()
                    .menu("预览适合宽度", Box::new(FitWidth))
                    .menu("预览实际大小", Box::new(ZoomReset))
                    .menu("放大预览", Box::new(ZoomIn))
                    .menu("缩小预览", Box::new(ZoomOut))
            }))
            .child(menu("menu-doc", "文档").dropdown_menu(|m, _, _| {
                m.menu("强制重新编译", Box::new(Recompile))
                    .menu("重新扫描项目", Box::new(Rescan))
                    .menu("在资源管理器中显示项目", Box::new(RevealProject))
                    .separator()
                    .menu("打开排版示例", Box::new(OpenDemo))
            }))
            .child(menu("menu-help", "帮助").dropdown_menu(|m, _, _| {
                m.menu("用户指南", Box::new(OpenGuide))
                    .menu("复制诊断信息", Box::new(CopyDiagnostics))
                    .menu("打开日志文件夹", Box::new(OpenLogFolder))
                    .separator()
                    .menu("关于字笺 / 快捷键", Box::new(About))
            }))
    }

    fn render_title(&self, max_width: f32, cx: &App) -> impl IntoElement {
        let tri = 10.;
        let docs: Vec<(String, String)> = self
            .cards
            .iter()
            .map(|c| (c.path.clone(), format!("{}  ·  {}", c.title, c.rel)))
            .collect();
        bar_button("title-menu", cx)
            .compact()
            .px_2()
            .h(px(28.))
            .child(
                h_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .max_w(px(max_width))
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(CREAM))
                            .child(self.title_text()),
                    )
                    // Smartisan's recessed folder triangle.
                    .child(
                        div()
                            .relative()
                            .size(px(tri))
                            .child(
                                svg()
                                    .path("skin/g_triangle.svg")
                                    .absolute()
                                    .top(px(1.))
                                    .size(px(tri))
                                    .text_color(rgba(0xfff0dc33)),
                            )
                            .child(
                                svg()
                                    .path("skin/g_triangle.svg")
                                    .absolute()
                                    .size(px(tri))
                                    .text_color(rgb(0x51423a)),
                            ),
                    ),
            )
            .dropdown_menu(move |mut m, _, _| {
                for (path, label) in &docs {
                    let label = label.clone();
                    m = m.menu_element(
                        Box::new(OpenDocument { path: path.clone() }),
                        move |window, _| path_menu_label(label.clone(), window),
                    );
                }
                if !docs.is_empty() {
                    m = m.separator();
                }
                m.menu("新建文稿…", Box::new(NewDocument))
                    .menu("打开文件夹…", Box::new(OpenFolder))
                    .menu("打开文件…", Box::new(OpenFile))
            })
    }

    /// Empty title-bar space that drags the window. Windows and Linux let
    /// the platform hit-test it; macOS owns the drag itself (the window
    /// options set `app_owns_titlebar_drag`), and double-click follows the
    /// system title-bar preference.
    fn drag_area(&self, id: &'static str) -> Stateful<Div> {
        let area = div().id(id).flex_1().h_full();
        if cfg!(target_os = "macos") {
            let should_move = Rc::new(Cell::new(false));
            let down = should_move.clone();
            let up = should_move.clone();
            let leave = should_move.clone();
            area.on_mouse_down(MouseButton::Left, move |_, _, _| down.set(true))
                .on_mouse_up(MouseButton::Left, move |_, _, _| up.set(false))
                .on_mouse_down_out(move |_, _, _| leave.set(false))
                .on_mouse_move(move |_, window, _| {
                    if should_move.get() {
                        should_move.set(false);
                        window.start_window_move();
                    }
                })
                .on_double_click(|_, window, _| window.titlebar_double_click())
        } else {
            area.window_control_area(WindowControlArea::Drag)
        }
    }

    fn render_titlebar(&self, window: &mut Window, cx: &App) -> impl IntoElement {
        let macos = cfg!(target_os = "macos");
        // Keep the walnut chrome quiet: menus, title and tools share one scale.
        let tool_size = 28.;
        let glyph_size = 15.;
        // Reserve room for menus and tools before sizing the centered title.
        let title_width = (f32::from(window.viewport_size().width)
            - if macos { 540. } else { 740. })
            .clamp(120., 360.);
        let tip = |mac: &'static str, win: &'static str| if macos { mac } else { win };
        let tool =
            |id: &'static str, glyph: &'static str, tip: &'static str, action: Box<dyn Action>| {
                bar_button(id, cx)
                    .compact()
                    .size(px(tool_size))
                    .tooltip(tip)
                    .child(bar_glyph(glyph, glyph_size))
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
        // In macOS fullscreen the traffic lights are hidden with the menu bar.
        let left_pad = if macos && !window.is_fullscreen() { 78. } else { 12. };

        walnut_bar(38.).child(
            h_flex()
                .relative()
                .size_full()
                .child(
                    h_flex()
                        .flex_1()
                        .h_full()
                        .pl(px(left_pad))
                        .gap_2()
                        .child(skin("notebook", 20., 20.))
                        .child(
                            div()
                                .mr_2()
                                .text_size(px(13.))
                                .font_weight(FontWeight::BOLD)
                                .child(embossed("字笺")),
                        )
                        .when(!macos, |left| left.child(self.render_menus(cx)))
                        .child(self.drag_area("title-drag-left")),
                )
                .child(self.render_title(title_width, cx))
                .child(
                    h_flex()
                        .flex_1()
                        .h_full()
                        .gap_1()
                        .child(self.drag_area("title-drag-right"))
                        .child(tool(
                            "tool-new",
                            "compose",
                            tip("新建文稿  ⌘N", "新建文稿  Ctrl+N"),
                            Box::new(NewDocument),
                        ))
                        .child(tool(
                            "tool-open",
                            "folder",
                            tip("打开文件夹  ⌘⇧O", "打开文件夹  Ctrl+Shift+O"),
                            Box::new(OpenFolder),
                        ))
                        .child(tool(
                            "tool-save",
                            "disk",
                            tip("保存  ⌘S", "保存  Ctrl+S"),
                            Box::new(Save),
                        ))
                        .child(tool(
                            "tool-compile",
                            "compile",
                            "强制重新编译  F5",
                            Box::new(Recompile),
                        ))
                        .child(tool(
                            "tool-export",
                            "plane",
                            tip("导出 PDF  ⌘⇧E", "导出 PDF  Ctrl+Shift+E"),
                            Box::new(ExportPdf),
                        ))
                        .child(groove_v(18.).mx_1())
                        .child(tool(
                            "tool-sidebar",
                            "sidebar",
                            tip("文稿栏  ⌘B", "文稿栏  Ctrl+B"),
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
                    this.persist();
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
    pub(super) fn binding_strip(tint: u32) -> Div {
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

    pub(super) fn card_frame(id: impl Into<ElementId>, lifted: bool) -> Stateful<Div> {
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

    fn render_card(&self, card: &DocCard, cx: &mut Context<Self>) -> impl IntoElement {
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

        // Ids by path, so hover and tooltip state follow a card when the
        // list reorders (starring moves cards to the top).
        Self::card_frame(SharedString::from(format!("card-{}", card.path)), active)
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
                                icon_well(
                                    div().id(SharedString::from(format!("pin-{}", card.path))),
                                )
                                .tooltip(move |w, cx| {
                                    Tooltip::new(if pinned {
                                        "取消固定（恢复自动选择入口）"
                                    } else {
                                        "固定为预览主文件"
                                    })
                                    .build(w, cx)
                                })
                                .child(skin(if pinned { "pin_on" } else { "pin" }, 16., 16.))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_pin(pin_path.clone(), cx);
                                    },
                                )),
                            )
                            .child(
                                icon_well(
                                    div().id(SharedString::from(format!("star-{}", card.path))),
                                )
                                .tooltip(move |w, cx| {
                                    Tooltip::new(if starred { "取消置顶" } else { "置顶" })
                                        .build(w, cx)
                                })
                                .child(skin(if starred { "star_on" } else { "star" }, 16., 16.))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_star(star_path.clone(), cx);
                                    },
                                )),
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
                                    this.persist();
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
                        if self.scanning {
                            "正在读取项目…"
                        } else if self.cards.is_empty() {
                            "这个文件夹里还没有 .typ 文稿（Ctrl+N 新建）"
                        } else {
                            "没有匹配的文稿"
                        },
                        WOOD_INK,
                    )),
            );
        }
        for card in cards {
            list = list.child(self.render_card(card, cx));
        }
        list
    }

    fn render_files(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let active = self.active();
        let root = self
            .ws
            .root()
            .map(|r| r.display().to_string())
            .unwrap_or_default();
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
        )
        // Right-click: create, rename, reveal, move to the recycle bin.
        .context_menu(move |_ix, entry, menu, _window, _cx| {
            let path = entry.item().id.to_string();
            let dir = if entry.is_folder() {
                path.clone()
            } else {
                Path::new(&path)
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| root.clone())
            };
            menu.menu("新建文稿…", Box::new(TreeNewFile { dir: dir.clone() }))
                .menu("新建文件夹…", Box::new(TreeNewFolder { dir }))
                .separator()
                .menu("重命名…", Box::new(TreeRename { path: path.clone() }))
                .menu(
                    "在资源管理器中显示",
                    Box::new(TreeReveal { path: path.clone() }),
                )
                .separator()
                .menu("移到回收站…", Box::new(TreeDelete { path }))
        });

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
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(FAINT))
                                            .child("右键可新建 / 重命名"),
                                    ),
                            )
                            .child(div().pl(px(26.)).child(paper_divider()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .min_w_0()
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
                this.persist();
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

    // ── Notepad: index tabs, sheet header, banners, ruled paper ──

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
            let failed = self.save_errors.contains_key(&path) || self.outside.contains_key(&path);
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
                    .when(failed, |t| {
                        t.child(div().size(px(6.)).rounded_full().bg(rgb(0xb5402f)))
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
                                let label = label.clone();
                                m = m.menu_element(
                                    Box::new(OpenDocument { path: path.clone() }),
                                    move |window, _| path_menu_label(label.clone(), window),
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
                        m.menu("新建文稿…", Box::new(NewDocument))
                            .menu("打开文件夹…", Box::new(OpenFolder))
                            .menu("打开文件…", Box::new(OpenFile))
                            .separator()
                            .menu("在项目中查找…", Box::new(ProjectSearch))
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
            .when_some(active.clone(), |header, path| {
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
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(ToggleStarActive { path: path.clone() }),
                                    cx,
                                )
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

    /// A slip of paper across the top of the sheet for things that need
    /// the user: failed saves and outside edits. It stays until resolved.
    fn render_banners(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active();
        let banner = |id: SharedString, tone: u32, text: String| {
            h_flex()
                .id(id)
                .flex_none()
                .min_h(px(36.))
                .pl(px(MARGIN_W + 14.))
                .pr(px(10.))
                .py_1()
                .gap_2()
                .bg(rgb(tone))
                .border_b_1()
                .border_color(rgba(0xb4966c4d))
                .text_size(px(12.5))
                .text_color(rgb(INK))
                .child(div().flex_1().min_w_0().child(text))
        };
        let button = |id: SharedString, label: &'static str| {
            div()
                .id(id)
                .px_2()
                .h(px(24.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(rgb(0xc9b28c))
                .bg(rgb(0xfffaf0))
                .cursor_pointer()
                .hover(|d| d.bg(rgb(0xfff3dc)))
                .text_size(px(12.))
                .child(label)
        };

        let mut col = v_flex().flex_none();
        // Save failures: every file still unsaved, listed until it saves.
        if !self.save_errors.is_empty() {
            let (first, reason) = self.save_errors.iter().next().unwrap();
            let more = self.save_errors.len() - 1;
            let text = if more > 0 {
                format!(
                    "{} 保存失败：{reason}（另有 {more} 个文件未能保存）",
                    file_name(first)
                )
            } else {
                format!(
                    "{} 保存失败：{reason}。修改仍在编辑器中。",
                    file_name(first)
                )
            };
            let paths: Vec<String> = self.save_errors.keys().cloned().collect();
            col = col.child(
                banner("banner-save".into(), 0xf6dcd2, text)
                    .child(
                        button("banner-retry".into(), "重试保存").on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.save_with_conflict_check(paths.clone(), window, cx)
                            },
                        )),
                    )
                    .child(
                        button("banner-save-as".into(), "另存为…")
                            .on_click(cx.listener(|this, _, window, cx| this.save_as(window, cx))),
                    ),
            );
        }
        // Outside edit to the open document while it has unsaved changes.
        if let Some(path) = active.filter(|p| self.outside.contains_key(p)) {
            let deleted = self.outside.get(&path).is_some_and(|v| v.is_none());
            let text = if deleted {
                "此文稿已在磁盘上被删除或移走。保存会在原位置重新创建它。".to_string()
            } else {
                "此文稿已被其他程序修改，而这里还有未保存的修改。".to_string()
            };
            let (reload, keep) = (path.clone(), path.clone());
            let mut row = banner("banner-outside".into(), 0xf8ecc8, text);
            if !deleted {
                row = row.child(button("banner-reload".into(), "载入磁盘版本").on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.reload_from_disk(&reload, window, cx)
                    }),
                ));
            }
            col = col.child(
                row.child(
                    button("banner-keep".into(), "保留我的版本")
                        .on_click(cx.listener(move |this, _, _, cx| this.keep_mine(&keep, cx))),
                ),
            );
        }
        col
    }

    /// Margin strip, its double edge and the writing rules, kept in step
    /// with the editor's scroll position. Compact mode is plain paper.
    fn render_ruled_paper(&self) -> impl IntoElement {
        let editor = self.active_editor().cloned();
        let compact = self.compact;
        let fallback_line = self.line_height();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                if compact {
                    return;
                }
                let (line_h, scroll_y) = match &editor {
                    Some(editor) => {
                        let state = editor.read(cx);
                        (
                            f32::from(state.line_height().unwrap_or(px(fallback_line))),
                            f32::from(state.scroll_offset().y),
                        )
                    }
                    None => (fallback_line, 0.),
                };
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
        let editor = self.active_editor().cloned();
        let empty = editor.is_none();

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
                            .child(div().relative().child(self.render_banners(cx)))
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_h_0()
                                    .child(self.render_ruled_paper())
                                    .when_some(editor, |area, editor| {
                                        area.child(
                                            div()
                                                .size_full()
                                                .pl(px(if self.compact {
                                                    4.
                                                } else {
                                                    MARGIN_W - 2.
                                                }))
                                                .child(
                                                    Editor::new(&editor)
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
                                                    "从左侧选一份文稿，或按 Ctrl+N 新建、Ctrl+O 打开",
                                                    0xb0a598,
                                                )),
                                        )
                                    }),
                            ),
                    ),
            )
    }

    // ── Preview column and status strip ─────────────────────

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
            // A small cream control resting on the desk; it must not let
            // clicks through to the page beneath (that would jump to source).
            .child(
                h_flex()
                    .occlude()
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
        let cursor = self.active_editor().map(|e| e.read(cx).cursor_position());
        let (led, status_text) = match &self.status {
            _ if !self.save_errors.is_empty() => (
                "led_red",
                format!("{} 个文件保存失败", self.save_errors.len()),
            ),
            _ if self.compiling => ("led_amber", "排版中…".to_string()),
            CompileStatus::Idle => ("led_amber", "就绪".to_string()),
            CompileStatus::Ok { ms } => ("led_green", format!("排版完成  {ms} ms")),
            CompileStatus::Failed { first, count } => {
                ("led_red", format!("{count} 处错误：{first}"))
            }
        };
        let errors = self.problems.iter().filter(|p| p.is_error).count();
        let warnings = self.problems.len() - errors;
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
                .when(!self.problems.is_empty(), |bar| {
                    bar.child(
                        div()
                            .id("status-problems")
                            .px_2()
                            .rounded(px(3.))
                            .cursor_pointer()
                            .hover(|d| d.bg(rgba(0x0000001f)))
                            .child(embossed(format!("✗ {errors}   ⚠ {warnings}")))
                            .on_click(|_, w, cx| w.dispatch_action(Box::new(ShowProblems), cx)),
                    )
                    .child(groove_v(14.))
                })
                .when_some(cursor, |bar, cursor| {
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
}

impl Render for TypstEditorApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = if self.ws.root().is_none() && self.active().is_none() {
            self.render_empty_state(cx).into_any_element()
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
        let panel = self.panel;

        v_flex()
            .id("typst-editor")
            .track_focus(&self.app_focus)
            .size_full()
            .font_family(serif)
            .text_size(px(14.))
            .text_color(rgb(INK))
            .bg(rgb(0xf0ddbc))
            .on_action(cx.listener(|this, _: &NewDocument, w, cx| this.new_document(w, cx)))
            .on_action(cx.listener(|this, _: &NewProject, w, cx| this.new_project(w, cx)))
            .on_action(cx.listener(|this, _: &OpenFile, w, cx| this.prompt_open(false, w, cx)))
            .on_action(cx.listener(|this, _: &OpenFolder, w, cx| this.prompt_open(true, w, cx)))
            .on_action(cx.listener(|this, a: &OpenRecent, w, cx| {
                let root = std::path::PathBuf::from(&a.root);
                if root.is_dir() {
                    this.open_path(root, w, cx);
                } else {
                    this.show_notice("该项目文件夹已不存在", cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Save, w, cx| {
                if let Some(active) = this.active() {
                    this.save_with_conflict_check(vec![active], w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SaveAs, w, cx| this.save_as(w, cx)))
            .on_action(cx.listener(|this, _: &SaveAll, w, cx| {
                let dirty = this.ws.dirty_paths();
                this.save_with_conflict_check(dirty, w, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, w, cx| {
                if let Some(active) = this.active() {
                    this.request(Pending::CloseTab(active), w, cx);
                } else {
                    // No tab left: close the window (Cmd+W a second time).
                    this.request(Pending::CloseWindow, w, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Find, w, cx| this.open_search(false, w, cx)))
            .on_action(cx.listener(|this, _: &FindReplace, w, cx| this.open_search(true, w, cx)))
            .on_action(cx.listener(|this, _: &ProjectSearch, w, cx| {
                this.panel = Some(Panel::Search);
                this.find_input.update(cx, |i, cx| i.focus(w, cx));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowProblems, _, cx| {
                this.panel = if this.panel == Some(Panel::Problems) {
                    None
                } else {
                    Some(Panel::Problems)
                };
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExportPdf, w, cx| this.export_pdf(w, cx)))
            .on_action(cx.listener(|this, a: &OpenDocument, w, cx| {
                this.open_document(a.path.clone(), w, cx)
            }))
            .on_action(
                cx.listener(|this, a: &ToggleStarActive, _, cx| {
                    this.toggle_star(a.path.clone(), cx)
                }),
            )
            .on_action(cx.listener(|this, a: &TreeNewFile, w, cx| {
                this.begin_name_dialog(
                    Pending::NewItem {
                        dir: a.dir.clone().into(),
                        folder: false,
                    },
                    "新文稿.typ",
                    w,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, a: &TreeNewFolder, w, cx| {
                this.begin_name_dialog(
                    Pending::NewItem {
                        dir: a.dir.clone().into(),
                        folder: true,
                    },
                    "新文件夹",
                    w,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, a: &TreeRename, w, cx| {
                let name = file_name(&a.path);
                this.begin_name_dialog(
                    Pending::Rename {
                        path: a.path.clone(),
                    },
                    &name,
                    w,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, a: &TreeDelete, w, cx| {
                this.pending = Some(Pending::Delete {
                    path: a.path.clone(),
                });
                this.dialog_focus.focus(w, cx);
                cx.notify();
            }))
            .on_action(
                cx.listener(|_, a: &TreeReveal, _, _| reveal_in_file_manager(Path::new(&a.path))),
            )
            .on_action(cx.listener(|this, _: &UndoEdit, window, cx| {
                if let Some(editor) = this.active_editor().cloned() {
                    editor.update(cx, |s, cx| s.focus(window, cx));
                    window.dispatch_action(Box::new(gpui_kit::component::input::Undo), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RedoEdit, window, cx| {
                if let Some(editor) = this.active_editor().cloned() {
                    editor.update(cx, |s, cx| s.focus(window, cx));
                    window.dispatch_action(Box::new(gpui_kit::component::input::Redo), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Recompile, _, cx| {
                this.compile_active(
                    std::time::Duration::ZERO,
                    CompileOptions { force: true },
                    false,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &Rescan, _, cx| this.rescan(cx)))
            .on_action(cx.listener(|this, _: &RevealProject, _, _| {
                if let Some(root) = this.ws.root() {
                    reveal_in_file_manager(root);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.show_sidebar = !this.show_sidebar;
                this.persist();
                this.refresh_macos_menus(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleOutline, _, cx| {
                this.show_sidebar = true;
                this.outline_open = !this.outline_open;
                this.persist();
                this.refresh_macos_menus(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleCompact, window, cx| {
                this.set_compact(!this.compact, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FontBigger, _, cx| {
                this.font_px = (this.font_px + 1.).min(24.);
                this.persist();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FontSmaller, _, cx| {
                this.font_px = (this.font_px - 1.).max(11.);
                this.persist();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FontReset, _, cx| {
                this.font_px = DEFAULT_FONT;
                this.persist();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenDemo, window, cx| this.open_demo(window, cx)))
            .on_action(cx.listener(|this, _: &CopyDiagnostics, _, cx| this.copy_diagnostics(cx)))
            .on_action(cx.listener(|_, _: &OpenLogFolder, _, _| {
                let dir = logging::log_dir();
                let _ = std::fs::create_dir_all(&dir);
                reveal_in_file_manager(&dir);
            }))
            .on_action(cx.listener(|this, _: &OpenGuide, _, cx| this.open_guide(cx)))
            .on_action(cx.listener(|this, _: &About, _, cx| {
                this.show_about = !this.show_about;
                cx.notify();
            }))
            // The native macOS menu bar routes here too; on other platforms
            // these simply have no key bindings and no in-window menu items.
            .on_action(cx.listener(|this, _: &Quit, w, cx| {
                this.request(Pending::CloseWindow, w, cx)
            }))
            .on_action(cx.listener(|_, _: &HideApp, _, cx| cx.hide()))
            .on_action(cx.listener(|_, _: &HideOthers, _, cx| cx.hide_other_apps()))
            .on_action(cx.listener(|_, _: &UnhideOthers, _, cx| cx.unhide_other_apps()))
            .on_action(cx.listener(|_, _: &MinimizeWindow, window, _| {
                window.minimize_window()
            }))
            .on_action(cx.listener(|_, _: &ZoomWindow, window, _| window.zoom_window()))
            .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| {
                window.toggle_fullscreen()
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                if this.pending.is_none() {
                    this.show_settings = true;
                    this.show_about = false;
                    this.dialog_focus.focus(window, cx);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_by(Some(1.1), cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_by(Some(1. / 1.1), cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.zoom_by(None, cx)))
            .on_action(cx.listener(|this, _: &FitWidth, _, cx| {
                this.preview.update(cx, |p, cx| p.set_fit_width(cx));
                cx.notify();
            }))
            .child(self.render_titlebar(_window, cx))
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
                    .when_some(panel, |desk, panel| {
                        desk.child(self.render_panel(panel, cx))
                    })
                    .when_some(self.notice.clone(), |desk, text| {
                        desk.child(self.render_notice(text))
                    })
                    .when(self.show_about, |desk| desk.child(self.render_about(cx)))
                    .when(self.show_settings, |desk| desk.child(self.render_settings(cx)))
                    .when_some(pending, |desk, action| {
                        desk.child(self.render_pending(&action, cx))
                    }),
            )
            .child(self.render_status_bar(cx))
    }
}
