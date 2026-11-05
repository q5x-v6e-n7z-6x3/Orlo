//! Current-tree selection and exact view-local reversal; no pending result can move newer carets.

use super::*;
use crate::editing::text::SelState;

pub(super) struct History {
    document: u64,
    rope: ropey::Rope,
    version: u64,
    source: (u64, u64),
    expected: SelState,
    generation: u64,
    previous: Vec<SelState>,
}

impl Editor {
    fn syntax_selection_guard(&mut self, cx: &mut Context<Self>) {
        self.catch_up(cx);
        self.sel(cx).sel_checkpoint();
        if self.syntax_selection.as_ref().is_some_and(|h| {
            let doc = self.doc(cx);
            doc.id() != h.document
                || doc.version != h.version
                || !h.rope.is_instance(&doc.rope)
                || self.highlighter.syntax_selection_source(&doc.rope) != Some(h.source)
                || self.read_state(cx).selection_generation() != h.generation
                || self.read_state(cx).selection_state() != h.expected
        }) {
            self.syntax_selection = None;
        }
    }

    pub(super) fn expand_syntax_selection(
        &mut self,
        _: &ExpandSyntaxSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.syntax_selection_guard(cx);
        let rope = self.doc(cx).rope.clone();
        let Some(source) = self.highlighter.syntax_selection_source(&rope) else {
            self.syntax_selection = None;
            return self.status("Syntax selection is unavailable for this language or document size", cx);
        };
        let before = self.read_state(cx).selection_state();
        let mut expanded = Vec::with_capacity(before.selections.len());
        for selection in &before.selections {
            let Some(range) = self.highlighter.enclosing_named_range(&rope, selection.range()) else {
                self.syntax_selection = None;
                let line = rope.char_to_line(selection.head.min(rope.len_chars()));
                self.highlight_in_background(line, line, cx);
                return self.status("Preparing syntax selection; run Expand Syntax Selection again", cx);
            };
            expanded.push(match range {
                Some(range) if selection.anchor_point() > selection.head_point() => {
                    Selection::new(range.end, range.start)
                }
                Some(range) => Selection::new(range.start, range.end),
                None => *selection,
            });
        }
        if expanded == before.selections {
            return self.status("No larger named syntax range", cx);
        }
        let mut history = self.syntax_selection.take().unwrap_or_else(|| History {
            document: self.doc(cx).id(),
            rope,
            version: self.doc(cx).version,
            source,
            expected: before.clone(),
            generation: self.read_state(cx).selection_generation(),
            previous: Vec::new(),
        });
        self.edit(cx, |s| s.select_preserving_primary(expanded));
        self.sel(cx).sel_checkpoint();
        history.previous.push(before);
        history.expected = self.read_state(cx).selection_state();
        history.generation = self.read_state(cx).selection_generation();
        self.syntax_selection = Some(history);
    }

    pub(super) fn shrink_syntax_selection(
        &mut self,
        _: &ShrinkSyntaxSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.syntax_selection_guard(cx);
        let Some(mut history) = self.syntax_selection.take() else {
            return self.status("No syntax selection to shrink", cx);
        };
        // A cancelled/stale parse cannot authorize a reversal either.
        if self.highlighter.enclosing_named_range(&self.doc(cx).rope, 0..0).is_none() {
            return self.status("Syntax selection is not current; run Expand Syntax Selection again", cx);
        }
        let Some(before) = history.previous.pop() else {
            return self.status("No syntax selection to shrink", cx);
        };
        self.edit(cx, |s| s.restore_selection_state(before));
        history.expected = self.read_state(cx).selection_state();
        history.generation = self.read_state(cx).selection_generation();
        if !history.previous.is_empty() {
            self.syntax_selection = Some(history);
        }
    }
}
