//! Font preferences on the same paper sheet used by the other dialogs.

use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::{IndexPath, h_flex, v_flex};
use gpui_kit::*;

use super::*;
use crate::skin::*;
use crate::theme::*;

const AUTOMATIC: &str = "自动选择（推荐）";
type FontSelect = SelectState<SearchableVec<SharedString>>;

pub(super) struct Preferences {
    ui: Entity<FontSelect>,
    editor: Entity<FontSelect>,
    _subscriptions: Vec<Subscription>,
}

impl TypstEditorApp {
    pub(super) fn make_preferences(
        settings: &Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Preferences {
        let mut installed = cx.text_system().all_font_names();
        installed.sort_by_key(|name| name.to_lowercase());
        installed.dedup();
        let items: Vec<SharedString> = std::iter::once(AUTOMATIC.to_owned())
            .chain(installed)
            .map(Into::into)
            .collect();
        let mut make = |saved: &str| {
            let index = items
                .iter()
                .position(|name| name.as_ref() == saved)
                .unwrap_or(0);
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(items.clone()),
                    Some(IndexPath::new(index)),
                    window,
                    cx,
                )
                .searchable(true)
            })
        };
        let ui = make(&settings.ui_font);
        let editor = make(&settings.editor_font);
        let subscriptions = [(ui.clone(), false), (editor.clone(), true)]
            .into_iter()
            .map(|(select, is_editor)| {
                cx.subscribe(
                    &select,
                    move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
                        let SelectEvent::Confirm(Some(value)) = event else {
                            return;
                        };
                        let saved = if value.as_ref() == AUTOMATIC {
                            String::new()
                        } else {
                            value.to_string()
                        };
                        if is_editor {
                            this.settings.editor_font = saved;
                        } else {
                            this.settings.ui_font = saved;
                        }
                        apply_fonts(&this.settings.ui_font, &this.settings.editor_font, cx);
                        this.persist();
                        cx.notify();
                    },
                )
            })
            .collect();
        Preferences {
            ui,
            editor,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_settings = false;
        if let Some(editor) = self.active_editor().cloned() {
            editor.update(cx, |state, cx| state.focus(window, cx));
        } else {
            self.app_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let section = |title: &'static str, detail: &'static str| {
            h_flex()
                .gap_3()
                .child(
                    div()
                        .w(px(110.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(INK_SOFT))
                        .child(detail),
                )
        };
        let sample = |family: SharedString, size: f32, text: &'static str| {
            div()
                .flex_none()
                .h(px(size + 24.))
                .px_3()
                .py_2()
                .bg(rgb(BAND))
                .border_1()
                .border_color(rgb(RULE))
                .rounded(px(3.))
                .font_family(family)
                .text_size(px(size))
                .overflow_hidden()
                .whitespace_nowrap()
                .child(text)
        };
        Self::modal(
            self.sheet(560.)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.close_settings(window, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(Self::dialog_title("设置 · 字体"))
                .child(paper_divider())
                .child(
                    v_flex()
                        .px_5()
                        .py_3()
                        .gap_2()
                        .child(section("界面字体", "用于菜单、文稿栏与面板文字"))
                        .child(
                            Select::new(&self.preferences.ui)
                                .w_full()
                                .search_placeholder("搜索已安装字体…")
                                .menu_max_h(px(220.)),
                        )
                        .child(sample(
                            fonts(cx).serif.clone(),
                            14.,
                            "字笺 · 把想法排成作品。Aa 0123456789",
                        ))
                        .child(section("编辑器字体", "用于源码编辑，建议选择等宽字体"))
                        .child(
                            Select::new(&self.preferences.editor)
                                .w_full()
                                .search_placeholder("搜索已安装字体…")
                                .menu_max_h(px(220.)),
                        )
                        .child(
                            h_flex()
                                .gap_3()
                                .child(div().flex_1().child("编辑器字号"))
                                .child(paper_button("settings-smaller", "", "−").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.font_px = (this.font_px - 1.).max(11.);
                                        this.persist();
                                        cx.notify();
                                    }),
                                ))
                                .child(
                                    div()
                                        .w(px(46.))
                                        .text_center()
                                        .child(format!("{} px", self.font_px)),
                                )
                                .child(paper_button("settings-bigger", "", "+").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.font_px = (this.font_px + 1.).min(24.);
                                        this.persist();
                                        cx.notify();
                                    }),
                                )),
                        )
                        .child(sample(
                            fonts(cx).mono.clone(),
                            self.font_px,
                            "#let title = [字笺 Typst] // 0123456789",
                        ))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(INK_SOFT))
                                .child("修改立即生效并自动保存。文档排版字体由 Typst 源码设置。"),
                        ),
                )
                .child(paper_divider())
                .child(
                    h_flex()
                        .p_4()
                        .gap_2()
                        .child(paper_button("settings-reset", "", "恢复默认字体").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.settings.ui_font.clear();
                                this.settings.editor_font.clear();
                                this.font_px = DEFAULT_FONT;
                                let automatic = SharedString::from(AUTOMATIC);
                                this.preferences.ui.update(cx, |s, cx| {
                                    s.set_selected_value(&automatic, window, cx)
                                });
                                this.preferences.editor.update(cx, |s, cx| {
                                    s.set_selected_value(&automatic, window, cx)
                                });
                                apply_fonts("", "", cx);
                                this.persist();
                                cx.notify();
                            }),
                        ))
                        .child(div().flex_1())
                        .child(paper_button("settings-done", "", "完成").on_click(
                            cx.listener(|this, _, window, cx| this.close_settings(window, cx)),
                        )),
                ),
        )
    }
}
