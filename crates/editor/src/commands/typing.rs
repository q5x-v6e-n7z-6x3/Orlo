use crate::buffer::*;

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    pub(crate) fn pair_of(c: char) -> Option<char> {
        match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' | '\'' => Some(c),
            _ => None,
        }
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    // ---- edits ----

    /// Switching keyboard modes ends a typing group, without changing text or history.
    pub fn toggle_overtype(&mut self) {
        self.set_typing(false);
        self.view.overtype = !self.view.overtype;
    }

    /// Literal keyboard text, without auto-pairs. Clipboard and structural edits use `insert`.
    pub fn type_literal(&mut self, text: &str) {
        if self.doc.edits_blocked() {
            return;
        }
        if !self.view.overtype || text.contains(['\r', '\n', '\t']) || text.is_empty() {
            return self.insert(text);
        }
        let count = text.graphemes(true).count();
        self.record();
        self.set_typing(true);
        self.materialize_virtual();
        self.edit(|s, sel| {
            if !sel.is_empty() {
                return (sel.range(), text.to_string());
            }
            let start = s.doc.snap_to_cluster(sel.start());
            // Count new clusters in their left context: an accent, emoji continuation or
            // second regional indicator can extend the preceding cluster without replacing
            // any of the source ahead. This is decided separately at every caret.
            let count = if start == 0 {
                count
            } else {
                let mut joined = s.doc.rope.slice(s.prev_cluster(start)..start).to_string();
                let before = joined.graphemes(true).count();
                joined.push_str(text);
                joined.graphemes(true).count().saturating_sub(before)
            };
            let mut end = start;
            for _ in 0..count {
                if end == s.len() || matches!(s.doc.rope.char(end), '\r' | '\n') {
                    break;
                }
                end = s.next_cluster(end);
            }
            (start..end, text.to_string())
        });
    }

    pub fn insert(&mut self, text: &str) {
        if self.doc.edits_blocked() {
            return;
        }
        let typing = !text.contains('\n');
        if !typing {
            self.set_typing(false);
        }
        self.record();
        self.set_typing(typing);
        if !text.is_empty() {
            self.materialize_virtual();
        }
        self.edit(|_, sel| (sel.range(), text.to_string()));
    }

    /// Keyboard typing: Sublime's auto-pairing of brackets and quotes on top of `insert`.
    /// Each caret decides for itself; Sublime pairs only when every caret agrees.
    pub fn type_text(&mut self, text: &str) {
        if self.doc.edits_blocked() {
            return;
        }
        if self.view.overtype {
            return self.type_literal(text);
        }
        let mut it = text.chars();
        let (Some(ch), None) = (it.next(), it.next()) else {
            return self.insert(text);
        };
        let close = Self::pair_of(ch);
        if close.is_none() && !matches!(ch, ')' | ']' | '}') {
            return self.insert(text);
        }
        let quote = matches!(ch, '"' | '\'');
        self.record();
        self.set_typing(true);
        self.materialize_virtual();
        self.edit_with(|s, _, sel| {
            let (a, b) = (sel.start(), sel.end());
            let prev = (a > 0).then(|| s.doc.rope.char(a - 1));
            let next = (b < s.len()).then(|| s.doc.rope.char(b));
            let plain = (a..b, ch.to_string(), Selection::caret(a + 1));
            let step_over = (a..a, String::new(), Selection::caret(a + 1));
            let Some(close) = close else {
                return if sel.is_empty() && next == Some(ch) { step_over } else { plain };
            };
            if !sel.is_empty() {
                let inner = s.doc.rope.slice(a..b).to_string();
                return (a..b, format!("{ch}{inner}{close}"), Selection::new(a + 1, b + 1));
            }
            if quote && next == Some(ch) {
                return step_over;
            }
            // at a line's end there is room too, with lines below or not
            let room = next.is_none_or(|n| if quote { " \t\r\n)]}>" } else { " \t\r\n)];}" }.contains(n));
            let glued = quote && prev.is_some_and(|p| Self::is_word_char(p) || p == ch);
            if room && !glued {
                (a..a, format!("{ch}{close}"), Selection::caret(a + 1))
            } else {
                plain
            }
        });
    }

    pub fn backspace(&mut self) {
        self.set_typing(false);
        self.record();
        for sel in &mut self.view.selections {
            if sel.is_empty() && sel.head_virtual > 0 {
                // Select one imaginary space, so the shared deletion path collapses it.
                sel.anchor_virtual -= 1;
            }
        }
        self.edit(|s, sel| {
            if !sel.is_empty() || sel.head_virtual > 0 {
                return (sel.range(), String::new());
            }
            let a = sel.start();
            // in leading spaces, back to the previous tab stop (Sublime, with spaces for tabs)
            let (line, col) = s.point(a);
            let ls = s.line_start(line);
            if s.doc.translate_tabs && col > 1 && s.doc.rope.slice(ls..a).chars().all(|c| c == ' ') {
                let n = match col % s.indent_width() {
                    0 => s.indent_width(),
                    r => r,
                };
                return (a - n.min(col)..a, String::new());
            }
            let prev = (a > 0).then(|| s.doc.rope.char(a - 1));
            let next = (a < s.len()).then(|| s.doc.rope.char(a));
            // between an auto-inserted pair both halves go
            if let (Some(p), Some(n)) = (prev, next) {
                if Self::pair_of(p) == Some(n) {
                    return (a - 1..a + 1, String::new());
                }
            }
            // CRLF is one line break even when raw text was inserted into the buffer.
            // Removing only LF would turn its hidden CR into content on the joined line.
            let start = if a >= 2 && prev == Some('\n') && s.doc.rope.char(a - 2) == '\r' {
                a - 2
            } else {
                a.saturating_sub(1)
            };
            (start..a, String::new())
        });
    }

    pub fn delete(&mut self) {
        self.set_typing(false);
        self.record();
        self.edit(|s, sel| match sel.is_empty() {
            // a whole emoji or accented letter, as Sublime's right_delete (backspace takes one
            // code point)
            true if sel.head_virtual > 0 => (sel.range(), String::new()),
            true => (sel.start()..s.next_cluster(sel.start()), String::new()),
            false => (sel.range(), String::new()),
        });
    }

    pub fn newline(&mut self) {
        self.newline_with_language(None);
    }

    pub fn newline_with_language(&mut self, language: Option<super::indentation::IndentLanguage>) {
        self.newline_indented(language, |_, _, _| None);
    }

    /// Enter, with `levels(start, end, blank)` the indent levels a language's syntax tree gives the
    /// line a break replacing `start..end` starts (`blank`: the line stays empty, a brace going
    /// below it); where it gives none, today's rules.
    pub fn newline_indented(
        &mut self,
        language: Option<super::indentation::IndentLanguage>,
        levels: impl Fn(usize, usize, bool) -> Option<usize>,
    ) {
        self.set_typing(false);
        self.record();
        self.materialize_virtual();
        self.edit_with(|s, _, sel| {
            let (line, col) = s.point(sel.start());
            let indent: String = (s.line(line).chars()).take(col).take_while(|c| *c == ' ' || *c == '\t').collect();
            let prev = (sel.start() > 0).then(|| s.doc.rope.char(sel.start() - 1));
            let next = (sel.end() < s.len()).then(|| s.doc.rope.char(sel.end()));
            let context = language.and_then(|language| {
                if sel.start() > super::indentation::LIMIT {
                    return None;
                }
                let prefix = s.doc.rope.slice(..sel.start()).to_string();
                let mut scanner = super::indentation::Context::new(language);
                let mut last = None;
                for line in prefix.split('\n') {
                    last = Some(scanner.line(line));
                }
                scanner.reliable().then_some(()).and_then(|_| last.map(|line| (language, line)))
            });
            // enter between `{` and `}` pushes the brace onto its own line
            let pushed = prev == Some('{') && next == Some('}') && context.as_ref().is_none_or(|(_, line)| line.active);
            let tree = levels(sel.start(), sel.end(), pushed).map(|n| s.indentation(n * s.indent_width()));
            let next_indent = if let Some(tree) = tree {
                tree
            } else if let Some((language, line)) = &context {
                let columns = super::line_editing::columns(&indent, s.doc.tab_size);
                s.indentation(super::indentation::next_columns(*language, line, columns, s.indent_width()))
            } else if matches!(prev, Some('{' | '(' | '[')) {
                if s.doc.indent_size.is_some() {
                    let columns = super::line_editing::columns(&indent, s.doc.tab_size);
                    s.indentation(columns + s.indent_width())
                } else {
                    format!("{indent}{}", s.indent_unit())
                }
            } else {
                indent.clone()
            };
            let mut text = format!("\n{next_indent}");
            let caret = Selection::caret(sel.start() + text.chars().count());
            if pushed {
                let closing = levels(sel.start(), sel.end(), false).map(|n| s.indentation(n * s.indent_width()));
                text.push('\n');
                text.push_str(closing.as_deref().unwrap_or(&indent));
            }
            (sel.range(), text, caret)
        });
    }
}
