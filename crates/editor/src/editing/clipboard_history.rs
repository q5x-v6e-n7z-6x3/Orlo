//! Paste from History: what Orlo's own Copy and Cut put on the clipboard, newest first.
//! In memory only; nothing here reaches the session or disk.

/// Entries kept; Sublime's history is the same size.
const MAX_ENTRIES: usize = 15;
/// Bigger copies are left out so fifteen of them cannot hold the memory of a whole file.
const MAX_BYTES: usize = 4 << 20;

/// Exactly what a Copy or Cut wrote: the text and `EditorState::copy_text`'s whole-lines flag.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipEntry {
    pub text: String,
    pub whole_lines: bool,
}

impl ClipEntry {
    /// The first line, tabs shown as → and spaces as ·, cut at 80 characters; ⏎ when it is empty.
    pub fn label(&self) -> String {
        let first = self.text.lines().next().unwrap_or("");
        if first.is_empty() {
            return "⏎".into();
        }
        let mut label: String = (first.chars().take(80))
            .map(|c| match c {
                '\t' => '→',
                ' ' => '·',
                c => c,
            })
            .collect();
        if first.chars().nth(80).is_some() {
            label.push('…');
        }
        label
    }

    /// "3 lines · whole lines".
    pub fn detail(&self) -> String {
        let n = self.text.lines().count();
        let lines = format!("{n} line{}", if n == 1 { "" } else { "s" });
        match self.whole_lines {
            true => format!("{lines} · whole lines"),
            false => lines,
        }
    }
}

/// App-wide, newest entry first.
#[derive(Default)]
pub struct ClipboardHistory {
    entries: Vec<ClipEntry>,
}

impl ClipboardHistory {
    pub fn record(&mut self, text: &str, whole_lines: bool) {
        if text.is_empty() || text.len() > MAX_BYTES {
            return;
        }
        let entry = ClipEntry { text: text.into(), whole_lines };
        self.entries.retain(|e| *e != entry);
        self.entries.insert(0, entry);
        self.entries.truncate(MAX_ENTRIES);
    }

    pub fn entries(&self) -> &[ClipEntry] {
        &self.entries
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(h: &ClipboardHistory) -> Vec<&str> {
        h.entries().iter().map(|e| e.text.as_str()).collect()
    }

    #[test]
    fn records_text_and_flag_newest_first() {
        let mut h = ClipboardHistory::default();
        h.record("one", false);
        h.record("two\n", true);
        assert_eq!(h.entries(), [entry("two\n", true), entry("one", false)]);
    }

    #[test]
    fn empty_text_is_not_recorded() {
        let mut h = ClipboardHistory::default();
        h.record("", false);
        h.record("", true);
        assert!(h.entries().is_empty());
    }

    #[test]
    fn an_equal_entry_moves_to_the_top_but_a_different_flag_is_another_entry() {
        let mut h = ClipboardHistory::default();
        h.record("a", false);
        h.record("b", false);
        h.record("a", false);
        assert_eq!(texts(&h), ["a", "b"]);
        h.record("b", true);
        assert_eq!(texts(&h), ["b", "a", "b"]);
    }

    #[test]
    fn keeps_the_fifteen_newest() {
        let mut h = ClipboardHistory::default();
        for i in 0..20 {
            h.record(&i.to_string(), false);
        }
        assert_eq!(h.entries().len(), 15);
        assert_eq!(h.entries()[0].text, "19");
        assert_eq!(h.entries()[14].text, "5");
    }

    #[test]
    fn entries_over_four_megabytes_are_not_kept() {
        let mut h = ClipboardHistory::default();
        h.record(&"x".repeat(4 << 20), false);
        h.record(&"x".repeat((4 << 20) + 1), false);
        assert_eq!(h.entries().len(), 1, "exactly 4 MB is kept, one more is not");
    }

    #[test]
    fn clear_empties_the_list() {
        let mut h = ClipboardHistory::default();
        h.record("a", false);
        h.clear();
        assert!(h.entries().is_empty());
    }

    fn entry(text: &str, whole_lines: bool) -> ClipEntry {
        ClipEntry { text: text.into(), whole_lines }
    }

    #[test]
    fn the_label_is_the_first_line_with_tabs_shown() {
        assert_eq!(entry("a\tb\nsecond", false).label(), "a→b");
        assert_eq!(entry("only\r\nline", false).label(), "only");
    }

    #[test]
    fn the_label_shows_spaces_and_an_empty_first_line() {
        assert_eq!(entry("  a b \n", false).label(), "··a·b·");
        assert_eq!(entry("   ", false).label(), "···");
        assert_eq!(entry("\n", true).label(), "⏎");
        assert_eq!(entry("\nfoo", false).label(), "⏎");
    }

    #[test]
    fn the_label_is_cut_at_80_characters() {
        assert_eq!(entry(&"é".repeat(80), false).label(), "é".repeat(80));
        assert_eq!(entry(&"é".repeat(81), false).label(), format!("{}…", "é".repeat(80)));
    }

    #[test]
    fn the_detail_counts_lines_and_says_whole_lines() {
        assert_eq!(entry("a", false).detail(), "1 line");
        assert_eq!(entry("a\nb\nc", false).detail(), "3 lines");
        assert_eq!(entry("a\nb\nc\n", true).detail(), "3 lines · whole lines");
        assert_eq!(entry("a\n", true).detail(), "1 line · whole lines");
    }
}
