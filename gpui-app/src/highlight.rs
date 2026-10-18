//! Typst syntax highlighting for the gpui-kit editor.
//!
//! gpui-kit ships no Typst tree-sitter grammar, so we plug in our own
//! `InputHighlighter` backed by `typst::syntax` — the same parser the
//! compiler uses. Edits are applied with `Source::edit`, which reparses only
//! around the change, and styling walks just the visible part of the tree,
//! so large documents stay responsive.

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::input::{
    EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter,
    InputHighlighterFactory, Rope,
};
use gpui_kit::{Context, HighlightStyle, SharedString, Window};
use typst::syntax::{FileId, LinkedNode, Source, Tag, VirtualPath};

pub const LANGUAGE: &str = "typst";

/// Factory for `EditorState::set_highlighter_factory`.
pub fn typst_highlighter_factory() -> InputHighlighterFactory {
    Rc::new(|language| {
        (language == LANGUAGE)
            .then(|| Box::new(TypstHighlighter::new()) as Box<dyn InputHighlighter>)
    })
}

/// Theme token name for a typst highlight tag.
fn token_name(tag: Tag) -> Option<&'static str> {
    Some(match tag {
        Tag::Comment => "comment",
        Tag::Punctuation => "punctuation",
        Tag::Escape => "string.escape",
        Tag::Strong | Tag::ListTerm => "emphasis.strong",
        Tag::Emph => "emphasis",
        Tag::Link => "link_uri",
        Tag::Raw => "text.literal",
        Tag::Label | Tag::Ref => "label",
        Tag::Heading => "title",
        Tag::ListMarker => "punctuation.list_marker",
        Tag::MathDelimiter => "punctuation.bracket",
        Tag::MathOperator | Tag::Operator => "operator",
        Tag::Keyword => "keyword",
        Tag::Number => "number",
        Tag::String => "string",
        Tag::Function => "function",
        Tag::Interpolated => "variable",
        Tag::Error => return None,
    })
}

pub struct TypstHighlighter {
    source: Source,
}

impl TypstHighlighter {
    fn new() -> Self {
        let id = FileId::new(None, VirtualPath::new("editor.typ"));
        Self {
            source: Source::new(id, String::new()),
        }
    }

    /// Bring the parse tree up to date with `text`, the document after
    /// `edit`. Uses the edit when it is consistent with what we have,
    /// otherwise falls back to a (diffing) full replace.
    fn apply(&mut self, edit: Option<&InputEdit>, text: &Rope) {
        if let Some(edit) = edit {
            let old_len = self.source.text().len();
            let consistent = edit.start_byte <= edit.old_end_byte
                && edit.old_end_byte <= old_len
                && edit.start_byte <= edit.new_end_byte
                && edit.new_end_byte <= text.len()
                && old_len - (edit.old_end_byte - edit.start_byte)
                    + (edit.new_end_byte - edit.start_byte)
                    == text.len()
                && self.source.text().is_char_boundary(edit.start_byte)
                && self.source.text().is_char_boundary(edit.old_end_byte);
            if consistent {
                let inserted = text.slice(edit.start_byte..edit.new_end_byte).to_string();
                self.source
                    .edit(edit.start_byte..edit.old_end_byte, &inserted);
                return;
            }
        }
        let text = text.to_string();
        if self.source.text() != text {
            self.source.replace(&text);
        }
    }
}

impl InputHighlighter for TypstHighlighter {
    fn language(&self) -> SharedString {
        LANGUAGE.into()
    }

    fn update(
        &mut self,
        edit: Option<InputEdit>,
        text: &Rope,
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        self.apply(edit.as_ref(), text);
    }

    fn update_batch(
        &mut self,
        edits: &[(InputEdit, Rope)],
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        for (edit, text) in edits {
            self.apply(Some(edit), text);
        }
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut spans = Vec::new();
        collect_spans(
            &LinkedNode::new(self.source.root()),
            None,
            range,
            &mut spans,
        );

        let mut out = Vec::with_capacity(spans.len() * 2 + 1);
        let mut pos = range.start;
        for (span, name) in spans {
            let start = span.start.max(range.start).max(pos);
            let end = span.end.min(range.end);
            if start >= end {
                continue;
            }
            if start > pos {
                out.push((pos..start, HighlightStyle::default()));
            }
            let mut style = resolver.style(name).unwrap_or_default();
            style.color = Some(crate::theme::syntax_color(name));
            out.push((start..end, style));
            pos = end;
        }
        if pos < range.end {
            out.push((pos..range.end, HighlightStyle::default()));
        }
        out
    }

    fn fold_ranges(&self, _text: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

/// Highlighted leaves intersecting `range`, in order. Markup containers
/// (heading, strong, raw, ...) colour the leaves inside them unless a leaf
/// has a more specific tag. Subtrees outside `range` are skipped.
fn collect_spans(
    node: &LinkedNode,
    inherited: Option<&'static str>,
    range: &Range<usize>,
    out: &mut Vec<(Range<usize>, &'static str)>,
) {
    let node_range = node.range();
    if node_range.end <= range.start || node_range.start >= range.end {
        // Zero-width leaves at the boundary carry no text either way.
        return;
    }
    let name = typst::syntax::highlight(node)
        .and_then(token_name)
        .or(inherited);
    if node.get().children().len() == 0 {
        if let Some(name) = name
            && !node_range.is_empty()
        {
            match out.last_mut() {
                Some((prev, prev_name)) if prev.end == node_range.start && *prev_name == name => {
                    prev.end = node_range.end;
                }
                _ => out.push((node_range, name)),
            }
        }
        return;
    }
    for child in node.children() {
        collect_spans(&child, name, range, out);
    }
}

#[cfg(test)]
pub fn compute_spans(source: &Source) -> Vec<(Range<usize>, &'static str)> {
    let mut spans = Vec::new();
    collect_spans(
        &LinkedNode::new(source.root()),
        None,
        &(0..source.text().len()),
        &mut spans,
    );
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &str) -> Vec<(String, &'static str)> {
        let source = Source::detached(text);
        compute_spans(&source)
            .into_iter()
            .map(|(r, name)| (text[r].to_string(), name))
            .collect()
    }

    #[test]
    fn highlights_markup_and_code() {
        let spans = spans("= 标题\n#let x = 1\n");
        assert!(
            spans
                .iter()
                .any(|(t, n)| t.contains("标题") && *n == "title")
        );
        assert!(
            spans
                .iter()
                .any(|(t, n)| t.contains("let") && *n == "keyword")
        );
        assert!(spans.iter().any(|(t, n)| t == "1" && *n == "number"));
    }

    struct Any;
    impl HighlightStyleResolver for Any {
        fn style(&self, _: &str) -> Option<HighlightStyle> {
            Some(HighlightStyle::default())
        }
    }

    #[test]
    fn styles_cover_requested_range() {
        let mut h = TypstHighlighter::new();
        h.apply(None, &Rope::from("#let x = 1"));
        let runs = h.styles(&(2..9), &Any);
        assert_eq!(runs.first().unwrap().0.start, 2);
        assert_eq!(runs.last().unwrap().0.end, 9);
        assert!(runs.windows(2).all(|w| w[0].0.end == w[1].0.start));
    }

    /// Apply `edits` one by one incrementally; the tree must match a fresh
    /// parse of the final text.
    #[test]
    fn incremental_edits_match_a_fresh_parse() {
        let mut h = TypstHighlighter::new();
        let mut text = String::from("= 标题\n\n正文 *加粗* 与 $x^2$。\n#let f(x) = x + 1\n");
        h.apply(None, &Rope::from(text.as_str()));
        let edits: [(usize, usize, &str); 4] = [
            (0, 0, "#set page(width: 10cm)\n"),
            (5, 9, "text"),
            (2, 2, "中文插入"),
            (10, 25, ""),
        ];
        for (start, old_len, insert) in edits {
            let start = (0..=start.min(text.len()))
                .rev()
                .find(|i| text.is_char_boundary(*i))
                .unwrap();
            let end = (start + old_len).min(text.len());
            let end = (end..=text.len())
                .find(|i| text.is_char_boundary(*i))
                .unwrap();
            text.replace_range(start..end, insert);
            let edit = InputEdit {
                start_byte: start,
                old_end_byte: end,
                new_end_byte: start + insert.len(),
                start_position: gpui_kit::base::input::Point::new(0, 0),
                old_end_position: gpui_kit::base::input::Point::new(0, 0),
                new_end_position: gpui_kit::base::input::Point::new(0, 0),
            };
            h.apply(Some(&edit), &Rope::from(text.as_str()));
            assert_eq!(h.source.text(), text);
        }
        assert_eq!(
            compute_spans(&h.source),
            compute_spans(&Source::detached(text.as_str()))
        );
    }

    #[test]
    fn inconsistent_edits_fall_back_to_a_full_replace() {
        let mut h = TypstHighlighter::new();
        h.apply(None, &Rope::from("abc"));
        let bogus = InputEdit {
            start_byte: 50,
            old_end_byte: 60,
            new_end_byte: 70,
            start_position: gpui_kit::base::input::Point::new(0, 0),
            old_end_position: gpui_kit::base::input::Point::new(0, 0),
            new_end_position: gpui_kit::base::input::Point::new(0, 0),
        };
        h.apply(Some(&bogus), &Rope::from("= changed"));
        assert_eq!(h.source.text(), "= changed");
    }
}
