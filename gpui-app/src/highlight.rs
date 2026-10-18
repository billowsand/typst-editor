//! Typst syntax highlighting for the gpui-kit editor.
//!
//! gpui-kit ships no Typst tree-sitter grammar, so we plug in our own
//! `InputHighlighter` backed by `typst::syntax` — the same parser the
//! compiler uses. `Source::replace` reparses incrementally, so a keystroke
//! only re-parses the syntax nodes around the edit.

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
    /// Sorted, non-overlapping highlighted byte ranges.
    spans: Vec<(Range<usize>, &'static str)>,
}

impl TypstHighlighter {
    fn new() -> Self {
        let id = FileId::new(None, VirtualPath::new("editor.typ"));
        Self {
            source: Source::new(id, String::new()),
            spans: Vec::new(),
        }
    }

    fn reparse(&mut self, text: &Rope) {
        let text = text.to_string();
        if self.source.text() != text {
            self.source.replace(&text);
        }
        self.spans = compute_spans(&self.source);
    }
}

impl InputHighlighter for TypstHighlighter {
    fn language(&self) -> SharedString {
        LANGUAGE.into()
    }

    fn update(
        &mut self,
        _edit: Option<InputEdit>,
        text: &Rope,
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        self.reparse(text);
    }

    fn update_batch(
        &mut self,
        edits: &[(InputEdit, Rope)],
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        if let Some((_, text)) = edits.last() {
            self.reparse(text);
        }
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut out = Vec::new();
        let mut pos = range.start;
        let first = self.spans.partition_point(|(r, _)| r.end <= range.start);
        for (span, name) in &self.spans[first..] {
            if span.start >= range.end {
                break;
            }
            let start = span.start.max(range.start);
            let end = span.end.min(range.end);
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

/// Highlight every leaf; markup containers (heading, strong, raw, ...) color
/// the leaves inside them unless a leaf has a more specific tag.
pub fn compute_spans(source: &Source) -> Vec<(Range<usize>, &'static str)> {
    let mut spans = Vec::new();
    walk(&LinkedNode::new(source.root()), None, &mut spans);
    spans
}

fn walk(
    node: &LinkedNode,
    inherited: Option<&'static str>,
    out: &mut Vec<(Range<usize>, &'static str)>,
) {
    let name = typst::syntax::highlight(node)
        .and_then(token_name)
        .or(inherited);
    if node.get().children().len() == 0 {
        let range = node.range();
        if let Some(name) = name
            && !range.is_empty()
        {
            // Merge with the previous span when contiguous and same style.
            match out.last_mut() {
                Some((prev, prev_name)) if prev.end == range.start && *prev_name == name => {
                    prev.end = range.end;
                }
                _ => out.push((range, name)),
            }
        }
        return;
    }
    for child in node.children() {
        walk(&child, name, out);
    }
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

    #[test]
    fn styles_cover_requested_range() {
        let mut h = TypstHighlighter::new();
        h.reparse(&Rope::from("#let x = 1"));
        struct Any;
        impl HighlightStyleResolver for Any {
            fn style(&self, _: &str) -> Option<HighlightStyle> {
                Some(HighlightStyle::default())
            }
        }
        let runs = h.styles(&(2..9), &Any);
        assert_eq!(runs.first().unwrap().0.start, 2);
        assert_eq!(runs.last().unwrap().0.end, 9);
        assert!(runs.windows(2).all(|w| w[0].0.end == w[1].0.start));
    }
}
