use crate::buffer::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Case {
    Upper,
    Lower,
    Title,
    Swap,
    LowerCamel,
    UpperCamel,
    Snake,
    Kebab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permute {
    Sort { case_sensitive: bool },
    Reverse,
    Unique,
    Shuffle,
}

/// Sublime's default wrap width when no ruler or wrap_width is set.
pub const WRAP_WIDTH: usize = 78;
pub(crate) fn capitalise(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_uppercase().chain(c.flat_map(|c| c.to_lowercase())).collect()).unwrap_or_default()
}

/// "fooBar_baz-qux HTTPServer" -> foo Bar baz qux HTTP Server
pub(crate) fn ident_words(t: &str) -> Vec<String> {
    let chars: Vec<char> = t.chars().collect();
    let mut words = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1);
        let hump = c.is_uppercase()
            && (prev.is_some_and(|p| p.is_lowercase() || p.is_numeric())
                || (prev.is_some_and(|p| p.is_uppercase()) && next.is_some_and(|n| n.is_lowercase())));
        if hump && !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

pub fn convert_case(t: &str, case: Case) -> String {
    match case {
        Case::Upper => t.to_uppercase(),
        Case::Lower => t.to_lowercase(),
        Case::Swap => {
            let mut out = String::new();
            for c in t.chars() {
                match c.is_uppercase() {
                    true => out.extend(c.to_lowercase()),
                    false => out.extend(c.to_uppercase()),
                }
            }
            out
        }
        Case::Title => {
            let (mut out, mut start) = (String::new(), true);
            for c in t.chars() {
                match (c.is_alphanumeric(), start) {
                    (true, true) => out.extend(c.to_uppercase()),
                    (true, false) => out.extend(c.to_lowercase()),
                    (false, _) => out.push(c),
                }
                start = !c.is_alphanumeric();
            }
            out
        }
        Case::Snake => ident_words(t).join("_").to_lowercase(),
        Case::Kebab => ident_words(t).join("-").to_lowercase(),
        Case::UpperCamel => ident_words(t).iter().map(|w| capitalise(w)).collect(),
        Case::LowerCamel => ident_words(t)
            .iter()
            .enumerate()
            .map(|(i, w)| match i {
                0 => w.to_lowercase(),
                _ => capitalise(w),
            })
            .collect(),
    }
}

pub(crate) fn permute(items: Vec<String>, op: Permute) -> Vec<String> {
    match op {
        Permute::Sort { case_sensitive } => {
            let mut v = items;
            match case_sensitive {
                true => v.sort(),
                // lowercased once a line, not once a comparison
                false => v.sort_by_cached_key(|s| s.to_lowercase()),
            }
            v
        }
        Permute::Reverse => items.into_iter().rev().collect(),
        Permute::Unique => {
            let mut seen = HashSet::new();
            items.into_iter().filter(|s| seen.insert(s.clone())).collect()
        }
        Permute::Shuffle => {
            // xorshift seeded by the clock: good enough for shuffling lines
            let mut x = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0x9e3779b9, |d| d.as_nanos() as u64)
                | 1;
            let mut v = items;
            for i in (1..v.len()).rev() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                v.swap(i, (x % (i as u64 + 1)) as usize);
            }
            v
        }
    }
}

pub(crate) fn bracket_kind(c: char) -> Option<(usize, bool)> {
    match c {
        '(' => Some((0, true)),
        ')' => Some((0, false)),
        '[' => Some((1, true)),
        ']' => Some((1, false)),
        '{' => Some((2, true)),
        '}' => Some((2, false)),
        _ => None,
    }
}

/// Explicit surround targets share automatic typing's pair vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum Surround {
    #[serde(rename = "(", alias = ")")]
    Round,
    #[serde(rename = "[", alias = "]")]
    Square,
    #[serde(rename = "{", alias = "}")]
    Curly,
    #[serde(rename = "'")]
    SingleQuote,
    #[serde(rename = "\"")]
    DoubleQuote,
}

impl Surround {
    pub fn opener(self) -> char {
        match self {
            Self::Round => '(',
            Self::Square => '[',
            Self::Curly => '{',
            Self::SingleQuote => '\'',
            Self::DoubleQuote => '"',
        }
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    /// Prepare delimiter-only edits without touching IME, selections or history.
    pub fn surround_edits(
        &self,
        target: Surround,
        replacement: Option<Surround>,
        count: std::num::NonZeroUsize,
    ) -> Vec<(Range<usize>, String)> {
        if self.doc.edits_blocked() || replacement == Some(target) {
            return Vec::new();
        }
        let mut pairs = Vec::new();
        let mut brackets = Vec::new();
        let mut quoted_until = None;
        let mut escaped = false;
        for (at, c) in self.doc.rope.chars().enumerate() {
            if quoted_until.is_some_and(|end| at <= end) {
                continue;
            }
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if matches!(c, '\'' | '"') {
                let glued = at > 0 && {
                    let prev = self.doc.rope.char(at - 1);
                    Self::is_word_char(prev) || prev == c
                };
                if glued {
                    continue;
                }
                let mut escaped = false;
                let close = self.doc.rope.chars_at(at + 1).enumerate().find_map(|(offset, next)| {
                    if escaped {
                        escaped = false;
                    } else if next == '\\' {
                        escaped = true;
                    } else if next == c {
                        return Some(at + 1 + offset);
                    }
                    None
                });
                if let Some(close) = close {
                    if c == target.opener() {
                        pairs.push((at, close));
                    }
                    quoted_until = Some(close);
                }
            } else if let Some((kind, open)) = bracket_kind(c) {
                if open {
                    brackets.push((kind, at));
                } else if let Some((matching, start)) = brackets.pop() {
                    if matching == kind {
                        if self.doc.rope.char(start) == target.opener() {
                            pairs.push((start, at));
                        }
                    } else {
                        // Crossing/malformed pairs cannot manufacture an enclosing target.
                        brackets.clear();
                    }
                }
            }
        }
        let mut targets = Vec::new();
        for sel in &self.view.selections {
            if sel.range().is_empty() && (sel.anchor_virtual > 0 || sel.head_virtual > 0) {
                continue;
            }
            let starts_pair = sel.is_empty() && pairs.iter().any(|&(open, _)| open == sel.head);
            let mut enclosing: Vec<_> = pairs
                .iter()
                .copied()
                .filter(|&(open, close)| {
                    open <= sel.start() && sel.end() <= close + 1 && !(starts_pair && close + 1 == sel.head)
                })
                .collect();
            // At adjacent delimiter boundaries, prefer the pair starting at the caret.
            enclosing.sort_by_key(|&(open, close)| (close - open, std::cmp::Reverse(open)));
            if let Some(&pair) = enclosing.get(count.get() - 1) {
                targets.push(pair);
            }
        }
        targets.sort_unstable();
        targets.dedup();
        if targets.windows(2).any(|pair| pair[1].0 <= pair[0].1) {
            return Vec::new();
        }
        let new = replacement.map(|pair| {
            let open = pair.opener();
            (open.to_string(), Self::pair_of(open).unwrap().to_string())
        });
        targets
            .into_iter()
            .flat_map(|(open, close)| {
                let (left, right) = new.clone().unwrap_or_default();
                [(open..open + 1, left), (close..close + 1, right)]
            })
            .collect()
    }

    /// Prepare word-only replacements; an overlapping pair rejects the entire command.
    /// Read-only preparation lets the UI retain IME and history on rejection or no change.
    pub fn transpose_word_edits(&self) -> Vec<(Range<usize>, String)> {
        if self.doc.edits_blocked() {
            return Vec::new();
        }
        let mut words = Vec::new();
        let mut end = 0;
        for (at, c) in self.doc.rope.chars().enumerate() {
            if at >= end && Self::is_word_char(c) {
                // A preceding separator's combining tail stays with that separator.
                let start = self.snap_to_cluster(at);
                // Prepend scalars join a following base; they are not separator prefixes.
                if start < at
                    && self.doc.rope.slice(start..at).chars().any(|prefix| {
                        !crate::selection::word_continuation(prefix)
                            && format!("{prefix}a").graphemes(true).count() != 1
                    })
                {
                    end = self.next_cluster(start);
                    continue;
                }
                let word = start..self.word_range_at(at).end;
                end = word.end;
                words.push(word);
            }
        }
        let mut pairs = Vec::new();
        for sel in &self.view.selections {
            if sel.head_virtual > 0 || words.len() < 2 {
                continue;
            }
            let head = self.snap_to_cluster(sel.head);
            let right = if head == self.len() {
                words.len() - 1
            } else {
                let at = words.partition_point(|word| word.end <= head);
                if words.get(at).is_some_and(|word| word.contains(&head)) {
                    at + 1
                } else {
                    at
                }
            };
            if right > 0 && right < words.len() {
                pairs.push((words[right - 1].clone(), words[right].clone()));
            }
        }
        pairs.sort_by_key(|(left, _)| left.start);
        if pairs.windows(2).any(|pair| pair[1].0.start < pair[0].1.end) {
            return Vec::new();
        }
        pairs
            .into_iter()
            .flat_map(|(left, right)| {
                let a = self.doc.rope.slice(left.clone()).to_string();
                let b = self.doc.rope.slice(right.clone()).to_string();
                [(left, b), (right, a)]
            })
            .filter(|(range, text)| self.doc.rope.slice(range.clone()) != text.as_str())
            .collect()
    }

    /// The digest of each non-empty selection in document order, or of the whole document
    /// as saved when none is selected.
    pub fn hash_selections(&self, h: Hash) -> std::io::Result<Vec<String>> {
        let mut ranges: Vec<_> = (self.view.selections.iter()).filter(|s| !s.is_empty()).map(|s| s.range()).collect();
        ranges.sort_by_key(|r| r.start);
        if ranges.is_empty() {
            return Ok(vec![digest(h, &self.bytes_as_saved(None)?)]);
        }
        (ranges.into_iter()).map(|r| Ok(digest(h, &self.bytes_as_saved(Some(r))?))).collect()
    }

    /// Each selection's text, in order.
    pub(crate) fn selected_texts(&self) -> Vec<String> {
        (self.view.selections.iter()).map(|s| self.doc.rope.slice(s.range()).to_string()).collect()
    }

    /// The innermost `()[]{}` pair around `from..to`: offsets of the opener and the closer.
    /// Brackets inside strings and comments count too.
    pub(crate) fn enclosing_brackets(&self, from: usize, to: usize) -> Option<(usize, usize)> {
        let mut depth = [0i32; 3];
        let back = self.doc.rope.chars_at(from).reversed();
        let (open, kind) = (1..=from).zip(back).find_map(|(n, c)| {
            match bracket_kind(c) {
                Some((k, false)) => depth[k] += 1,
                Some((k, true)) if depth[k] == 0 => return Some((from - n, k)),
                Some((k, true)) => depth[k] -= 1,
                None => {}
            }
            None
        })?;
        let mut d = 0;
        let to = to.min(self.len());
        for (j, c) in (to..).zip(self.doc.rope.chars_at(to)) {
            match bracket_kind(c) {
                Some((k, true)) if k == kind => d += 1,
                Some((k, false)) if k == kind && d == 0 => return Some((open, j)),
                Some((k, false)) if k == kind => d -= 1,
                _ => {}
            }
        }
        None
    }
}

impl<D: DocRef, V: ViewMut> Editing<D, V> {
    /// What a transformation works on: each non-empty selection (widened to whole lines, with
    /// their line breaks, for `lines`), or the whole document when nothing is selected. A target
    /// flagged `false` is an empty selection among others, left alone.
    pub(crate) fn transform_targets(&mut self, lines: bool) -> Vec<(Selection, bool)> {
        self.normalize();
        if self.view.selections.iter().all(|s| s.is_empty()) {
            return vec![(Selection::new(0, self.len()), true)];
        }
        if !lines {
            return (self.view.selections.iter()).map(|s| (*s, !s.range().is_empty())).collect();
        }
        let blocks = self.blocks_covering(self.view.selections.iter().filter(|s| !s.is_empty()));
        (blocks.into_iter())
            .map(|(l0, l1)| {
                // the block's own line break goes with it: a block the transform empties
                // takes its line away rather than leaving a blank one
                (Selection::new(self.line_start(l0), self.offset(l1 + 1, 0)), true)
            })
            .collect()
    }

    /// The text `transform_selections(lines, ..)` would work on, one string per target.
    pub fn transform_texts(&mut self, lines: bool) -> Vec<String> {
        (self.transform_targets(lines).into_iter())
            .filter(|(_, run)| *run)
            .map(|(sel, _)| self.doc.rope.slice(sel.range()).to_string())
            .collect()
    }

    // ---- Edit > Mark ----

    pub fn set_mark(&mut self) {
        self.view.mark = Some(self.cursor());
    }

    pub fn clear_mark(&mut self) {
        self.view.mark = None;
    }

    pub fn select_to_mark(&mut self) {
        if let Some(m) = self.view.mark {
            self.set_selection(m.min(self.len()), self.cursor());
        }
    }

    pub fn swap_with_mark(&mut self) {
        if let Some(m) = self.view.mark {
            self.view.mark = Some(self.cursor());
            self.set_cursor(m);
        }
    }

    // ---- Selection > Expand ----

    /// Grow to whole lines with the head at the top; repeating adds the line above.
    pub fn select_line_upward(&mut self) {
        self.set_typing(false);
        for i in 0..self.view.selections.len() {
            let sel = self.view.selections[i];
            let (l0, c0) = self.point(sel.start());
            let (l1, c1) = self.point(sel.end());
            let whole = !sel.is_empty() && c0 == 0 && c1 == 0;
            let top = if whole { l0.saturating_sub(1) } else { l0 };
            let end = if whole || c1 == 0 && l1 > l0 { sel.end() } else { self.offset(l1 + 1, 0) };
            self.view.selections[i] = Selection::new(end, self.line_start(top));
        }
        self.normalize();
    }

    /// Expand Selection to Block and to Paragraph, as Sublime's Expand Selection to Paragraph:
    /// a caret takes its paragraph (with the last
    /// line's break), or on a blank line that line alone; pressed again it takes in the next
    /// blank line or paragraph.
    pub fn expand_to_paragraph(&mut self) {
        self.set_typing(false);
        let blank = |s: &Self, l: usize| s.line(l).trim().is_empty();
        let next_start = |s: &Self, l: usize| s.offset(l + 1, 0).max(s.line_end(l));
        let unit = |s: &Self, l: usize| {
            if blank(s, l) {
                return (l, l);
            }
            let (mut a, mut b) = (l, l);
            while a > 0 && !blank(s, a - 1) {
                a -= 1;
            }
            while b + 1 < s.line_count() && !blank(s, b + 1) {
                b += 1;
            }
            (a, b)
        };
        for i in 0..self.view.selections.len() {
            let sel = self.view.selections[i];
            let (l0, l1) = self.covered_lines(&sel);
            let (a, _) = unit(self, l0);
            let (_, mut b) = unit(self, l1);
            let mut range = Selection::new(self.line_start(a), next_start(self, b));
            // already a whole unit: the next one joins it
            if !sel.is_empty() && range.range() == sel.range() && b + 1 < self.line_count() {
                b = unit(self, b + 1).1;
                range = Selection::new(range.anchor, next_start(self, b));
            }
            self.view.selections[i] = range;
        }
        self.normalize();
    }

    pub fn expand_to_indentation(&mut self) {
        self.set_typing(false);
        let width = |s: &Self, l: usize| -> Option<usize> {
            let t = s.line(l);
            (!t.trim().is_empty()).then(|| t.chars().take_while(|c| c.is_whitespace()).count())
        };
        for i in 0..self.view.selections.len() {
            let sel = self.view.selections[i];
            let (l0, _) = self.point(sel.start());
            let (l1, _) = self.point(sel.end());
            let Some(base) = width(self, l0) else {
                continue;
            };
            let fits = |s: &Self, l: usize| width(s, l).is_none_or(|w| w >= base);
            let (mut a, mut b) = (l0, l1);
            while a > 0 && fits(self, a - 1) {
                a -= 1;
            }
            while b + 1 < self.line_count() && fits(self, b + 1) {
                b += 1;
            }
            while a < l0 && width(self, a).is_none() {
                a += 1;
            }
            while b > l1 && width(self, b).is_none() {
                b -= 1;
            }
            self.view.selections[i] = Selection::new(self.line_start(a), self.offset(b + 1, 0));
        }
        self.normalize();
    }

    /// Expand Selection to Brackets: the contents first, then each enclosing pair's.
    pub fn expand_to_brackets(&mut self) {
        self.set_typing(false);
        for i in 0..self.view.selections.len() {
            let sel = self.view.selections[i];
            let (a, b) = (sel.start(), sel.end());
            if let Some((o, c)) = self.enclosing_brackets(a, b) {
                self.view.selections[i] = match (o + 1, c) == (a, b) {
                    true => Selection::new(o, c + 1),
                    false => Selection::new(o + 1, c),
                };
            }
        }
        self.normalize();
    }

    /// Sublime's "smart" expansion: the word first, then the enclosing brackets.
    pub fn expand_smart(&mut self) {
        if self.view.selections.iter().all(|s| s.is_empty()) {
            self.select_next_occurrence();
        } else {
            self.expand_to_brackets();
        }
    }

    /// ctrl-m: jump between the two ends of the enclosing bracket pair.
    pub fn jump_to_bracket(&mut self) {
        self.set_typing(false);
        for i in 0..self.view.selections.len() {
            let caret = self.view.selections[i].head;
            let prev = (caret > 0).then(|| self.doc.rope.char(caret - 1));
            let next = (caret < self.len()).then(|| self.doc.rope.char(caret));
            let pos = if prev.and_then(bracket_kind).is_some_and(|(_, open)| !open) {
                caret - 1
            } else if next.and_then(bracket_kind).is_some_and(|(_, open)| open) {
                caret + 1
            } else {
                caret
            };
            if let Some((o, c)) = self.enclosing_brackets(pos, pos) {
                let to = if caret <= o + 1 { c } else { o + 1 };
                self.view.selections[i] = Selection::caret(to);
            }
        }
        self.normalize();
    }

    /// Quick Find (⌘⌥G), as Sublime: from a caret it selects the word under it, and
    /// while that word (or the next found that way) is selected it finds whole words; from a
    /// selection, any occurrence.
    pub fn find_under(&mut self) {
        self.set_typing(false);
        let last = *self.view.selections.last().unwrap();
        if last.is_empty() {
            let r = self.word_range_at(last.head);
            if !r.is_empty() {
                self.set_selection(r.start, r.end);
                self.view.quick_find_word = Some(Selection::new(r.start, r.end));
            }
            return;
        }
        let whole_word = self.view.quick_find_word == Some(last);
        let needle = self.doc.rope.slice(last.range()).to_string();
        let opts = FindOpts { case_sensitive: true, whole_word, ..FindOpts::default() };
        if let Some(next) = self.find_next(&needle, last.end(), &opts) {
            self.set_selection(next.start, next.end);
            self.view.quick_find_word = whole_word.then(|| Selection::new(next.start, next.end));
        }
    }
}

impl<D: DocMut, V: ViewMut> Editing<D, V> {
    // ---- Sublime's Edit > Text / Line / Convert Case / permute commands ----

    /// Rewrite each selection (or the word under a caret, as Sublime does) with `f`.
    pub(crate) fn map_selected(&mut self, f: impl Fn(&str) -> String) {
        self.set_typing(false);
        self.record();
        let mut converted: Option<(Range<usize>, String)> = None;
        self.edit_with(|s, _, sel| {
            let r = if sel.is_empty() {
                match &converted {
                    Some((r, _)) if r.contains(&sel.head) => r.clone(),
                    _ => s.word_range_at(sel.head),
                }
            } else {
                sel.range()
            };
            let new = match &converted {
                Some((prev, new)) if *prev == r => new.clone(),
                _ => {
                    let new = f(&s.doc.rope.slice(r.clone()).to_string());
                    converted = Some((r.clone(), new.clone()));
                    new
                }
            };
            let end = r.start + new.chars().count();
            let keep = if sel.is_empty() { Selection::caret(sel.head.min(end)) } else { Selection::new(r.start, end) };
            (r, new, keep)
        });
    }

    pub fn convert_case(&mut self, case: Case) {
        self.map_selected(|t| convert_case(t, case));
    }

    /// Transform each non-empty selection with `f` (the whole document when none is): one undo
    /// step. `lines` widens a selection to whole lines, as Sort Lines does. Any `Err` changes
    /// nothing and comes back naming the target's first line.
    pub fn transform_selections(
        &mut self,
        lines: bool,
        f: impl Fn(&str) -> Result<String, String>,
    ) -> Result<(), String> {
        if self.doc.edits_blocked() {
            return Err(if self.doc.viewer_mode() {
                "Turn off viewer mode before editing"
            } else {
                "Stop following before editing"
            }
            .into());
        }
        let targets = self.transform_targets(lines);
        // all converted before any edit: one failure leaves the buffer as it was
        let mut converted = Vec::with_capacity(targets.len());
        let mut changed = false;
        for (sel, run) in &targets {
            converted.push(match run {
                false => None,
                true => {
                    let old = self.doc.rope.slice(sel.range()).to_string();
                    let new = f(&old).map_err(|e| format!("Line {}: {e}", self.point(sel.start()).0 + 1))?;
                    // line breaks are LF in the buffer, as a paste makes them
                    let new = new.replace("\r\n", "\n");
                    changed |= new != old;
                    Some(new)
                }
            });
        }
        // nothing to change: no step, and the selections stay (not the whole document selected)
        if !changed {
            return Ok(());
        }
        self.set_typing(false);
        self.record();
        self.view.selections = targets.iter().map(|(sel, _)| *sel).collect();
        self.edit_with(|s, i, sel| match &converted[i] {
            Some(new) => {
                let mut r = sel.range();
                // an emptied last line has no break of its own: the one before it goes instead
                let last = !r.is_empty() && r.end == s.len() && s.doc.rope.char(r.end - 1) != '\n';
                if lines && last && new.is_empty() && r.start > 0 {
                    r.start = s.off_cr_lf(r.start - 1);
                }
                let end = r.start + new.chars().count();
                (r.clone(), new.clone(), Selection::new(r.start, end))
            }
            None => (sel.head..sel.head, String::new(), *sel),
        });
        Ok(())
    }

    /// Replace the whole document with `f` of its text as one undo step, the caret staying
    /// where it was; true when the text changed (an `f` that changes nothing leaves no step).
    pub fn transform_document(&mut self, f: impl Fn(&str) -> Result<String, String>) -> Result<bool, String> {
        if self.doc.edits_blocked() {
            return Err(if self.doc.viewer_mode() {
                "Turn off viewer mode before editing"
            } else {
                "Stop following before editing"
            }
            .into());
        }
        let old = self.text();
        // line breaks are LF in the buffer, as a paste makes them
        let mut new = f(&old)?.replace("\r\n", "\n");
        if new == old {
            return Ok(false);
        }
        let caret = self.cursor().min(new.chars().count());
        self.set_typing(false);
        self.record();
        self.view.selections = vec![Selection::new(0, self.len())];
        // the one selection is asked once
        let caret = Selection::caret(caret);
        self.edit_with(|_, _, sel| (sel.range(), std::mem::take(&mut new), caret));
        Ok(true)
    }

    /// Swap adjacent words at selection heads, preserving separators and ordinary range mapping.
    pub fn transpose_words(&mut self) -> bool {
        let edits = self.transpose_word_edits();
        self.replace_ranges(edits)
    }

    /// ctrl-t: swap the characters around each caret; with selections, rotate their texts.
    pub fn transpose(&mut self) {
        self.set_typing(false);
        self.record();
        if self.view.selections.iter().all(|s| s.is_empty()) {
            // left to right, each caret moves one on; one landing on the next swallows it, which
            // then does not swap (Sublime 4215)
            let mut moved_to = None;
            self.edit_with(|s, _, sel| {
                let a = s.doc.snap_to_cluster(sel.head);
                if a == 0 || a >= s.len() || moved_to == Some(a) {
                    moved_to = None;
                    return (a..a, String::new(), *sel);
                }
                let (start, end) = (s.prev_cluster(a), s.next_cluster(a));
                moved_to = Some(end);
                let p = s.doc.rope.slice(start..a);
                let n = s.doc.rope.slice(a..end);
                (start..end, format!("{n}{p}"), Selection::caret(end))
            });
            return;
        }
        let texts = self.selected_texts();
        let n = texts.len();
        self.replace_selected(|i| texts[(i + n - 1) % n].clone());
    }

    /// Each selection's text replaced by `text(i)`, which is then selected.
    fn replace_selected(&mut self, text: impl Fn(usize) -> String) {
        self.edit_with(|_, i, sel| {
            let t = text(i);
            let end = sel.start() + t.chars().count();
            (sel.range(), t, Selection::new(sel.start(), end))
        });
    }

    /// cmd-shift-j: join each selected line with the next, its indent collapsed to one space.
    pub fn join_lines(&mut self) {
        if self.doc.edits_blocked() {
            return;
        }
        self.set_typing(false);
        self.record();
        for (l0, l1) in self.line_blocks().into_iter().rev() {
            let last = if l0 == l1 { l1 + 1 } else { l1 };
            for l in (l0 + 1..=last).rev() {
                if l >= self.line_count() {
                    continue;
                }
                let brk = self.line_end(l - 1);
                let ws = self.line(l).chars().take_while(|c| c.is_whitespace()).count();
                let removed = self.line_start(l) + ws - brk;
                self.splice(brk..brk + removed, " ");
                for s in &mut self.view.selections {
                    for p in [&mut s.anchor, &mut s.head] {
                        if *p > brk {
                            *p = (*p + 1).saturating_sub(removed).max(brk + 1);
                        }
                    }
                }
                self.touch(l - 1);
            }
        }
        self.normalize();
    }

    /// Sort, reverse, unique or shuffle the selected lines; the whole buffer when nothing is selected.
    pub fn permute_lines(&mut self, op: Permute) {
        if self.doc.edits_blocked() {
            return;
        }
        self.set_typing(false);
        self.record();
        let blocks = if self.view.selections.iter().all(|s| s.is_empty()) {
            vec![self.covered_lines(&Selection::new(0, self.len()))]
        } else {
            self.line_blocks()
        };
        let n = self.view.changes.len();
        for (l0, l1) in blocks.into_iter().rev() {
            let (s, e) = (self.line_start(l0), self.line_end_raw(l1));
            let lines: Vec<String> = (l0..=l1).map(|l| self.line_raw(l)).collect();
            let new = permute(lines, op).join("\n");
            let before = self.view.changes.len();
            self.splice(s..e, &new);
            if self.view.changes.len() > before {
                self.touch(l0);
            }
        }
        let changes = self.view.changes[n..].to_vec();
        self.map_selections(&changes, false);
        self.normalize();
    }

    /// Permute the selections' texts among themselves; Unique deletes repeated texts.
    pub fn permute_selections(&mut self, op: Permute) {
        self.set_typing(false);
        self.record();
        let texts = self.selected_texts();
        let new = if op == Permute::Unique {
            let mut seen = HashSet::new();
            (texts.into_iter())
                .map(|t| match seen.insert(t.clone()) {
                    true => t,
                    false => String::new(),
                })
                .collect()
        } else {
            permute(texts, op)
        };
        self.replace_selected(|i| new[i].clone());
    }

    /// Delete to Hard EOL / BOL; at the edge already, the line break goes. Like Sublime's
    /// macro, every selection first extends to the edge, so overlapping ones merge.
    pub fn delete_to_line_edge(&mut self, forward: bool) {
        self.set_typing(false);
        self.record();
        let ranges: Vec<Selection> = (self.view.selections.iter())
            .map(|sel| {
                let r = if forward {
                    sel.start()..self.line_end(self.point(sel.end()).0)
                } else {
                    self.line_start(self.point(sel.start()).0)..sel.end()
                };
                // at the edge the line break goes, a stray CRLF whole
                match (r.is_empty(), forward) {
                    (false, _) => Selection::new(r.start, r.end),
                    (true, true) => Selection::new(r.start, self.next_cluster(r.start)),
                    (true, false) => Selection::new(self.prev_cluster(r.start), r.start),
                }
            })
            .collect();
        self.view.selections = ranges;
        self.normalize();
        self.view.kill = self.selected_texts().concat();
        self.edit(|_, sel| (sel.range(), String::new()));
    }

    pub fn delete_to_mark(&mut self) {
        if self.view.mark.is_some() {
            self.select_to_mark();
            let sel = self.view.selections[0];
            self.view.kill = self.doc.rope.slice(sel.range()).to_string();
            self.record();
            self.edit(|_, sel| (sel.range(), String::new()));
            self.view.mark = Some(self.cursor());
        }
    }

    /// ctrl-y: insert what the last kill removed.
    pub fn yank(&mut self) {
        if !self.view.kill.is_empty() {
            let t = self.view.kill.clone();
            self.set_typing(false);
            self.insert(&t);
            self.set_typing(false);
        }
    }

    // ---- Edit > Wrap, Paste and Indent ----

    /// Sublime's Wrap Paragraph: each paragraph a selection touches, or the one at a caret, is
    /// refilled on its own to at most `width` columns. A paragraph is a run of non-blank lines
    /// that are all code or all `comment` lines; every line keeps the first line's indent and
    /// comment marker. A caret on a blank line wraps nothing. Plain text's `- `, `* `, `> ` list
    /// markers get no hanging indent (Sublime's do); add one if lists are wrapped often.
    pub fn wrap_lines(&mut self, width: usize, comment: Option<&str>) {
        if self.doc.edits_blocked() {
            return;
        }
        self.set_typing(false);
        self.record();
        // a line's indent and comment marker (with the spaces after it), and whether it is
        // blank once those are gone
        let split = |t: &str| -> (usize, bool) {
            let ws = t.len() - t.trim_start().len();
            let mut p = ws;
            if let Some(c) = comment.filter(|c| t[ws..].starts_with(c)) {
                p += c.len();
                p += t[p..].len() - t[p..].trim_start_matches(' ').len();
            }
            (p, t[p..].trim().is_empty())
        };
        let kind = |s: &Self, l: usize| -> Option<bool> {
            let t = s.line(l);
            let (p, blank) = split(&t);
            let marked = comment.is_some_and(|c| t[..p].trim_start().starts_with(c));
            (!blank).then_some(marked)
        };
        let mut paragraphs: Vec<(usize, usize)> = Vec::new();
        for sel in &self.view.selections {
            let (l0, l1) = self.covered_lines(sel);
            for l in l0..=l1 {
                let Some(k) = kind(self, l) else { continue };
                if paragraphs.last().is_some_and(|&(_, b)| l <= b) {
                    continue;
                }
                let (mut a, mut b) = (l, l);
                while a > 0 && kind(self, a - 1) == Some(k) {
                    a -= 1;
                }
                while b + 1 < self.line_count() && kind(self, b + 1) == Some(k) {
                    b += 1;
                }
                paragraphs.push((a, b));
            }
        }
        paragraphs.sort_unstable();
        paragraphs.dedup();
        let n = self.view.changes.len();
        for &(l0, l1) in paragraphs.iter().rev() {
            let first = self.line(l0);
            let prefix = first[..split(&first).0].to_string();
            let prefix_cols = super::line_editing::columns(&prefix, self.doc.tab_size);
            let words: Vec<String> = (l0..=l1)
                .flat_map(|l| {
                    let t = self.line(l);
                    let words = t[split(&t).0..].split_whitespace().map(String::from);
                    words.collect::<Vec<_>>()
                })
                .collect();
            let mut out: Vec<String> = Vec::new();
            let (mut cur, mut cols) = (prefix.clone(), prefix_cols);
            for w in words {
                let wn = w.chars().count();
                if cols > prefix_cols && cols + 1 + wn > width {
                    out.push(std::mem::replace(&mut cur, prefix.clone()));
                    cols = prefix_cols;
                }
                if cols > prefix_cols {
                    cur.push(' ');
                    cols += 1;
                }
                cur.push_str(&w);
                cols += wn;
            }
            out.push(cur);
            let (s, e) = (self.line_start(l0), self.line_end(l1));
            self.splice(s..e, &out.join("\n"));
        }
        if self.view.changes.len() > n {
            let changes = self.view.changes[n..].to_vec();
            self.map_selections(&changes, false);
            let first = paragraphs.first().map_or(0, |p| p.0);
            self.touch(first);
        }
        self.normalize();
    }

    pub fn paste_and_indent(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n");
        let base: String = text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        self.set_typing(false);
        self.record();
        self.edit(|s, sel| {
            let indent = s.indent_of(s.point(sel.start()).0);
            let lines: Vec<String> = (text.split('\n').enumerate())
                .map(|(i, l)| {
                    let l = l.strip_prefix(base.as_str()).unwrap_or(l);
                    match i == 0 || l.is_empty() {
                        true => l.to_string(),
                        false => format!("{indent}{l}"),
                    }
                })
                .collect();
            (sel.range(), lines.join("\n"))
        });
    }
}

impl Document {
    /// The bytes a save would write for `range` (the whole document when `None`): the line
    /// ending applied, and the byte order mark only for the whole document (`None`, or a range
    /// covering all of it).
    pub fn bytes_as_saved(&self, range: Option<Range<usize>>) -> std::io::Result<Vec<u8>> {
        let whole = range.as_ref().is_none_or(|r| *r == (0..self.len()));
        let range = range.unwrap_or(0..self.len());
        let text = match &self.endings {
            Some(map) => map.text(&self.rope, range)?,
            None => file_text(self.rope.slice(range).to_string(), self.line_ending, false),
        };
        orlo_project::encoding::encode_owned(text, self.encoding, whole && self.bom)
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    /// Validate all original whole physical selections before any IME or history mutation.
    pub fn adjust_value_edits(
        &self,
        step: crate::editing::columns::ValueStep,
    ) -> Result<Vec<(Range<usize>, String)>, String> {
        if self.doc.edits_blocked() {
            return Err(if self.doc.viewer_mode() {
                "Turn off viewer mode before editing"
            } else {
                "Stop following before editing"
            }
            .into());
        }
        self.view
            .selections
            .iter()
            .filter(|s| !s.range().is_empty())
            .filter_map(|s| {
                let range = s.range();
                let old = self.doc.rope.slice(range.clone()).to_string();
                match crate::editing::columns::adjust_value(&old, step) {
                    Ok(new) if new == old => None,
                    Ok(new) => Some(Ok((range, new))),
                    Err(error) => Some(Err(format!("Line {}: {error}", self.doc.rope.char_to_line(range.start) + 1))),
                }
            })
            .collect()
    }
}

impl<D: DocRef, V: ViewRef> Editing<D, V> {
    /// New value steps precompute through the existing pipeline/scopes before native input ends.
    pub fn value_pipeline_outputs(
        &self,
        pipeline: &crate::search::pipeline::Pipeline,
    ) -> Result<Option<Vec<String>>, String> {
        if self.doc.edits_blocked() {
            return Err(if self.doc.viewer_mode() {
                "Turn off viewer mode before editing"
            } else {
                "Stop following before editing"
            }
            .into());
        }
        if self.view.selections.iter().all(|s| s.range().is_empty())
            && self.view.selections.iter().any(Selection::has_virtual)
        {
            return Ok(None);
        }
        // Normalize only a detached view, using exactly the existing pipeline target policy.
        let mut view = self.view.duplicate(&self.doc);
        let mut state = Editing { doc: &*self.doc, view: &mut view };
        let mut changed = false;
        let outputs = state
            .transform_texts(pipeline.lines())
            .iter()
            .map(|old| {
                let new = pipeline.run(old).map_err(|e| pipeline.failure(e))?;
                changed |= new != *old;
                Ok(new)
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(changed.then_some(outputs))
    }
}
