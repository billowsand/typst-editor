//! Skeuomorphic building blocks: walnut bars, the birch desk, paper cards,
//! embossed and letterpressed lettering, recessed fields and raised buttons.
//!
//! GPUI has no text shadows, so lettering depth is faked with a second copy
//! of the text offset by one pixel underneath.

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::theme::*;

pub fn shadow(x: f32, y: f32, blur: f32, spread: f32, color: u32) -> BoxShadow {
    BoxShadow {
        color: rgba(color).into(),
        offset: point(px(x), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    }
}

pub fn inset(x: f32, y: f32, blur: f32, color: u32) -> BoxShadow {
    BoxShadow {
        inset: true,
        ..shadow(x, y, blur, 0., color)
    }
}

/// A sheet of paper resting on the desk.
pub fn paper_shadow() -> Vec<BoxShadow> {
    vec![
        shadow(0., 1., 1.5, 0., 0x5a3c1e47),
        shadow(0., 3., 8., 0., 0x5a3c1e1c),
    ]
}

/// A sheet lifted a little off the desk (active card, preview pages).
pub fn lifted_shadow() -> Vec<BoxShadow> {
    vec![
        shadow(0., 1., 2., 0., 0x5a3c1e52),
        shadow(0., 6., 16., 0., 0x5a3c1e2e),
    ]
}

// ── Lettering ───────────────────────────────────────────────

/// Text with a 1px shadow copy: `shadow` below `color`.
fn layered(text: SharedString, color: Hsla, shade: Hsla, dy: f32) -> Div {
    div()
        .relative()
        .whitespace_nowrap()
        .child(
            div()
                .absolute()
                .top(px(dy))
                .left_0()
                .whitespace_nowrap()
                .text_color(shade)
                .child(text.clone()),
        )
        .child(div().relative().text_color(color).child(text))
}

/// Cream lettering raised out of walnut (dark shadow below).
pub fn embossed(text: impl Into<SharedString>) -> Div {
    layered(text.into(), rgb(CREAM).into(), rgba(0x2a1c1199).into(), 1.)
}

/// Ink pressed into paper or wood (white highlight below).
pub fn letterpress(text: impl Into<SharedString>, color: u32) -> Div {
    layered(text.into(), rgb(color).into(), rgba(0xffffffb3).into(), 1.)
}

// ── Glyphs ──────────────────────────────────────────────────

fn layered_glyph(name: &str, size: f32, color: Hsla, shade: Hsla, dy: f32) -> Div {
    let path: SharedString = format!("skin/g_{name}.svg").into();
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(
            svg()
                .path(path.clone())
                .absolute()
                .top(px(dy))
                .left_0()
                .size(px(size))
                .text_color(shade),
        )
        .child(
            svg()
                .path(path)
                .absolute()
                .top_0()
                .left_0()
                .size(px(size))
                .text_color(color),
        )
}

/// Cream glyph raised out of walnut.
pub fn bar_glyph(name: &str, size: f32) -> Div {
    layered_glyph(name, size, rgb(CREAM).into(), rgba(0x2a1c11b3).into(), 1.)
}

/// Grey glyph pressed into paper.
pub fn paper_glyph(name: &str, size: f32, color: u32) -> Div {
    layered_glyph(name, size, rgb(color).into(), rgba(0xffffffd9).into(), 1.)
}

/// Full-colour skin image (pins, stars, clips, ribbons, lights).
pub fn skin(name: &str, w: f32, h: f32) -> Img {
    img(SharedString::from(format!("skin/{name}.svg")))
        .w(px(w))
        .h(px(h))
        .flex_none()
}

// ── Surfaces ────────────────────────────────────────────────

/// Walnut strip: texture, a top highlight and a dark bottom edge.
pub fn walnut_bar(height: f32) -> Div {
    div()
        .relative()
        .flex_none()
        .h(px(height))
        .w_full()
        .overflow_hidden()
        .bg(rgb(WALNUT))
        .child(
            img("textures/walnut.jpg")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .object_fit(ObjectFit::Cover),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(rgba(0xffffff14), 0.),
                    linear_color_stop(rgba(0x00000014), 1.),
                )),
        )
        // Light catching the top edge, and the dark lower edge.
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(px(1.))
                .bg(rgba(0xfff0dc2e)),
        )
        .child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .w_full()
                .h(px(1.))
                .bg(rgb(WALNUT_EDGE)),
        )
}

/// Soft shadow a walnut bar casts onto the desk below it.
pub fn bar_cast_shadow(top: bool) -> Div {
    div()
        .absolute()
        .left_0()
        .w_full()
        .h(px(9.))
        .bg(linear_gradient(
            if top { 180. } else { 0. },
            linear_color_stop(rgba(0x3c281442), 0.),
            linear_color_stop(rgba(0x3c281400), 1.),
        ))
}

/// The birch desk filling its parent.
pub fn desk() -> Img {
    img("textures/birch.jpg")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .object_fit(ObjectFit::Cover)
}

/// A recessed pill, like Smartisan's search field.
pub fn inset_pill() -> Div {
    div()
        .rounded_full()
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgb(0xebe7e2), 0.),
            linear_color_stop(rgb(0xfaf9f6), 0.6),
        ))
        .border_1()
        .border_color(rgb(0xd9ccbd))
        .shadow(vec![
            inset(0., 1., 3., 0x6a4a2a40),
            shadow(0., 1., 0., 0., 0xffffffb3),
        ])
}

/// A small recessed well on paper (folder selector, zoom readout).
pub fn inset_well() -> Div {
    div()
        .rounded(px(4.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgb(0xe6dfd3), 0.),
            linear_color_stop(rgb(0xf1ebe1), 1.),
        ))
        .border_1()
        .border_color(rgb(0xddd3c5))
        .shadow(vec![
            inset(0., 1., 2., 0x6a4a2a33),
            shadow(0., 1., 0., 0., 0xffffffcc),
        ])
}

/// Transparent button for walnut bars: recesses into the wood on hover.
pub fn bar_button(id: impl Into<ElementId>, cx: &App) -> Button {
    Button::new(id)
        .custom(
            ButtonCustomVariant::new(cx)
                .color(hsla(0., 0., 0., 0.))
                .foreground(rgb(CREAM).into())
                .hover(rgba(0x0000001f).into())
                .active(rgba(0x00000036).into()),
        )
        .rounded(px(5.))
}

/// Raised cream paper button (welcome screen, dialogs).
pub fn paper_button(id: impl Into<ElementId>, glyph: &str, label: &str) -> Stateful<Div> {
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap_2()
        .px_4()
        .h(px(36.))
        .rounded(px(5.))
        .cursor_pointer()
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgb(0xfffdf8), 0.),
            linear_color_stop(rgb(0xf1e9da), 1.),
        ))
        .border_1()
        .border_color(rgb(0xd4c5ae))
        .shadow(vec![
            shadow(0., 1., 1.5, 0., 0x5a3c1e40),
            inset(0., 1., 0., 0xffffffcc),
        ])
        .text_color(rgb(INK))
        .hover(|s| s.border_color(rgb(0xc4ae8c)))
        .active(|s| {
            s.bg(rgb(0xece2d0))
                .shadow(vec![inset(0., 1., 3., 0x5a3c1e40)])
        })
        .when(!glyph.is_empty(), |b| {
            b.child(paper_glyph(glyph, 16., INK_SOFT))
        })
        .child(label.to_string())
}

/// Engraved separator: a dark groove with a highlight beside it.
pub fn groove_v(height: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .h(px(height))
        .child(div().w(px(1.)).h_full().bg(rgba(0x2a1c1166)))
        .child(div().w(px(1.)).h_full().bg(rgba(0xfff0dc26)))
}

/// Paper divider: a faint rule with a white line under it.
pub fn paper_divider() -> Div {
    div()
        .flex_none()
        .w_full()
        .child(div().h(px(1.)).w_full().bg(rgb(DIVIDER)))
        .child(div().h(px(1.)).w_full().bg(rgba(0xffffffcc)))
}
