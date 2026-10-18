//! 字笺 palette, sampled from Smartisan Notes: walnut bars, a pale birch
//! desk, cream note paper with tan rules, and dark brown serif ink.

use std::sync::Arc;

use gpui_kit::component::Theme;
use gpui_kit::*;

// Walnut bars.
pub const WALNUT: u32 = 0x736252;
pub const WALNUT_EDGE: u32 = 0x4d3e31;
/// Cream lettering pressed out of the walnut.
pub const CREAM: u32 = 0xfbf2e2;

// Paper.
pub const PAPER: u32 = 0xfbf7ec;
pub const CARD: u32 = 0xfbf8f1;
pub const CARD_HEAD: u32 = 0xfdfcf7;
pub const BAND: u32 = 0xf6f2e9;
pub const MARGIN: u32 = 0xede7db;
pub const RULE: u32 = 0xe6dfd2;
pub const DIVIDER: u32 = 0xeee8de;

// Ink.
pub const INK: u32 = 0x574033;
pub const INK_SOFT: u32 = 0x756153;
pub const FAINT: u32 = 0xb3aba1;
/// Lettering engraved into the birch desk.
pub const WOOD_INK: u32 = 0xa0825c;
pub const AMBER: u32 = 0xb98c57;

/// Relative paths on cards: secondary, but they tell same-named files apart.
pub const PATH_INK: u32 = 0x8a7864;

/// Preferred faces, best first. The first one installed wins; the last
/// entries ship with every Windows/macOS install.
const SERIF_FACES: &[&str] = &[
    "Noto Serif SC",
    "Source Han Serif SC",
    "Source Han Serif CN",
    "Songti SC",
    "STSong",
    "SimSun",
    "Microsoft YaHei UI",
    "PingFang SC",
];
const MONO_FACES: &[&str] = &[
    "LXGW Bright Code",
    "Sarasa Mono SC",
    "Maple Mono NF CN",
    "Noto Sans Mono CJK SC",
    "Cascadia Mono",
    "Consolas",
    "Menlo",
    "Courier New",
];

/// Fonts chosen at startup from what is installed.
pub struct Fonts {
    pub serif: SharedString,
    pub mono: SharedString,
}

impl Global for Fonts {}

pub fn fonts(cx: &App) -> &Fonts {
    cx.global::<Fonts>()
}

fn pick(installed: &[String], faces: &[&str], fallback: &str) -> SharedString {
    faces
        .iter()
        .find(|face| installed.iter().any(|name| name == *face))
        .copied()
        .unwrap_or(fallback)
        .to_string()
        .into()
}

pub fn install(cx: &mut App) {
    let installed = cx.text_system().all_font_names();
    let fonts = Fonts {
        serif: pick(&installed, SERIF_FACES, ".SystemUIFont"),
        mono: pick(&installed, MONO_FACES, "monospace"),
    };
    let (serif, mono) = (fonts.serif.clone(), fonts.mono.clone());
    cx.set_global(fonts);

    let t = Theme::global_mut(cx);
    t.font_family = serif;
    t.mono_font_family = mono;
    t.font_size = px(14.);
    t.mono_font_size = px(15.);
    t.radius = px(3.);
    t.background = rgb(0xf0ddbc).into();
    t.foreground = rgb(INK).into();
    t.title_bar = rgb(WALNUT).into();
    t.title_bar_border = rgb(WALNUT_EDGE).into();
    t.sidebar = rgb(CARD).into();
    t.sidebar_foreground = rgb(INK).into();
    t.muted = rgb(0xece3d3).into();
    t.muted_foreground = rgb(FAINT).into();
    t.border = rgb(0xe0d6c6).into();
    t.input = rgb(0xdfd3c7).into();
    t.accent = rgb(0xf1e4cb).into();
    t.accent_foreground = rgb(INK).into();
    t.primary = rgb(AMBER).into();
    t.primary_hover = rgb(0xa57a48).into();
    t.primary_active = rgb(0x8f6638).into();
    t.primary_foreground = rgb(CARD).into();
    t.caret = rgb(0x8a5a2b).into();
    t.ring = rgb(AMBER).into();
    t.selection = rgba(0xd9b67a66).into();
    t.colors.list = rgb(CARD).into();
    t.list_hover = rgb(0xf3eadb).into();
    t.list_active = rgb(0xf1e4cb).into();
    t.list_active_border = rgb(0xd8bd8c).into();
    t.popover = rgb(CARD).into();
    t.popover_foreground = rgb(INK).into();
    t.button = rgb(0xf7f1e6).into();
    t.button_hover = rgb(0xefe5d4).into();
    t.button_active = rgb(0xe6d8c0).into();
    t.button_foreground = rgb(INK).into();
    t.secondary = rgb(BAND).into();
    t.secondary_foreground = rgb(INK).into();
    t.success = rgb(0x5d7d48).into();
    t.danger = rgb(0xa94a3c).into();
    t.scrollbar_thumb = rgba(0x9c7b5266).into();
    t.scrollbar_thumb_hover = rgba(0x9c7b52aa).into();

    // The editor paints on transparent ground: the notepad (ruled paper,
    // margin) is drawn underneath it by the app.
    let h = Arc::make_mut(&mut t.highlight_theme);
    h.name = "字笺 · 稿纸".into();
    h.style.editor_background = Some(hsla(0., 0., 0., 0.));
    h.style.editor_foreground = Some(rgb(INK).into());
    h.style.editor_gutter_background = Some(hsla(0., 0., 0., 0.));
    h.style.editor_line_number = Some(rgb(0xc2b6a4).into());
    h.style.editor_active_line_number = Some(rgb(AMBER).into());
    h.style.editor_active_line = Some(rgba(0xe9cf9f26).into());
}

/// Ink colours for syntax: brown-black prose with fountain-pen accents.
pub fn syntax_color(name: &str) -> Hsla {
    rgb(match name {
        "comment" => 0xa89c88,
        "keyword" | "operator" => 0x9a4f2c,
        "function" | "variable" => 0x3f6577,
        "string" | "string.escape" => 0x6b7a3a,
        "number" => 0xa66f22,
        "title" | "emphasis.strong" => 0x7a3f1d,
        "label" | "link_uri" => 0x557a7c,
        "punctuation" | "punctuation.bracket" | "punctuation.list_marker" => 0x9b8872,
        _ => INK,
    })
    .into()
}
