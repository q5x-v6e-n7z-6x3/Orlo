//! Immutable source snapshots for native printing. No document or view is retained.

use ropey::Rope;
use std::ops::Range;

#[derive(Debug)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub color: gpui::Hsla,
}
#[derive(Debug)]
pub struct Prepared {
    pub text: String,
    pub styles: Vec<StyleSpan>,
}

#[derive(Clone)]
pub struct Snapshot {
    rope: Rope,
    selections: Vec<Range<usize>>,
    pub title: String,
    syntax: String,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub font_size: u8,
    pub tab_width: u8,
    pub line_numbers: bool,
    pub wrap: bool,
    pub syntax_colors: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { font_size: 11, tab_width: 4, line_numbers: false, wrap: true, syntax_colors: false }
    }
}

impl Options {
    pub fn validate(self) -> Result<(), String> {
        if !(8..=24).contains(&self.font_size) {
            return Err("Font size must be a whole number from 8 to 24".into());
        }
        if !(1..=16).contains(&self.tab_width) {
            return Err("Tab width must be a whole number from 1 to 16".into());
        }
        Ok(())
    }
}

impl Snapshot {
    pub fn new(rope: Rope, selections: Vec<Range<usize>>, title: String) -> Self {
        Self { rope, selections, title, syntax: "Plain Text".into() }
    }

    pub fn set_syntax(&mut self, syntax: String) {
        self.syntax = syntax;
    }

    pub fn source(&self, selection: bool, options: Options) -> Result<String, String> {
        Ok(self.prepare(selection, Options { syntax_colors: false, ..options })?.text)
    }

    pub fn prepare(&self, selection: bool, options: Options) -> Result<Prepared, String> {
        options.validate()?;
        if self.rope.len_bytes() > 16 * 1024 * 1024 {
            return Err("The document exceeds the 16 MiB print preparation limit".into());
        }
        let mut ranges =
            if selection { self.selections.clone() } else { std::iter::once(0..self.rope.len_chars()).collect() };
        if ranges.iter().any(|r| r.start > r.end || r.end > self.rope.len_chars()) {
            return Err("The captured selection is outside the source".into());
        }
        if selection {
            ranges.retain(|r| !r.is_empty());
            if ranges.is_empty() {
                return Err("Select text before opening Print, or choose Whole document".into());
            }
        }
        ranges.sort_by_key(|r| r.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in ranges {
            if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        // Bound expensive highlighting; large documents retain their exact plain output.
        let mut classes = Vec::new();
        if options.syntax_colors && self.rope.len_bytes() <= 2_000_000 && self.rope.len_lines() <= 100_000 {
            use std::sync::{atomic::AtomicBool, Arc};
            let highlighter = orlo_syntax::highlight::Highlighter::by_name(&self.syntax);
            if highlighter.syntax_name() != "Plain Text" {
                let mut runs = orlo_syntax::highlight::LineRuns::new(
                    highlighter,
                    self.rope.clone(),
                    Arc::new(AtomicBool::new(false)),
                );
                for line in 0..self.rope.len_lines() {
                    let mut byte = self.rope.line_to_byte(line);
                    for (len, color, _) in runs.line(line) {
                        let end = byte + len;
                        classes.push((self.rope.byte_to_char(byte)..self.rope.byte_to_char(end), color));
                        byte = end;
                    }
                }
            }
        }
        let mut prepared = Prepared { text: String::new(), styles: Vec::new() };
        let mut utf16 = 0;
        for (excerpt, range) in merged.into_iter().enumerate() {
            if excerpt > 0 {
                prepared.text.push('\n');
                check_output_size(&prepared.text)?;
                utf16 += 1;
            }
            let line = self.rope.char_to_line(range.start);
            let prefix = self.rope.slice(self.rope.line_to_char(line)..range.start).to_string();
            let (_, columns) = orlo_editor::viewport::expand_tabs(&prefix, options.tab_width as usize);
            let mut chars = Vec::new();
            let mut iter = self.rope.slice(range.clone()).chars().enumerate().peekable();
            while let Some((offset, ch)) = iter.next() {
                if ch == '\r' {
                    if iter.peek().is_some_and(|(_, next)| *next == '\n') {
                        iter.next();
                    }
                    chars.push(('\n', range.start + offset));
                } else {
                    chars.push((ch, range.start + offset));
                }
            }
            let mut start = 0;
            let mut index = 0;
            loop {
                let end = chars[start..].iter().position(|(ch, _)| *ch == '\n').map_or(chars.len(), |n| start + n);
                if options.line_numbers && !(start == chars.len() && start > 0 && chars[start - 1].0 == '\n') {
                    use std::fmt::Write;
                    let prefix = format!("{}  ", line + index + 1);
                    utf16 += prefix.len();
                    write!(prepared.text, "{prefix}").unwrap();
                    check_output_size(&prepared.text)?;
                }
                let mut column = if index == 0 { *columns.last().unwrap_or(&0) } else { 0 };
                for &(ch, offset) in &chars[start..end] {
                    let text = ch.to_string();
                    let (piece, cols) =
                        orlo_editor::viewport::expand_tabs_at(&text, options.tab_width as usize, column);
                    column += cols.last().copied().unwrap_or(0);
                    let len = piece.encode_utf16().count();
                    let i = classes.partition_point(|(range, _)| range.end <= offset);
                    if let Some((range, color)) = classes.get(i).filter(|(range, _)| range.contains(&offset)) {
                        let _ = range;
                        if let Some(last) =
                            prepared.styles.last_mut().filter(|last| last.range.end == utf16 && last.color == *color)
                        {
                            last.range.end += len;
                        } else {
                            prepared.styles.push(StyleSpan { range: utf16..utf16 + len, color: *color });
                        }
                    }
                    prepared.text.push_str(&piece);
                    check_output_size(&prepared.text)?;
                    utf16 += len;
                }
                if end == chars.len() {
                    break;
                }
                prepared.text.push('\n');
                check_output_size(&prepared.text)?;
                utf16 += 1;
                start = end + 1;
                index += 1;
            }
        }
        Ok(prepared)
    }
}

fn check_output_size(text: &str) -> Result<(), String> {
    if text.len() > 64 * 1024 * 1024 {
        Err("The expanded print source exceeds the 64 MiB limit".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_selection_maps_syntax_across_tabs_and_utf16() {
        let text = "let 😀 = \"value\";\nlet y = 2;";
        let mut snapshot = Snapshot::new(Rope::from_str(text), vec![4..16, 17..27], "x".into());
        snapshot.set_syntax("Rust".into());
        let prepared =
            snapshot.prepare(true, Options { syntax_colors: true, line_numbers: true, ..Options::default() }).unwrap();
        assert_eq!(prepared.text, "1  😀 = \"value\";\n2  let y = 2;");
        assert!(prepared
            .styles
            .iter()
            .any(|run| run.range.start >= 3 && run.range.end <= prepared.text.encode_utf16().count()));
        assert!(prepared.styles.iter().all(|run| run.range.start >= 3));
    }

    #[test]
    fn excerpt_colors_follow_original_context_through_crlf_tabs_and_surrogates() {
        let text = "let s = \"😀\ttext\";\r\n/* begin\r\ninside */\r\nlast";
        let at = |word: &str| text[..text.find(word).unwrap()].chars().count();
        let start = at("😀");
        let comment = at("inside");
        let mut snapshot =
            Snapshot::new(Rope::from_str(text), vec![start..at("\";"), comment..comment + 6], "x".into());
        snapshot.set_syntax("Rust".into());
        let options = Options { syntax_colors: true, line_numbers: true, ..Options::default() };
        let all = snapshot.prepare(false, options).unwrap();
        let selected = snapshot.prepare(true, options).unwrap();
        assert!(selected.text.starts_with("1  😀"));
        assert!(selected.text.ends_with("3  inside"));
        let color_at = |p: &Prepared, word: &str| {
            let offset = p.text[..p.text.find(word).unwrap()].encode_utf16().count();
            p.styles.iter().find(|s| s.range.contains(&offset)).map(|s| s.color)
        };
        for word in ["😀", "text", "inside"] {
            assert!(color_at(&selected, word).is_some());
            assert_eq!(color_at(&selected, word), color_at(&all, word));
        }
        let second_prefix = selected.text[..selected.text.find("3  inside").unwrap()].encode_utf16().count();
        assert!(selected.styles.iter().all(|s| !s.range.contains(&0) && !s.range.contains(&second_prefix)));
        let plain = snapshot.prepare(true, Options { syntax_colors: false, ..options }).unwrap();
        assert!(plain.styles.is_empty());
        snapshot.set_syntax("missing-language".into());
        assert!(snapshot.prepare(true, options).unwrap().styles.is_empty());
    }

    #[test]
    fn large_sources_fall_back_to_plain_or_refuse_bounded_preparation() {
        let snapshot = Snapshot::new(Rope::from_str(&"\n".repeat(100_001)), vec![], "x".into());
        let prepared = snapshot.prepare(false, Options { syntax_colors: true, ..Options::default() }).unwrap();
        assert_eq!(prepared.text.len(), 100_001);
        assert!(prepared.styles.is_empty());
        let large = Snapshot::new(Rope::from_str(&"x".repeat(16 * 1024 * 1024 + 1)), vec![], "x".into());
        assert!(large.prepare(false, Options::default()).unwrap_err().contains("16 MiB"));
    }

    #[test]
    fn wrapping_and_color_defaults_preserve_existing_print() {
        let options = Options::default();
        assert!(options.wrap);
        assert!(!options.syntax_colors);
    }

    #[test]
    fn source_snapshot_keeps_unicode_tabs_and_all_folded_source_after_edits() {
        let mut rope = Rope::from_str("α\t中😀\nhidden\nlast");
        let snapshot = Snapshot::new(rope.clone(), vec![2..4, 5..11], "source".into());
        rope.remove(..);
        assert_eq!(snapshot.source(false, Options::default()).unwrap(), "α   中😀\nhidden\nlast");
        assert_eq!(snapshot.source(true, Options::default()).unwrap(), "中😀\nhidden");
    }

    #[test]
    fn selections_merge_overlap_and_preserve_source_line_numbers() {
        let snapshot =
            Snapshot::new(Rope::from_str("one\ntwo\nthree\nfour"), vec![8..13, 4..7, 5..7, 0..0], "x".into());
        let options = Options { line_numbers: true, ..Options::default() };
        assert_eq!(snapshot.source(true, options).unwrap(), "2  two\n3  three");
        assert_eq!(snapshot.source(false, options).unwrap(), "1  one\n2  two\n3  three\n4  four");
    }

    #[test]
    fn empty_selection_and_invalid_options_are_errors_but_empty_document_prints() {
        let snapshot = Snapshot::new(Rope::new(), std::iter::once(0..0).collect(), "x".into());
        assert!(snapshot.source(true, Options::default()).is_err());
        assert_eq!(snapshot.source(false, Options::default()).unwrap(), "");
        for (font_size, tab_width) in [(0, 4), (25, 4), (11, 0), (11, 17)] {
            assert!(snapshot.source(false, Options { font_size, tab_width, ..Options::default() }).is_err());
        }
        let invalid = Snapshot::new(Rope::from_str("é"), std::iter::once(0..2).collect(), "x".into());
        assert!(invalid.source(true, Options::default()).is_err());
    }

    #[test]
    fn selected_partial_line_tabs_use_original_source_columns() {
        let snapshot =
            Snapshot::new(Rope::from_str("abc\tdef\r\nnext\rfinal"), std::iter::once(3..7).collect(), "x".into());
        assert_eq!(snapshot.source(true, Options::default()).unwrap(), " def");
        assert_eq!(snapshot.source(false, Options::default()).unwrap(), "abc def\nnext\nfinal");
    }
    #[test]
    fn selection_ending_at_newline_does_not_number_unselected_next_line() {
        let snapshot = Snapshot::new(Rope::from_str("one\ntwo\nthree"), std::iter::once(4..8).collect(), "x".into());
        assert_eq!(snapshot.source(true, Options { line_numbers: true, ..Options::default() }).unwrap(), "2  two\n");
    }
}
