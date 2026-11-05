use crate::buffer::*;

/// The columns `text` spans from the start of a line, tabs to their stops.
pub(crate) fn columns(text: &str, tab: usize) -> usize {
    col_after(text.into(), tab)
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    // ---- line commands ----

    /// Lines a selection covers; a non-empty selection ending at column 0 stops short of that line.
    pub fn covered_lines(&self, sel: &Selection) -> (usize, usize) {
        let (l0, _) = self.point(sel.start());
        let (l1, c1) = self.point(sel.end());
        (l0, l1 - usize::from(!sel.is_empty() && c1 == 0 && l1 > l0))
    }

    /// Sorted, non-overlapping line blocks covered by the selections; touching blocks merge.
    pub(crate) fn line_blocks(&self) -> Vec<(usize, usize)> {
        self.blocks_covering(self.view.selections.iter())
    }

    pub(crate) fn blocks_covering<'a>(&self, selections: impl Iterator<Item = &'a Selection>) -> Vec<(usize, usize)> {
        let mut blocks: Vec<(usize, usize)> = selections.map(|s| self.covered_lines(s)).collect();
        blocks.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (a, b) in blocks {
            match merged.last_mut() {
                Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        merged
    }

    // ---- clipboard ----

    /// The selected texts joined by newlines. With every selection empty it is their lines
    /// instead, each once, and the flag is true: `paste` puts them back as whole lines. A
    /// caret beside a selection adds nothing (Sublime).
    pub fn copy_text(&self) -> (String, bool) {
        let whole_lines = self.view.selections.iter().all(|s| s.is_empty() && !s.has_virtual());
        if whole_lines {
            let mut lines: Vec<usize> = (self.view.selections.iter()).map(|s| self.point(s.head).0).collect();
            lines.dedup();
            return (lines.iter().map(|&l| self.line(l) + "\n").collect(), true);
        }
        let mut texts = self.selected_texts();
        // a caret's empty text is no line of its own
        texts.retain(|t| !t.is_empty());
        (texts.join("\n"), false)
    }
}

impl<D: DocRef, V: ViewMut> Editing<D, V> {
    /// Carets on one line become one, as Sublime's Add Line macros move every caret to the
    /// line's edge first.
    pub(crate) fn one_caret_per_line(&mut self, at_end: bool) {
        let sels = (self.view.selections.iter())
            .map(|s| {
                let (l, _) = self.point(if at_end { s.end() } else { s.start() });
                Selection::caret(match at_end {
                    true => self.line_end(l),
                    false => self.line_start(l),
                })
            })
            .collect();
        self.view.selections = sels;
        self.normalize();
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    /// Rewrite every covered line with `f`, keeping selections on the same characters.
    pub(crate) fn map_lines(&mut self, f: impl Fn(&str) -> String) {
        if self.doc.edits_blocked() {
            return;
        }
        self.set_typing(false);
        self.record();
        let lines: Vec<usize> = (self.line_blocks().into_iter()).flat_map(|(a, b)| a..=b).collect();
        let points: Vec<((usize, usize), (usize, usize))> =
            (self.view.selections.iter()).map(|s| (self.point(s.anchor), self.point(s.head))).collect();
        let mut deltas: HashMap<usize, isize> = HashMap::new();
        for &l in lines.iter().rev() {
            let old = self.line(l);
            let new = f(&old);
            if new != old {
                let start = self.line_start(l);
                self.splice(start..start + old.chars().count(), &new);
                let delta = new.chars().count() as isize - old.chars().count() as isize;
                deltas.insert(l, delta);
                self.touch(l);
            }
        }
        let adjust = |(l, c): (usize, usize)| {
            let c = (c as isize + deltas.get(&l).copied().unwrap_or(0)).max(0) as usize;
            self.offset(l, c)
        };
        let sels: Vec<Selection> = points.iter().map(|(a, h)| Selection::new(adjust(*a), adjust(*h))).collect();
        self.view.selections = sels;
        self.normalize();
    }

    /// Recompute selected leading whitespace from a bounded source snapshot. Returns false
    /// if the source prefix is too large to establish lexical context reliably.
    pub fn reindent_selection(&mut self, language: super::indentation::IndentLanguage) -> bool {
        use super::indentation::{dedent_keyword, next_columns_active, Context, IndentLanguage, LIMIT};
        if self.doc.edits_blocked() {
            return false;
        }
        let blocks = self.line_blocks();
        let Some(last) = blocks.last().map(|block| block.1) else {
            return true;
        };
        let end = self.line_end(last);
        if end > LIMIT {
            return false;
        }
        let width = self.indent_width();
        let mut scanner = Context::new(language);
        let mut next = 0usize;
        let mut previous_exited = false;
        // Candidate headers let an already-aligned outer else retain its meaning when
        // the immediately preceding statement exits an inner block.
        let mut headers: Vec<(usize, &str)> = Vec::new();
        let mut edits = Vec::new();
        for l in 0..=last {
            let text = self.line(l);
            let content = text.trim_start_matches([' ', '\t']);
            let lead = &text[..text.len() - content.len()];
            let parsed = scanner.line(&text);
            if !scanner.reliable() {
                return false;
            }
            let selected = blocks.iter().any(|&(a, b)| (a..=b).contains(&l));
            let closes = parsed.code.trim_start().chars().take_while(|c| matches!(c, ')' | ']' | '}')).count();
            let actual = columns(lead, self.doc.tab_size);
            let word = super::indentation::first_word(&parsed.code);
            let branch = language == IndentLanguage::Python && dedent_keyword(&parsed.code);
            let mut target = next.saturating_sub(closes * width);
            let matching = |kind: &str| match word {
                "elif" => kind == "if",
                "else" => matches!(kind, "if" | "for" | "while" | "try"),
                "except" | "finally" => kind == "try",
                _ => false,
            };
            let mut matched = None;
            if branch {
                matched = headers
                    .iter()
                    .rposition(|&(level, kind)| level == actual && matching(kind))
                    .or_else(|| headers.iter().rposition(|&(level, kind)| level <= next && matching(kind)));
                if let Some(i) = matched {
                    target = headers[i].0;
                } else if !previous_exited {
                    target = target.saturating_sub(width);
                }
            }
            let base = if parsed.starts_protected || (selected && !content.is_empty()) { target } else { actual };
            if selected && !content.is_empty() && !parsed.starts_protected {
                let replacement = self.indentation(base);
                if replacement != lead {
                    let start = self.line_start(l);
                    edits.push((start..start + lead.chars().count(), replacement));
                }
            }
            if !content.is_empty() && !parsed.code.trim().is_empty() {
                if language == IndentLanguage::Python {
                    if let Some(i) = matched {
                        let kind = headers[i].1;
                        headers.truncate(i);
                        headers.push((base, kind));
                    } else if !branch {
                        while headers.last().is_some_and(|(level, _)| *level >= base) {
                            headers.pop();
                        }
                        if parsed.code.trim_end().ends_with(':') && matches!(word, "if" | "for" | "while" | "try") {
                            headers.push((
                                base,
                                match word {
                                    "if" => "if",
                                    "for" => "for",
                                    "while" => "while",
                                    _ => "try",
                                },
                            ));
                        }
                    }
                }
                next = next_columns_active(language, &parsed, base, width);
                // A leading closer was already subtracted above. An opener later on the
                // same line, such as `} else {`, still opens a new block.
                let opened = parsed.after_depth.saturating_add(closes).saturating_sub(parsed.before_depth);
                if closes > 0 {
                    let colon =
                        usize::from(language == IndentLanguage::Python && parsed.code.trim_end().ends_with(':'));
                    next = base + opened.max(colon) * width;
                }
                previous_exited = language == IndentLanguage::Python && super::indentation::exits_block(&parsed.code);
            }
        }
        if edits.is_empty() {
            return true;
        }
        if edits.iter().any(|(r, _)| {
            r.end > self.len() || !self.doc.rope.slice(r.clone()).chars().all(|c| matches!(c, ' ' | '\t'))
        }) {
            return false;
        }
        self.set_typing(false);
        self.record();
        let n = self.view.changes.len();
        for (range, replacement) in edits.iter().rev() {
            self.splice(range.clone(), replacement);
        }
        let changes = self.view.changes[n..].to_vec();
        self.map_selections(&changes, false);
        self.touch(blocks[0].0);
        self.normalize();
        true
    }

    fn reindent(&mut self, increase: bool) {
        let size = self.indent_width();
        let tab = self.doc.tab_size;
        let spaces = self.doc.translate_tabs;
        self.map_lines(|text| {
            if increase && text.is_empty() {
                return String::new();
            }
            let content = text.trim_start_matches([' ', '\t']);
            let lead = &text[..text.len() - content.len()];
            let columns = columns(lead, tab);
            let columns = match increase {
                true => columns + size,
                false => columns.saturating_sub(size),
            };
            let lead = indentation(columns, tab, spaces);
            format!("{lead}{content}")
        });
    }

    pub fn indent(&mut self) {
        if self.doc.indent_size.is_some() {
            return self.reindent(true);
        }
        let unit = self.indent_unit();
        self.map_lines(|t| match t.is_empty() {
            true => String::new(),
            false => format!("{unit}{t}"),
        });
    }

    pub fn unindent(&mut self) {
        let size = self.indent_width();
        if self.doc.indent_size.is_some() {
            return self.reindent(false);
        }
        self.map_lines(|t| {
            if let Some(rest) = t.strip_prefix('\t') {
                return rest.to_string();
            }
            let n = t.chars().take_while(|c| *c == ' ').count().min(size);
            t.chars().skip(n).collect()
        });
    }

    /// Tab indents when a selection spans lines, otherwise inserts spaces.
    pub fn tab(&mut self) {
        let spans_lines =
            (self.view.selections.iter()).any(|s| !s.is_empty() && self.point(s.start()).0 != self.point(s.end()).0);
        if spans_lines {
            self.indent();
            return;
        }
        // spaces up to the next tab stop, like Sublime with translate_tabs_to_spaces
        self.record();
        self.set_typing(true);
        self.materialize_virtual();
        // the line as the carets before this one on it left it, and how many chars they added
        let mut cur: Option<(usize, String, isize)> = None;
        self.edit(|s, sel| {
            if !s.doc.translate_tabs && s.doc.indent_size.is_none() {
                return (sel.range(), "\t".into());
            }
            let (line, col) = s.point(sel.start());
            if cur.as_ref().is_some_and(|c| c.0 != line) {
                cur = None;
            }
            let (_, text, shift) = cur.get_or_insert_with(|| (line, s.line(line), 0));
            let at = (col as isize + *shift) as usize;
            let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(b, _)| b);
            let col = col_after(text[..byte(at)].into(), s.doc.tab_size);
            let width = s.indent_width() - col % s.indent_width();
            let pad = if s.doc.translate_tabs {
                " ".repeat(width)
            } else {
                let mut pad = String::new();
                let mut current = col;
                while s.doc.tab_size - current % s.doc.tab_size <= col + width - current {
                    current += s.doc.tab_size - current % s.doc.tab_size;
                    pad.push('\t');
                }
                pad.push_str(&" ".repeat(col + width - current));
                pad
            };
            let len = sel.range().len();
            let (b0, b1) = (byte(at), byte(at + len));
            text.replace_range(b0..b1, &pad);
            *shift += pad.len() as isize - len as isize;
            (sel.range(), pad)
        });
    }

    /// Sublime's Toggle Comment: when every line has the marker it comes off each line
    /// (with one space after it); else each selection's lines get it at their smallest indent
    /// (a caret's line at its own), blank lines in a selection skipped. Offsets at the marker
    /// column go past it, so toggling back restores them.
    pub fn toggle_comment(&mut self, prefix: &str) {
        if self.doc.edits_blocked() {
            return;
        }
        self.set_typing(false);
        self.record();
        let blank = |s: &Self, l: usize| s.line(l).trim().is_empty();
        let indent = |s: &Self, l: usize| s.line(l).chars().take_while(|c| *c == ' ' || *c == '\t').count();
        // (lines, marker column) per selection, each line once
        let mut seen = HashSet::new();
        let mut regions: Vec<(Vec<usize>, usize)> = Vec::new();
        for sel in &self.view.selections {
            let (a, b) = self.covered_lines(sel);
            let lines: Vec<usize> =
                (a..=b).filter(|&l| (sel.is_empty() || !blank(self, l)) && seen.insert(l)).collect();
            let col = |l: usize| {
                let start = self.line_start(l);
                col_after(self.doc.rope.slice(start..start + indent(self, l)), self.doc.tab_size)
            };
            let min = lines.iter().map(|&l| col(l)).min().unwrap_or(0);
            regions.push((lines, min));
        }
        let all: Vec<usize> = regions.iter().flat_map(|r| r.0.iter().copied()).collect();
        let commented = |l: usize| (self.line(l).trim_start_matches([' ', '\t'])).starts_with(prefix);
        let remove = !all.is_empty() && all.iter().all(|&l| commented(l));
        // (offset, chars removed, text inserted), applied from the end
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        for (lines, min) in &regions {
            for &l in lines {
                let start = self.line_start(l);
                let t = self.line(l);
                if remove {
                    let at = indent(self, l);
                    let after: String = t.chars().skip(at + prefix.chars().count()).collect();
                    let n = prefix.chars().count() + usize::from(after.starts_with(' '));
                    edits.push((start + at, n, String::new()));
                } else {
                    let at = char_at(self.doc.rope.slice(start..start + t.chars().count()), self.doc.tab_size, *min);
                    edits.push((start + at, 0, format!("{prefix} ")));
                }
            }
        }
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        let n = self.view.changes.len();
        for (at, removed, text) in &edits {
            self.splice(*at..at + removed, text);
        }
        if self.view.changes.len() > n {
            let changes = self.view.changes[n..].to_vec();
            self.map_selections(&changes, true);
            let first = all.iter().min().copied().unwrap_or(0);
            self.touch(first);
        }
        self.normalize();
    }

    /// Empty selection: copy the line below itself and move the caret down with it.
    /// Non-empty: insert the selected text right after itself and select the copy (Sublime).
    pub fn duplicate_line(&mut self) {
        self.set_typing(false);
        self.record();
        self.edit_with(|s, _, sel| {
            if sel.is_empty() {
                let (l, c) = s.point(sel.head);
                let end = s.line_end_raw(l);
                let text = format!("\n{}", s.line_raw(l));
                (end..end, text, Selection::caret(end + 1 + c))
            } else {
                let e = sel.end();
                let text = s.doc.rope.slice(sel.range()).to_string();
                let n = text.chars().count();
                (e..e, text, Selection::new(e, e + n))
            }
        });
    }

    pub fn delete_line(&mut self) {
        if self.doc.edits_blocked() {
            return;
        }
        self.set_typing(false);
        self.record();
        self.normalize();
        let count = self.line_count();
        let ranges: Vec<Range<usize>> = (self.line_blocks().into_iter())
            .map(|(a, b)| {
                let mut start = self.line_start(a);
                let end = self.offset(b + 1, 0);
                // the last line has no newline of its own, so eat the one before it
                if b + 1 >= count && start > 0 {
                    start = self.off_cr_lf(start - 1);
                }
                start..end
            })
            .collect();
        if ranges.iter().all(|r| r.is_empty()) {
            return;
        }
        for r in ranges.iter().rev() {
            self.splice(r.clone(), "");
        }
        let mut removed = 0;
        self.view.selections = ranges
            .iter()
            .map(|r| {
                let caret = Selection::caret(r.start - removed);
                removed += r.end - r.start;
                caret
            })
            .collect();
        let first = ranges.first().map_or(0, |r| r.start);
        self.touch(self.doc.rope.char_to_line(first.min(self.len())));
        self.normalize();
    }

    pub fn move_line_up(&mut self) {
        self.move_lines(true);
    }

    pub fn move_line_down(&mut self) {
        self.move_lines(false);
    }

    pub(crate) fn move_lines(&mut self, up: bool) {
        if self.doc.edits_blocked() {
            return;
        }
        self.set_typing(false);
        self.record();
        self.normalize();
        let mut blocks = self.line_blocks();
        if !up {
            blocks.reverse();
        }
        for (l0, l1) in blocks {
            let count = self.line_count();
            if (up && l0 == 0) || (!up && l1 + 1 >= count) {
                continue;
            }
            // lengths come from line starts, so a CR at a line's end moves with its line
            let bs = self.line_start(l0);
            let next = self.offset(l1 + 1, 0);
            // the neighbour line moves to the other side, so the moved lines keep their
            // bookmarks and other views' carets
            let shift = if up {
                let o = self.line_start(l0 - 1);
                let other = self.doc.rope.slice(o..bs).to_string();
                let n = other.chars().count();
                self.splice(o..bs, "");
                if l1 + 1 < count {
                    self.splice(next - n..next - n, &other);
                } else {
                    let end = self.len();
                    self.splice(end..end, &format!("\n{}", &other[..other.len() - 1]));
                }
                -(n as isize)
            } else {
                let other_end = self.offset(l1 + 2, 0);
                let other = self.doc.rope.slice(next..other_end).to_string();
                if l1 + 2 < count {
                    self.splice(next..other_end, "");
                    self.splice(bs..bs, &other);
                    other.chars().count() as isize
                } else {
                    // the last line has no line feed: the block's goes with it
                    self.splice(next - 1..other_end, "");
                    self.splice(bs..bs, &format!("{other}\n"));
                    other.chars().count() as isize + 1
                }
            };
            let first_line = l0.min(if up { l0 - 1 } else { l1 + 1 });
            for s in &mut self.view.selections {
                for p in [&mut s.anchor, &mut s.head] {
                    // `next` keeps a selection that ends at the next line's column 0 intact
                    if *p >= bs && *p <= next {
                        *p = (*p as isize + shift) as usize;
                    }
                }
            }
            self.touch(first_line);
        }
        self.normalize();
    }

    pub fn insert_line_after(&mut self) {
        self.set_typing(false);
        self.record();
        self.one_caret_per_line(true);
        self.edit_with(|s, _, sel| {
            let (l, _) = s.point(sel.end());
            let end = s.line_end(l);
            let text = format!("\n{}", s.indent_of(l));
            let caret = Selection::caret(end + text.chars().count());
            (end..end, text, caret)
        });
    }

    pub fn insert_line_before(&mut self) {
        self.set_typing(false);
        self.record();
        self.one_caret_per_line(false);
        self.edit_with(|s, _, sel| {
            let (l, _) = s.point(sel.start());
            let start = s.line_start(l);
            let indent = s.indent_of(l);
            let caret = Selection::caret(start + indent.chars().count());
            (start..start, format!("{indent}\n"), caret)
        });
    }

    /// `copy_text`'s text goes: the selections, or with all of them empty, their lines.
    pub fn cut(&mut self) -> (String, bool) {
        if self.doc.edits_blocked() {
            return (String::new(), false);
        }
        let text = self.copy_text();
        self.set_typing(false);
        self.record();
        if text.1 {
            for i in 0..self.view.selections.len() {
                let (l, _) = self.point(self.view.selections[i].head);
                self.view.selections[i] = Selection::new(self.line_start(l), self.offset(l + 1, 0));
            }
        }
        self.edit(|_, sel| (sel.range(), String::new()));
        text
    }

    /// With as many clipboard lines as cursors, each cursor gets its own line (Sublime).
    /// `whole_line` text (see `copy_text`) goes in above each caret's line instead.
    /// Always its own undo step: typing on either side does not merge into it.
    pub fn paste(&mut self, text: &str, whole_line: bool) {
        let text = &text.replace("\r\n", "\n");
        let body = match whole_line {
            true => text.strip_suffix('\n').unwrap_or(text),
            false => text,
        };
        let lines: Vec<&str> = body.split('\n').collect();
        let per_caret = self.view.selections.len() > 1 && lines.len() == self.view.selections.len();
        let line_carets: Vec<_> = self.view.selections.iter().map(|s| whole_line && s.is_empty()).collect();
        let mut replacements: Vec<_> = (0..self.view.selections.len())
            .map(|i| match (per_caret, whole_line) {
                (true, true) => format!("{}\n", lines[i]),
                (true, false) => lines[i].to_string(),
                (false, _) => text.to_string(),
            })
            .collect();
        self.set_typing(false);
        self.record();
        if !text.is_empty() && self.view.selections.iter().any(Selection::has_virtual) {
            // Whole-line clipboard text inserts above an empty caret's line, but replaces
            // a selection at its actual start, including a wholly virtual selection.
            self.materialize_virtual_where(|i| !line_carets[i] && !replacements[i].is_empty());
        }
        self.edit_with(|s, i, sel| {
            let t = std::mem::take(&mut replacements[i]);
            let n = t.chars().count();
            if t.is_empty() && sel.range().is_empty() {
                return (sel.range(), t, Selection::from_points(sel.start_point(), sel.start_point()));
            }
            if line_carets[i] {
                let start = s.line_start(s.point(sel.head).0);
                (start..start, t, Selection { anchor: sel.head + n, head: sel.head + n, ..*sel })
            } else {
                (sel.range(), t, Selection::caret(sel.start() + n))
            }
        });
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    /// The block comment around `r` (its start inside the comment, its end not past it), as
    /// the replacement that takes its markers off, `sel` mapped through it: the nearest opener
    /// at or before `r`, then the first closer after it. Markers with their inner spaces
    /// (`<!-- `) or without. By the text, not the syntax's comment scope as Sublime, so markers
    /// inside a string count, and it looks at most 4096 chars either way; scopes would need the
    /// highlighter here.
    pub(crate) fn block_comment_around(
        &self,
        r: Range<usize>,
        sel: &Selection,
        open: &str,
        close: &str,
    ) -> Option<(Range<usize>, String, Selection)> {
        const LOOK: usize = 4096;
        let (o, c) = (open.trim(), close.trim());
        // the markers are looked for by byte in the text around `r`, copied
        let (lo, hi) = (r.start.saturating_sub(LOOK), (r.end + LOOK).min(self.len()));
        let base = self.doc.rope.char_to_byte(lo);
        let rb = self.doc.rope.char_to_byte(r.start.min(hi)) - base;
        // the nearest opener starting at or before `r`; a closer between it and `r`: not in
        // one. A long selection's text before it is looked at alone first (a select-all
        // toggled on has no opener, and is not copied whole for nothing)
        let mid = match r.len() > LOOK {
            true => (r.start + o.chars().count()).min(hi),
            false => hi,
        };
        let mut text = self.doc.rope.slice(lo..mid).to_string();
        let upto = text.ceil_char_boundary((rb + o.len()).min(text.len()));
        let sb = (text[..upto].rmatch_indices(o)).find(|(b, _)| *b <= rb)?.0;
        if text[..rb].rfind(c).is_some_and(|b| b > sb) {
            return None;
        }
        let rest = self.doc.rope.slice(mid..hi);
        text.reserve(rest.len_bytes());
        rest.chunks().for_each(|ch| text.push_str(ch));
        let eb = sb + o.len() + text[sb + o.len()..].find(c)? + c.len();
        let (s, e) = (lo + text[..sb].chars().count(), lo + text[..eb].chars().count());
        if r.start >= e || r.end > e {
            return None;
        }
        // the inner spaces of `<!-- ` and ` -->` go too when they are there
        let ob = match text[sb..].starts_with(open) {
            true => open.len(),
            false => o.len(),
        };
        let cb = match eb >= sb + ob + close.len() && text[..eb].ends_with(close) {
            true => close.len(),
            false => c.len(),
        };
        let (ol, cl) = (open[..ob].chars().count(), close[close.len() - cb..].chars().count());
        let ol = if ob == o.len() { o.chars().count() } else { ol };
        let cl = if cb == c.len() { c.chars().count() } else { cl };
        let inner = text[sb + ob..eb - cb].to_string();
        // as erasing the markers moves them: into the gap where a marker was
        let map = |p: usize| match p {
            p if p < s => p,
            p if p >= e => p - ol - cl,
            p => p.clamp(s + ol, e - cl) - ol,
        };
        Some((s..e, inner, Selection::new(map(sel.anchor), map(sel.head))))
    }

    /// The spaces and tabs at the end of line `l`; a no-break or ideographic space is text.
    pub(crate) fn trailing_space(&self, l: usize) -> Range<usize> {
        let (start, end) = (self.line_start(l), self.line_end(l));
        let mut chars = self.doc.rope.chars_at(end);
        let mut n = 0;
        while n < end - start && matches!(chars.prev(), Some(' ' | '\t')) {
            n += 1;
        }
        end - n..end
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    /// Toggle Comment in a syntax with only block comments (CSS, HTML, XML, Markdown), as
    /// Sublime 4215 does: a selection or caret inside a block comment takes its
    /// markers off; else a caret's or one-line selection's line is wrapped from its indent, and a
    /// selection over several lines is wrapped as it is (a line break at its end left out).
    pub fn toggle_comment_with_block(&mut self, open: &str, close: &str) {
        self.set_typing(false);
        self.record();
        let (on, cn) = (open.chars().count(), close.chars().count());
        self.edit_with(|s, _, sel| {
            if let Some(c) = s.block_comment_around(sel.range(), sel, open, close) {
                return c;
            }
            let (l0, l1) = s.covered_lines(sel);
            let range = match l0 == l1 {
                true => {
                    let t = s.line(l0);
                    let indent = t.chars().take_while(|c| *c == ' ' || *c == '\t').count();
                    let from = s.line_start(l0) + indent;
                    // a comment starting the line comes off, wherever the selection is
                    if let Some(c) = s.block_comment_around(from..from, sel, open, close) {
                        return c;
                    }
                    from..s.line_end(l0)
                }
                false => sel.start()..s.line_end(l1).min(sel.end()),
            };
            let text = s.doc.rope.slice(range.clone()).to_string();
            // as Sublime's inserts move them: past the opener from its start, past the closer
            // only from beyond the end
            let map = |p: usize| match p {
                p if p < range.start => p,
                p if p <= range.end => p + on,
                p => p + on + cn,
            };
            let keep = Selection::new(map(sel.anchor), map(sel.head));
            (range, format!("{open}{text}{close}"), keep)
        });
    }

    pub fn toggle_block_comment(&mut self, open: &str, close: &str) {
        self.set_typing(false);
        self.record();
        self.edit_with(|s, _, sel| {
            let text = s.doc.rope.slice(sel.range()).to_string();
            let a = sel.start();
            if let Some(inner) = text.strip_prefix(open).and_then(|t| t.strip_suffix(close)) {
                let end = a + inner.chars().count();
                return (sel.range(), inner.to_string(), Selection::new(a, end));
            }
            let (on, n) = (open.chars().count(), text.chars().count());
            let keep = if sel.is_empty() { Selection::caret(a + on) } else { Selection::new(a + on, a + on + n) };
            (sel.range(), format!("{open}{text}{close}"), keep)
        });
    }

    /// Trim Trailing Whitespace, or with `remove_blank` Remove Blank Lines (white space alone
    /// counts as blank): on the selected lines, or the whole buffer when every selection is a
    /// caret.
    pub fn clean_whitespace(&mut self, remove_blank: bool) {
        if self.doc.edits_blocked() {
            return;
        }
        let blocks = if self.view.selections.iter().any(|s| !s.is_empty()) {
            self.line_blocks()
        } else {
            vec![(0, self.line_count() - 1)]
        };
        self.set_typing(false);
        self.record();
        let n = self.view.changes.len();
        let mut first = None;
        for (a, b) in blocks.into_iter().rev() {
            for l in (a..=b).rev() {
                let start = self.line_start(l);
                let ws = self.trailing_space(l);
                let range = match remove_blank {
                    true if ws.start != start => continue,
                    true => start..self.offset(l + 1, 0),
                    false => ws,
                };
                if !range.is_empty() {
                    self.splice(range, "");
                    first = Some(l);
                }
            }
        }
        if let Some(line) = first {
            let changes = self.view.changes[n..].to_vec();
            self.map_selections(&changes, false);
            self.touch(line.min(self.line_count() - 1));
        }
        self.normalize();
    }

    /// Save-time cleanup: trailing white space, then blank lines at the end, then a final
    /// line break; one undo step. True when the text changed.
    pub fn cleanup_for_save(&mut self, c: &Cleanup) -> bool {
        if self.doc.edits_blocked() {
            return false;
        }
        if *c == Cleanup::default() {
            return false;
        }
        // where this cleanup's changes begin; the first one ends any typing group and starts the
        // undo step, so a cleanup with nothing to clean leaves both alone
        let mut began = None;
        let mut cut = |s: &mut Self, range: Range<usize>, text: &str| {
            if began.is_none() {
                s.set_typing(false);
                s.record();
                began = Some(s.view.changes.len());
            }
            s.splice(range, text);
        };
        if c.trim != Trim::None {
            let mut heads: Vec<usize> = self.view.selections.iter().map(|s| s.head).collect();
            heads.sort_unstable();
            for l in (0..self.line_count()).rev() {
                let ws = self.trailing_space(l);
                if ws.is_empty() {
                    continue;
                }
                // a caret in the white space, or at its start or end, is mid-typing
                let on_caret = c.trim == Trim::NotOnCaret && {
                    let i = heads.partition_point(|&h| h < ws.start);
                    heads.get(i).is_some_and(|&h| h <= ws.end)
                };
                if !on_caret {
                    cut(self, ws, "");
                }
            }
        }
        if c.remove_trailing_blank_lines {
            // what follows the last line with text goes, that line's own break stays
            let last = (0..self.line_count()).rev().find(|&l| self.trailing_space(l).start != self.line_start(l));
            let from = last.map_or(0, |l| self.offset(l + 1, 0));
            if from < self.len() {
                cut(self, from..self.len(), "");
            }
        }
        let end = self.len();
        if c.remove_final_newline && end > 0 {
            let mut from = end;
            while from > 0 && self.doc.rope.char(from - 1) == '\n' {
                from = self.off_cr_lf(from - 1);
            }
            if from < end {
                cut(self, from..end, "");
            }
        }
        let end = self.len();
        if c.ensure_newline && end > 0 && self.doc.rope.char(end - 1) != '\n' {
            cut(self, end..end, "\n");
        }
        let Some(n) = began else {
            return false;
        };
        let changes = self.view.changes[n..].to_vec();
        let first = changes.iter().map(|c| c.start).min().unwrap_or(0);
        self.map_selections(&changes, false);
        self.touch(self.doc.rope.char_to_line(first.min(self.len())));
        self.normalize();
        true
    }

    /// Convert leading tabs to spaces (or spaces to tabs) on every line.
    pub fn convert_indentation(&mut self, to_spaces: bool) {
        if self.doc.edits_blocked() {
            return;
        }
        self.set_typing(false);
        self.record();
        let size = self.doc.tab_size;
        let n = self.view.changes.len();
        let mut first = None;
        for l in (0..self.line_count()).rev() {
            let t = self.line(l);
            // spaces and tabs only: an ideographic space or NBSP is text
            let lead = &t[..t.len() - t.trim_start_matches([' ', '\t']).len()];
            let new_lead = indentation(columns(lead, size), size, to_spaces);
            if new_lead != lead {
                let start = self.line_start(l);
                self.splice(start..start + lead.chars().count(), &new_lead);
                first = Some(l);
            }
        }
        if let Some(l) = first {
            let changes = self.view.changes[n..].to_vec();
            self.map_selections(&changes, false);
            self.touch(l);
        }
        self.normalize();
    }
}
