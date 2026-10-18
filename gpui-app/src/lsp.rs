//! Completion and hover for the editor, answered in-process by `typst-ide`
//! through `typst-core` (no language server process needed).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{CompletionProvider, HoverProvider, Rope, RopeExt};
use gpui_kit::{App, Result, Task, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Hover, HoverContents, MarkupContent, MarkupKind, TextEdit,
};
use typst_core::CompilerState;
use typst_core::ide::{CompletionKind, Tooltip};

/// Shared handle to "which file is the editor showing right now".
pub type ActivePath = Rc<RefCell<Option<String>>>;

pub struct TypstLsp {
    pub compiler: Arc<CompilerState>,
    pub active_path: ActivePath,
}

impl CompletionProvider for TypstLsp {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let Some(path) = self.active_path.borrow().clone() else {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        };
        let compiler = self.compiler.clone();
        let rope = text.clone();
        let explicit = trigger.trigger_kind == lsp_types::CompletionTriggerKind::INVOKED;

        cx.background_executor().spawn(async move {
            let source = rope.to_string();
            let Some((from, completions)) =
                typst_core::ide::autocomplete(&compiler, &path, &source, offset, explicit)
            else {
                return Ok(CompletionResponse::Array(vec![]));
            };

            let range = lsp_types::Range::new(
                rope.offset_to_position(from.min(offset)),
                rope.offset_to_position(offset),
            );
            let items = completions
                .into_iter()
                .map(|c| {
                    let apply = c.apply.as_deref().unwrap_or(c.label.as_str());
                    CompletionItem {
                        label: c.label.to_string(),
                        kind: Some(completion_kind(&c.kind)),
                        detail: c.detail.as_ref().map(|d| d.to_string()),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range,
                            new_text: strip_snippet_placeholders(apply),
                        })),
                        ..Default::default()
                    }
                })
                .collect();
            Ok(CompletionResponse::Array(items))
        })
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        // typst-ide decides from the syntax context whether a completion
        // applies; just skip whitespace and multi-char pastes.
        let mut chars = new_text.chars();
        matches!((chars.next(), chars.next()), (Some(c), None) if !c.is_whitespace())
    }
}

impl HoverProvider for TypstLsp {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<Hover>>> {
        let Some(path) = self.active_path.borrow().clone() else {
            return Task::ready(Ok(None));
        };
        let compiler = self.compiler.clone();
        let source = text.to_string();

        cx.background_executor().spawn(async move {
            let value = match typst_core::ide::tooltip(&compiler, &path, &source, offset) {
                Some(Tooltip::Text(text)) => text.to_string(),
                Some(Tooltip::Code(code)) => format!("```typst\n{code}\n```"),
                None => return Ok(None),
            };
            Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                }),
                range: None,
            }))
        })
    }
}

fn completion_kind(kind: &CompletionKind) -> CompletionItemKind {
    match kind {
        CompletionKind::Syntax => CompletionItemKind::SNIPPET,
        CompletionKind::Func => CompletionItemKind::FUNCTION,
        CompletionKind::Type => CompletionItemKind::CLASS,
        CompletionKind::Param => CompletionItemKind::VARIABLE,
        CompletionKind::Constant => CompletionItemKind::CONSTANT,
        CompletionKind::Path => CompletionItemKind::FILE,
        CompletionKind::Package => CompletionItemKind::MODULE,
        CompletionKind::Label => CompletionItemKind::REFERENCE,
        CompletionKind::Font => CompletionItemKind::TEXT,
        CompletionKind::Symbol(_) => CompletionItemKind::TEXT,
    }
}

/// typst-ide snippets use `${name}` placeholders; the gpui-kit editor has no
/// snippet engine, so drop the placeholders and keep the literal text.
fn strip_snippet_placeholders(apply: &str) -> String {
    let mut out = String::with_capacity(apply.len());
    let mut rest = apply;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        match rest[start..].find('}') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::strip_snippet_placeholders;

    #[test]
    fn strips_placeholders() {
        assert_eq!(strip_snippet_placeholders("heading(${body})"), "heading()");
        assert_eq!(strip_snippet_placeholders("#let ${x} = ${y}"), "#let  = ");
        assert_eq!(strip_snippet_placeholders("plain"), "plain");
    }
}
