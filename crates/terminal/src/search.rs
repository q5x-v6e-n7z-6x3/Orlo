//! Bounded logical-line search of a retained screen snapshot. Coordinates are terminal cells, not chars.

use crate::Screen;
use std::sync::atomic::{AtomicBool, Ordering};

type Position = (usize, u16);
type Match = (Position, Position);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub regex: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self { case_sensitive: true, regex: false }
    }
}

pub struct SearchResults {
    screen: Screen,
    selection: Option<crate::Selection>,
    matches: Vec<Match>,
    retained: usize,
    current: usize,
    text: Vec<String>,
}

impl SearchResults {
    pub fn find(screen: Screen, query: &str, cancel: &AtomicBool) -> Option<Self> {
        Self::find_with_options(screen, query, SearchOptions::default(), cancel).ok().flatten()
    }

    /// Searches only visible cells of an independent snapshot, never the live PTY.
    /// The linear engine excludes backtracking constructs. Work is bounded per logical line.
    pub fn find_with_options(
        mut screen: Screen,
        query: &str,
        options: SearchOptions,
        cancel: &AtomicBool,
    ) -> Result<Option<Self>, String> {
        const LINE_LIMIT: usize = 8 << 20;
        const MATCH_LIMIT: usize = 100_000;
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let pattern = if query.is_empty() {
            None
        } else {
            if query.len() > 8192 {
                return Err("Search pattern exceeds 8 KiB".into());
            }
            let pattern = if options.regex { query.to_string() } else { regex_syntax::escape(query) };
            Some(
                regex_automata::meta::Regex::builder()
                    .syntax(regex_automata::util::syntax::Config::new().case_insensitive(!options.case_sensitive))
                    .configure(regex_automata::meta::Regex::config().nfa_size_limit(Some(10 << 20)))
                    .build(&pattern)
                    .map_err(|e| format!("Invalid search pattern: {e}"))?,
            )
        };
        let original_offset = screen.scrollback();
        screen.set_scrollback(usize::MAX);
        let retained = screen.scrollback();
        let (rows, _) = screen.size();
        let mut text = String::new();
        let mut cells = Vec::new();
        let mut matches = Vec::new();
        let mut matched_text = Vec::new();
        let mut base = 0;
        while base < retained + rows as usize {
            let Some(pattern) = pattern.as_ref() else { break };
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            screen.set_scrollback(retained.saturating_sub(base));
            let count = if base < retained { (retained - base).min(rows as usize) } else { rows as usize };
            for row in 0..count as u16 {
                let wrapped = screen.row_wrapped(row);
                let cols = screen.row_columns(row);
                let end = if wrapped {
                    cols
                } else {
                    (0..cols).rfind(|&col| screen.cell(row, col).is_some_and(|c| c.has_contents())).map_or(0, |c| c + 1)
                };
                for col in 0..end {
                    let Some(cell) = screen.cell(row, col) else { continue };
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let start = text.len();
                    let contents = cell.contents();
                    text.push_str(if contents.is_empty() { " " } else { contents });
                    cells.push((
                        start..text.len(),
                        (base + row as usize, col),
                        (base + row as usize, col + if cell.is_wide() { 2 } else { 1 }),
                    ));
                    if text.len() > LINE_LIMIT {
                        return Err("Search logical line exceeds 8 MiB".into());
                    }
                }
                if !wrapped || base + row as usize + 1 == retained + rows as usize {
                    for hit in pattern.find_iter(&text) {
                        if cancel.load(Ordering::Relaxed) {
                            return Ok(None);
                        }
                        // Zero-width matches have no terminal cells to highlight or copy.
                        if hit.is_empty() {
                            continue;
                        }
                        if matches.len() == MATCH_LIMIT {
                            return Err("Search exceeds 100,000 matches; narrow the query".into());
                        }
                        let first = cells.partition_point(|(r, _, _)| r.end <= hit.start());
                        let last = cells.partition_point(|(r, _, _)| r.start < hit.end()) - 1;
                        matches.push((cells[first].1, cells[last].2));
                        matched_text.push(text[hit.range()].to_string());
                    }
                    text.clear();
                    cells.clear();
                }
            }
            base += count;
        }
        let mut result = Self { screen, selection: None, matches, retained, current: 0, text: matched_text };
        if result.is_empty() {
            result.screen.set_scrollback(original_offset);
        } else {
            result.reveal();
        }
        Ok(Some(result))
    }

    /// Query supersession removes matches, never the independently displayed source/view.
    pub fn clear_matches(&mut self) {
        self.matches.clear();
        self.text.clear();
        self.current = 0;
    }

    /// Transfer a view only between clones of exactly the same source revision and layout.
    pub fn preserve_view_from(&mut self, previous: &Self) -> bool {
        if !self.screen.same_contents(&previous.screen) {
            return false;
        }
        self.current = previous.current().unwrap_or(0).min(self.len().saturating_sub(1));
        self.screen.set_scrollback(previous.screen.scrollback());
        self.selection = previous.selection;
        true
    }

    pub fn selection(&self) -> Option<crate::Selection> {
        self.selection
    }
    pub fn set_selection(&mut self, selection: Option<crate::Selection>) {
        self.selection = selection;
    }
    pub fn extend_selection(&mut self, position: crate::CellPosition) -> bool {
        self.selection.as_mut().map(|(_, end)| *end = position).is_some()
    }
    pub fn screen(&self) -> &Screen {
        &self.screen
    }
    pub fn len(&self) -> usize {
        self.matches.len()
    }
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }
    pub fn current(&self) -> Option<usize> {
        (!self.is_empty()).then_some(self.current)
    }

    pub fn scroll_lines(&mut self, lines: isize) {
        self.selection = None;
        self.screen.set_scrollback(self.screen.scrollback().saturating_add_signed(lines));
    }

    pub fn select(&mut self, index: usize) {
        self.selection = None;
        self.current = index.min(self.len().saturating_sub(1));
        if !self.is_empty() {
            self.reveal();
        }
    }

    pub fn step(&mut self, forward: bool) {
        self.selection = None;
        if !self.is_empty() {
            self.current = (self.current + if forward { 1 } else { self.len() - 1 }) % self.len();
            self.reveal();
        }
    }

    fn reveal(&mut self) {
        let offset = self.matches.get(self.current).map_or(0, |(a, _)| self.retained.saturating_sub(a.0));
        self.screen.set_scrollback(offset);
    }

    pub fn highlights(&self, row: u16, col: u16) -> bool {
        let p = (self.retained - self.screen.scrollback() + row as usize, col);
        self.matches.get(self.current).is_some_and(|&(a, b)| a <= p && p < b)
    }

    pub fn selected_text(&self) -> Option<String> {
        self.current().map(|i| self.text[i].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Emulator;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn superseding_matches_keeps_view_and_only_identical_sources_share_selection() {
        let mut e = Emulator::with_size(2, 12, 40);
        for _ in 0..20 {
            e.process(b"hit selected\r\n");
        }
        let cancel = AtomicBool::new(false);
        let mut previous = SearchResults::find(e.screen().clone(), "hit", &cancel).unwrap();
        previous.select(3);
        previous.scroll_lines(-1);
        previous.set_selection(Some(((0, 4), (0, 12))));
        let offset = previous.screen().scrollback();
        let mut replacement = SearchResults::find(e.screen().clone(), "hit", &cancel).unwrap();
        assert!(replacement.preserve_view_from(&previous));
        assert_eq!(replacement.current(), Some(3));
        assert_eq!(replacement.screen().scrollback(), offset);
        assert_eq!(replacement.selection(), previous.selection());
        previous.clear_matches();
        assert!(previous.is_empty());
        assert!(previous.selected_text().is_none());
        assert_eq!(previous.screen().scrollback(), offset);
        assert_eq!(previous.selection(), Some(((0, 4), (0, 12))));
        previous.set_selection(None);
        let mut replacement = SearchResults::find(e.screen().clone(), "selected", &cancel).unwrap();
        assert!(replacement.preserve_view_from(&previous));
        assert!(replacement.selection().is_none());
        e.process(b"\r\nunrelated");
        let mut live = SearchResults::find(e.screen().clone(), "hit", &cancel).unwrap();
        assert!(!live.preserve_view_from(&previous), "even append without eviction forbids unvalidated cell transfer");
    }

    #[test]
    fn empty_search_and_selection_keep_original_scroll_and_reject_bounded_patterns() {
        let mut e = Emulator::with_size(2, 12, 30);
        for _ in 0..20 {
            e.process(b"retained\r\n");
        }
        e.screen_mut().set_scrollback(10);
        let cancel = AtomicBool::new(false);
        let mut r = SearchResults::find(e.screen().clone(), "", &cancel).unwrap();
        r.set_selection(Some(((0, 0), (0, 8))));
        assert_eq!(r.screen().scrollback(), 10);
        r.select(0);
        assert_eq!(r.screen().scrollback(), 10);
        assert!(SearchResults::find_with_options(
            e.screen().clone(),
            &"a".repeat(8193),
            SearchOptions::default(),
            &cancel
        )
        .is_err());
        let options = SearchOptions { regex: true, ..Default::default() };
        assert!(SearchResults::find_with_options(e.screen().clone(), "a{100000000}", options, &cancel).is_err());
        let r = SearchResults::find_with_options(e.screen().clone(), "^ret.*$", options, &cancel).unwrap().unwrap();
        assert_eq!(r.len(), 20);
        assert_eq!(r.selected_text().as_deref(), Some("retained"));
        let r = SearchResults::find_with_options(e.screen().clone(), "retained\nretained", options, &cancel)
            .unwrap()
            .unwrap();
        assert!(r.is_empty(), "hard lines remain separate");
    }

    #[test]
    fn case_regex_copy_and_unicode_spans_use_actual_logical_source() {
        let mut e = Emulator::with_size(3, 6, 10);
        e.process("中😀e\u{301}AbCabc\r\na.b aab\r\n".as_bytes());
        let cancel = AtomicBool::new(false);
        let options = SearchOptions { case_sensitive: false, regex: false };
        let mut r = SearchResults::find_with_options(e.screen().clone(), "abc", options, &cancel).unwrap().unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r.selected_text().as_deref(), Some("AbC"));
        assert_eq!(r.matches[0], ((0, 5), (1, 2)));
        r.step(true);
        assert_eq!(r.selected_text().as_deref(), Some("abc"));
        let r = SearchResults::find_with_options(e.screen().clone(), "😀e\u{301}", options, &cancel).unwrap().unwrap();
        assert_eq!(r.matches, vec![((0, 2), (0, 5))]);
        let options = SearchOptions { regex: true, ..options };
        let r = SearchResults::find_with_options(e.screen().clone(), "a.c", options, &cancel).unwrap().unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r.selected_text().as_deref(), Some("AbC"));
        let r = SearchResults::find(e.screen().clone(), "a.b", &cancel).unwrap();
        assert_eq!(r.len(), 1, "literal punctuation remains literal");
    }

    #[test]
    fn regex_empty_invalid_zero_width_and_nonoverlapping_candidates() {
        let mut e = Emulator::with_size(2, 20, 10);
        e.process(b"ababa\r\nAbA");
        let cancel = AtomicBool::new(false);
        let options = SearchOptions { case_sensitive: true, regex: true };
        for query in ["", "^", "a{0}"] {
            assert!(
                SearchResults::find_with_options(e.screen().clone(), query, options, &cancel)
                    .unwrap()
                    .unwrap()
                    .is_empty(),
                "{query}"
            );
        }
        assert!(SearchResults::find_with_options(e.screen().clone(), "[", options, &cancel).is_err());
        let r = SearchResults::find_with_options(e.screen().clone(), "aba", options, &cancel).unwrap().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r.selected_text().as_deref(), Some("aba"));
        assert!(SearchResults::find_with_options(e.screen().clone(), "aba", options, &AtomicBool::new(true))
            .unwrap()
            .is_none());
    }

    #[test]
    fn searches_retained_unicode_and_soft_wraps_without_changing_live_screen() {
        let mut emulator = Emulator::with_size(2, 6, 10);
        emulator.process("中e\u{301}XYZabc\r\nlast\r\nnew".as_bytes());
        let live = emulator.screen().contents();
        let result = SearchResults::find(emulator.screen().clone(), "XYZabc", &AtomicBool::new(false)).unwrap();
        assert_eq!(result.matches, vec![((0, 3), (1, 3))]);
        assert_eq!(result.selected_text(), Some("XYZabc".into()));
        assert_eq!(emulator.screen().contents(), live);
        let result = SearchResults::find(emulator.screen().clone(), "e\u{301}", &AtomicBool::new(false)).unwrap();
        assert_eq!(result.matches, vec![((0, 2), (0, 3))]);
    }

    #[test]
    fn navigation_wraps_and_empty_or_cancelled_queries_do_not_match() {
        let mut emulator = Emulator::with_size(2, 10, 2);
        emulator.process(b"hit hit\r\nhit\r\nend");
        let cancel = AtomicBool::new(false);
        let mut found = SearchResults::find(emulator.screen().clone(), "hit", &cancel).unwrap();
        assert_eq!(found.matches.len(), 3);
        found.step(false);
        assert_eq!(found.current(), Some(2));
        found.step(true);
        assert_eq!(found.current(), Some(0));
        assert_eq!(found.selected_text(), Some("hit".into()));
        assert!(SearchResults::find(emulator.screen().clone(), "", &cancel).unwrap().matches.is_empty());
        assert!(SearchResults::find(emulator.screen().clone(), "Hit", &cancel).unwrap().matches.is_empty());
        assert!(SearchResults::find(emulator.screen().clone(), "hit", &AtomicBool::new(true)).is_none());
    }

    #[test]
    fn retention_eviction_alternate_screen_and_blank_cells_have_real_coordinates() {
        let mut emulator = Emulator::with_size(2, 12, 1);
        emulator.process(b"evicted\r\nkept\r\n\x1b[4Ghit\r\nend");
        let cancel = AtomicBool::new(false);
        assert!(SearchResults::find(emulator.screen().clone(), "evicted", &cancel).unwrap().matches.is_empty());
        let found = SearchResults::find(emulator.screen().clone(), "   hit", &cancel).unwrap();
        assert_eq!(found.matches, vec![((1, 0), (1, 6))]);
        emulator.process(b"\x1b[?1049hother");
        assert!(SearchResults::find(emulator.screen().clone(), "kept", &cancel).unwrap().matches.is_empty());
    }

    #[test]
    #[ignore = "release performance gate"]
    fn bench_terminal_search() {
        let mut emulator = Emulator::with_size(24, 80, 10_000);
        for _ in 0..11_000 {
            emulator.process("source/file.rs:12:3 中 e\u{301} error: retained output\r\n".as_bytes());
        }
        let start = std::time::Instant::now();
        let result = SearchResults::find(emulator.screen().clone(), "error", &AtomicBool::new(false)).unwrap();
        let elapsed = start.elapsed();
        assert!(result.len() >= 10_000);
        eprintln!("retained terminal snapshot and search: {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_millis(250));
    }
}
