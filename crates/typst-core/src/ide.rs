use serde::{Deserialize, Serialize};
use typst::foundations::{Content, NativeElement, Packed, StyleChain};
use typst::introspection::Introspector;
use typst::layout::{Frame, FrameItem, PagedDocument, Point, Position, Size};
use typst::model::HeadingElem;
use typst::syntax::{Side, Source, Span, SyntaxKind};
use typst::visualize::Geometry;
use typst::Document;
use typst::World;

use crate::compiler::{CompileResult, CompilerState, OutlineItem, OutlinePosition};
use crate::world::TypstWorld;

pub use typst_ide::{Completion, CompletionKind, Tooltip};

// ── Data Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionResult {
    pub page: usize,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpResult {
    #[serde(rename = "type")]
    pub jump_type: String,
    pub file: Option<String>,
    pub offset: Option<usize>,
    pub url: Option<String>,
    pub position: Option<PositionResult>,
}

// ── Outline ─────────────────────────────────────────────────

/// Fill `result.outline` from the current document (cleared on failure).
pub fn populate_outline(state: &CompilerState, result: &mut CompileResult) {
    if !result.success {
        result.outline.clear();
        return;
    }
    result.outline = document_outline(state);
}

pub fn document_outline(state: &CompilerState) -> Vec<OutlineItem> {
    state
        .with_document(|world, doc| build_document_outline(world, doc.introspector()))
        .unwrap_or_default()
}

fn build_document_outline(world: &TypstWorld, introspector: &Introspector) -> Vec<OutlineItem> {
    introspector
        .query(&HeadingElem::ELEM.select())
        .iter()
        .filter_map(|elem| {
            let heading = elem.to_packed::<HeadingElem>()?;
            let bookmarked = heading
                .bookmarked
                .get(StyleChain::default())
                .unwrap_or_else(|| heading.outlined.get(StyleChain::default()));
            if !bookmarked {
                return None;
            }

            let title = plain_outline_title(&heading.body);
            if title.is_empty() {
                return None;
            }

            let location = heading.location()?;
            let doc_pos = introspector.position(location);
            let span = effective_heading_span(heading);
            let (file, offset) =
                resolve_span_to_file_offset(world, span, 0).unwrap_or((String::new(), 0));
            let has_location = !file.is_empty();

            Some(OutlineItem {
                title,
                level: heading.resolve_level(StyleChain::default()).get(),
                file: has_location.then_some(file),
                offset: has_location.then_some(offset),
                position: Some(OutlinePosition {
                    page: doc_pos.page.get(),
                    x: doc_pos.point.x.to_pt(),
                    y: doc_pos.point.y.to_pt(),
                }),
            })
        })
        .collect()
}

fn plain_outline_title(content: &Content) -> String {
    content.plain_text().trim().to_string()
}

fn effective_heading_span(heading: &Packed<HeadingElem>) -> Span {
    let span = heading.span();
    if span.is_detached() {
        heading.body.span()
    } else {
        span
    }
}

// ── Editor -> Preview ───────────────────────────────────────

fn positions_to_results(positions: Vec<Position>) -> Vec<PositionResult> {
    positions
        .into_iter()
        .map(|pos| PositionResult {
            page: pos.page.get(),
            x: pos.point.x.to_pt(),
            y: pos.point.y.to_pt(),
        })
        .collect()
}

/// Preview positions for a 0-based line/character cursor.
pub fn jump_from_cursor(
    state: &CompilerState,
    file_path: &str,
    line: usize,
    character: usize,
) -> Vec<PositionResult> {
    state
        .with_document(|world, doc| {
            let Some(source) = world.source_by_path(file_path) else {
                return Vec::new();
            };
            let Some(cursor) = source.lines().line_column_to_byte(line, character) else {
                return Vec::new();
            };
            positions_to_results(typst_ide::jump_from_cursor(doc, &source, cursor))
        })
        .unwrap_or_default()
}

/// Preview positions for a byte offset in `file_path`.
pub fn jump_from_offset(
    state: &CompilerState,
    file_path: &str,
    offset: usize,
) -> Vec<PositionResult> {
    state
        .with_document(|world, doc| {
            let Some(source) = world.source_by_path(file_path) else {
                return Vec::new();
            };
            let cursor = offset.min(source.text().len());
            positions_to_results(typst_ide::jump_from_cursor(doc, &source, cursor))
        })
        .unwrap_or_default()
}

// ── Preview -> Editor ───────────────────────────────────────

/// Resolve a click on 1-based `page` at (`x`, `y`) pt to a source location.
pub fn jump_from_click(state: &CompilerState, page: usize, x: f64, y: f64) -> Option<JumpResult> {
    state.with_document(|world, doc| {
        let page_idx = page.saturating_sub(1);
        if page_idx >= doc.pages.len() {
            return None;
        }

        let click = Point::new(typst::layout::Abs::pt(x), typst::layout::Abs::pt(y));
        let frame = &doc.pages[page_idx].frame;
        let jump = typst_ide::jump_from_click(world, doc, frame, click)?;

        Some(match jump {
            typst_ide::Jump::File(file_id, offset) => {
                let name = world
                    .resolve_absolute_path(file_id)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| file_id.vpath().as_rootless_path().display().to_string());
                JumpResult {
                    jump_type: "file".into(),
                    file: Some(name),
                    offset: Some(offset),
                    url: None,
                    position: None,
                }
            }
            typst_ide::Jump::Url(url) => JumpResult {
                jump_type: "url".into(),
                file: None,
                offset: None,
                url: Some(url.to_string()),
                position: None,
            },
            typst_ide::Jump::Position(pos) => resolve_source_jump_from_position(world, doc, pos)
                .unwrap_or_else(|| JumpResult {
                    jump_type: "position".into(),
                    file: None,
                    offset: None,
                    url: None,
                    position: Some(PositionResult {
                        page: pos.page.get(),
                        x: pos.point.x.to_pt(),
                        y: pos.point.y.to_pt(),
                    }),
                }),
        })
    })?
}

fn resolve_source_jump_from_position(
    world: &TypstWorld,
    doc: &PagedDocument,
    pos: Position,
) -> Option<JumpResult> {
    let page_idx = pos.page.get().checked_sub(1)?;
    let page = doc.pages.get(page_idx)?;
    let (span, offset) = source_span_from_click(world, &page.frame, pos.point)?;
    let (file, offset) = resolve_span_to_file_offset(world, span, offset)?;

    Some(JumpResult {
        jump_type: "file".into(),
        file: Some(file),
        offset: Some(offset),
        url: None,
        position: None,
    })
}

fn source_span_from_click(
    world: &TypstWorld,
    frame: &Frame,
    click: Point,
) -> Option<(Span, usize)> {
    for &(mut pos, ref item) in frame.items().rev() {
        match item {
            FrameItem::Group(group) => {
                if let Some(hit) = source_span_from_click(world, &group.frame, click - pos) {
                    return Some(hit);
                }
            }
            FrameItem::Text(text) => {
                for glyph in &text.glyphs {
                    let width = glyph.x_advance.at(text.size);
                    let hitbox_pos = Point::new(pos.x, pos.y - text.size);
                    if is_in_rect(hitbox_pos, Size::new(width, text.size), click) {
                        let (span, glyph_offset) = glyph.span;
                        let mut resolved_offset = glyph_offset as usize;
                        let source = world.source(span.id()?).ok()?;
                        let node = source.find(span)?;
                        if matches!(node.kind(), SyntaxKind::Text | SyntaxKind::MathText)
                            && (click.x - pos.x) > width / 2.0
                        {
                            resolved_offset += glyph.range().len();
                        }
                        return Some((span, resolved_offset));
                    }
                    pos.x += width;
                }
            }
            FrameItem::Shape(shape, span) => {
                let Geometry::Rect(size) = shape.geometry else {
                    continue;
                };
                if is_in_rect(pos, size, click) {
                    return Some((*span, 0));
                }
            }
            FrameItem::Image(_, size, span) if is_in_rect(pos, *size, click) => {
                return Some((*span, 0));
            }
            _ => {}
        }
    }

    None
}

fn resolve_span_to_file_offset(
    world: &TypstWorld,
    span: Span,
    extra_offset: usize,
) -> Option<(String, usize)> {
    let source = world.source(span.id()?).ok()?;
    let range = source.find(span)?.range();
    let offset = range.start.saturating_add(extra_offset.min(range.len()));
    let path = world.resolve_absolute_path(span.id()?)?;
    Some((path.display().to_string(), offset))
}

fn is_in_rect(pos: Point, size: Size, click: Point) -> bool {
    pos.x <= click.x && pos.x + size.x >= click.x && pos.y <= click.y && pos.y + size.y >= click.y
}

// ── Completion / Hover ──────────────────────────────────────

/// Build a `Source` for `file_path` holding the editor's live `text`, so IDE
/// queries see unsaved edits even before the next compile lands.
fn live_source(world: &TypstWorld, file_path: &str, text: &str) -> Option<Source> {
    let source = world.source_by_path(file_path)?;
    if source.text() == text {
        Some(source)
    } else {
        Some(Source::new(source.id(), text.to_string()))
    }
}

/// Autocomplete at byte `cursor`. Returns the byte offset where the
/// replaced word starts, plus the candidates.
pub fn autocomplete(
    state: &CompilerState,
    file_path: &str,
    text: &str,
    cursor: usize,
    explicit: bool,
) -> Option<(usize, Vec<Completion>)> {
    state
        .with_world(|world, doc| {
            let source = live_source(world, file_path, text)?;
            typst_ide::autocomplete(world, doc, &source, cursor.min(text.len()), explicit)
        })
        .flatten()
}

/// Hover tooltip at byte `cursor`.
pub fn tooltip(
    state: &CompilerState,
    file_path: &str,
    text: &str,
    cursor: usize,
) -> Option<Tooltip> {
    state
        .with_world(|world, doc| {
            let source = live_source(world, file_path, text)?;
            typst_ide::tooltip(world, doc, &source, cursor.min(text.len()), Side::After)
        })
        .flatten()
}
