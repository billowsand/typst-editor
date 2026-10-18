//! Preview panel: pages rasterized with `typst-render` and shown as GPU
//! textures. Only pages near the viewport keep a bitmap; zoom re-rasterizes
//! at the new pixel density while the old bitmap stays up as a placeholder.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use smallvec::SmallVec;
use typst_core::CompilerState;
use typst_core::ide::PositionResult;

/// Pages kept rasterized beyond the visible ones, on each side.
const KEEP_AROUND: usize = 2;
/// Gap between pages, logical px.
const PAGE_GAP: f32 = 18.;

pub enum PreviewEvent {
    /// The user clicked text in the preview that maps to `file` at `offset`.
    JumpToSource { file: String, offset: usize },
    /// The user clicked a link.
    OpenUrl(String),
}

/// Largest bitmap side we upload: common GPU texture limit.
const MAX_TEXTURE_SIDE: f32 = 8192.;
/// Upper bound on pixels per point, whatever the zoom and screen.
const MAX_PIXEL_PER_PT: f32 = 4.;

/// Pixels per point for a page: the requested density, capped so the
/// bitmap stays within the texture limit and a sane memory budget.
pub fn page_density(requested: f32, width_pt: f64, height_pt: f64) -> f32 {
    let longest = width_pt.max(height_pt).max(1.) as f32;
    requested
        .min(MAX_PIXEL_PER_PT)
        .min(MAX_TEXTURE_SIDE / longest)
        .max(0.1)
}

struct PageSlot {
    width_pt: f64,
    height_pt: f64,
    hash: u64,
    image: Option<Arc<RenderImage>>,
    /// `(page hash, pixels per pt)` the current `image` was rendered for.
    image_key: Option<(u64, u32)>,
    /// Key of an in-flight rasterization, if any.
    pending_key: Option<(u64, u32)>,
}

pub struct PreviewPanel {
    compiler: Arc<CompilerState>,
    pages: Vec<PageSlot>,
    zoom: f32,
    /// Follow the panel width: the widest page fills it, re-fit on resize.
    fit_width: bool,
    scroll: ScrollHandle,
    /// Window-space bounds of each page from the last prepaint.
    page_bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    /// Cursor location(s) mirrored from the editor.
    markers: Vec<PositionResult>,
    /// Images to release from the GPU atlas on the next frame.
    retired: Vec<Arc<RenderImage>>,
}

impl EventEmitter<PreviewEvent> for PreviewPanel {}

impl PreviewPanel {
    pub fn new(compiler: Arc<CompilerState>) -> Self {
        Self {
            compiler,
            pages: Vec::new(),
            zoom: 1.0,
            fit_width: true,
            scroll: ScrollHandle::new(),
            page_bounds: Rc::new(RefCell::new(Vec::new())),
            markers: Vec::new(),
            retired: Vec::new(),
        }
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn fit_width(&self) -> bool {
        self.fit_width
    }

    /// Fit the widest page to the panel, and keep doing so on resize.
    pub fn set_fit_width(&mut self, cx: &mut Context<Self>) {
        self.fit_width = true;
        cx.notify();
    }

    /// A fixed zoom level; leaves fit-width mode.
    pub fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        self.fit_width = false;
        self.zoom = zoom.clamp(0.25, 5.0);
        cx.notify();
    }

    /// Adopt a fresh compile. Pages whose hash is unchanged keep their bitmap;
    /// changed pages keep showing the old bitmap until the new one is ready.
    pub fn set_document(
        &mut self,
        sizes: Vec<(f64, f64)>,
        hashes: Vec<u64>,
        cx: &mut Context<Self>,
    ) {
        let mut old = std::mem::take(&mut self.pages).into_iter();
        for ((width_pt, height_pt), hash) in sizes.into_iter().zip(hashes) {
            let mut slot = old.next().unwrap_or(PageSlot {
                width_pt,
                height_pt,
                hash,
                image: None,
                image_key: None,
                pending_key: None,
            });
            if slot.width_pt != width_pt || slot.height_pt != height_pt {
                // A stretched placeholder looks worse than a blank page.
                self.retired.extend(slot.image.take());
                slot.image_key = None;
            }
            slot.width_pt = width_pt;
            slot.height_pt = height_pt;
            slot.hash = hash;
            self.pages.push(slot);
        }
        self.retired.extend(old.filter_map(|slot| slot.image));
        self.page_bounds.borrow_mut().truncate(self.pages.len());
        cx.notify();
    }

    /// Show cursor markers and scroll the first one into view if needed.
    pub fn set_markers(&mut self, markers: Vec<PositionResult>, cx: &mut Context<Self>) {
        if let Some(first) = markers.first() {
            self.reveal(first);
        }
        self.markers = markers;
        cx.notify();
    }

    fn reveal(&self, pos: &PositionResult) {
        let bounds = self.page_bounds.borrow();
        let Some(page) = pos.page.checked_sub(1).and_then(|ix| bounds.get(ix)) else {
            return;
        };
        let viewport = self.scroll.bounds();
        let y = page.origin.y + px(pos.y as f32 * self.zoom);
        let margin = viewport.size.height / 4.;
        if y > viewport.origin.y + margin && y < viewport.bottom() - margin {
            return;
        }
        // Scroll offsets are negative; move so the marker sits a third down.
        let offset = self.scroll.offset();
        let content_y = y - viewport.origin.y - offset.y;
        let target = (content_y - viewport.size.height / 3.).max(px(0.));
        self.scroll.set_offset(point(offset.x, -target));
    }

    fn on_page_click(&mut self, page_ix: usize, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(bounds) = self.page_bounds.borrow().get(page_ix).copied() else {
            return;
        };
        let local = event.position - bounds.origin;
        let x = f32::from(local.x) as f64 / self.zoom as f64;
        let y = f32::from(local.y) as f64 / self.zoom as f64;
        let Some(jump) = typst_core::ide::jump_from_click(&self.compiler, page_ix + 1, x, y) else {
            return;
        };
        match jump.jump_type.as_str() {
            "url" => {
                if let Some(url) = jump.url {
                    cx.emit(PreviewEvent::OpenUrl(url));
                }
            }
            // A cross-reference: show where it points, in the preview.
            "position" => {
                if let Some(pos) = jump.position {
                    self.set_markers(vec![pos], cx);
                }
            }
            _ => {
                if let (Some(file), Some(offset)) = (jump.file, jump.offset) {
                    cx.emit(PreviewEvent::JumpToSource { file, offset });
                }
            }
        }
    }

    /// In fit-width mode, size the widest page to the panel (last frame's
    /// width), snapped to whole percents so a resize does not re-rasterize
    /// on every pixel.
    fn apply_fit_width(&mut self) {
        if !self.fit_width {
            return;
        }
        let width = f32::from(self.scroll.bounds().size.width);
        let widest = self
            .pages
            .iter()
            .map(|p| p.width_pt as f32)
            .fold(0., f32::max);
        if width <= 0. || widest <= 0. {
            return;
        }
        // Side padding on both sides plus room for the scrollbar.
        let available = width - 2. * PAGE_GAP - 14.;
        self.zoom = ((available / widest) * 100.).floor().clamp(25., 300.) / 100.;
    }

    /// Pages intersecting the viewport, from the previous frame's layout.
    fn visible_range(&self) -> std::ops::Range<usize> {
        let bounds = self.page_bounds.borrow();
        let viewport = self.scroll.bounds();
        if bounds.len() != self.pages.len() || viewport.size.height <= px(0.) {
            return 0..self.pages.len().min(2);
        }
        let first = bounds.iter().position(|b| b.bottom() >= viewport.origin.y);
        let last = bounds.iter().rposition(|b| b.origin.y <= viewport.bottom());
        match (first, last) {
            (Some(first), Some(last)) if first <= last => first..last + 1,
            _ => 0..0,
        }
    }

    /// Kick off rasterization for pages near the viewport and evict the rest.
    fn schedule_rasterization(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for image in self.retired.drain(..) {
            cx.drop_image(image, Some(window));
        }

        let visible = self.visible_range();
        let keep = visible.start.saturating_sub(KEEP_AROUND)
            ..(visible.end + KEEP_AROUND).min(self.pages.len());
        // Snap density so tiny layout changes don't trigger re-renders.
        let requested = (self.zoom * window.scale_factor() * 8.).round() / 8.;

        for (ix, slot) in self.pages.iter_mut().enumerate() {
            if !keep.contains(&ix) {
                if let Some(image) = slot.image.take() {
                    cx.drop_image(image, Some(window));
                }
                slot.image_key = None;
                continue;
            }
            let pixel_per_pt = page_density(requested, slot.width_pt, slot.height_pt);
            let key = (slot.hash, (pixel_per_pt * 1000.) as u32);
            if slot.image_key == Some(key) || slot.pending_key == Some(key) {
                continue;
            }
            slot.pending_key = Some(key);

            let compiler = self.compiler.clone();
            cx.spawn_in(window, async move |this, cx| {
                let image = cx
                    .background_spawn(async move {
                        let (w, h, mut rgba) = compiler.render_page_rgba(ix, pixel_per_pt)?;
                        for px in rgba.chunks_exact_mut(4) {
                            swap_rgba_pa_to_bgra(px);
                        }
                        let buffer = image::RgbaImage::from_raw(w, h, rgba)?;
                        let frames: SmallVec<[image::Frame; 1]> =
                            SmallVec::from_elem(image::Frame::new(buffer), 1);
                        Some(Arc::new(RenderImage::new(frames)))
                    })
                    .await;
                this.update_in(cx, |this, window, cx| {
                    let Some(slot) = this.pages.get_mut(ix) else {
                        return;
                    };
                    if slot.pending_key != Some(key) {
                        // Superseded by a newer compile or zoom level.
                        if let Some(image) = image {
                            cx.drop_image(image, Some(window));
                        }
                        return;
                    }
                    slot.pending_key = None;
                    // Record the attempt even when it failed (the document
                    // went away), so it is not retried on every frame.
                    slot.image_key = Some(key);
                    if let Some(image) = image {
                        if let Some(old) = slot.image.replace(image) {
                            cx.drop_image(old, Some(window));
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    fn render_page(&self, ix: usize, slot: &PageSlot, cx: &mut Context<Self>) -> impl IntoElement {
        let zoom = self.zoom;
        let bounds_store = self.page_bounds.clone();
        // Cursor marks in red-brown ink, like a proofreader's pencil.
        let marker_color: Hsla = rgb(0xc0533d).into();

        let page = div()
            .id(("page", ix))
            .relative()
            .flex_none()
            .w(px(slot.width_pt as f32 * zoom))
            .h(px(slot.height_pt as f32 * zoom))
            .bg(white())
            .shadow(crate::skin::lifted_shadow())
            .child(
                canvas(
                    move |bounds, _, _| {
                        let mut store = bounds_store.borrow_mut();
                        if store.len() <= ix {
                            store.resize(ix + 1, Bounds::default());
                        }
                        store[ix] = bounds;
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .when_some(slot.image.clone(), |page, image| {
                page.child(img(image).absolute().size_full())
            })
            .children(
                self.markers
                    .iter()
                    .filter(|m| m.page == ix + 1)
                    .map(move |m| {
                        div()
                            .absolute()
                            .left(px((m.x as f32 - 1.) * zoom))
                            .top(px((m.y as f32 - 11.) * zoom))
                            .w(px(2.5 * zoom))
                            .h(px(14. * zoom))
                            .bg(marker_color.opacity(0.8))
                            .rounded_sm()
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.on_page_click(ix, event, cx);
                }),
            )
            // A paperclip holds the stack together at the first page.
            .when(ix == 0, |page| {
                page.child(
                    crate::skin::skin("paperclip_v", 20., 40.)
                        .absolute()
                        .top(px(-15.))
                        .left(px(34.)),
                )
            });

        v_flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .child(page)
            .child(div().text_size(px(11.)).child(crate::skin::letterpress(
                format!("—  {}  —", ix + 1),
                crate::theme::WOOD_INK,
            )))
    }
}

impl Render for PreviewPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_fit_width();
        self.schedule_rasterization(window, cx);

        let pages: Vec<_> = self
            .pages
            .iter()
            .enumerate()
            .map(|(ix, slot)| self.render_page(ix, slot, cx).into_any_element())
            .collect();

        div()
            .id("preview-scroll")
            .relative()
            .size_full()
            .overflow_scroll()
            .track_scroll(&self.scroll)
            // Layout moves on scroll; re-render to rasterize newly visible pages.
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .child(
                v_flex()
                    .relative()
                    .min_w_full()
                    .items_center()
                    .gap(px(PAGE_GAP))
                    // Leave room for the zoom control floating at the top.
                    .pt(px(56.))
                    .pb(px(28.))
                    .px(px(PAGE_GAP))
                    .children(pages),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::page_density;

    #[test]
    fn density_is_capped_by_texture_size_and_budget() {
        // A4 at 2x screen and 5x zoom asks for 10 px/pt: capped at 4.
        assert_eq!(page_density(10., 595., 842.), 4.);
        // A very long page is limited by the texture side instead.
        let d = page_density(2., 595., 20000.);
        assert!(20000. * d <= 8192. + 0.5);
        // Normal requests pass through.
        assert_eq!(page_density(1.5, 595., 842.), 1.5);
    }
}
