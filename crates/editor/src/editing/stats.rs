//! Edit > Document Statistics and Character Info: what the text holds, counted in Unicode terms.

use crate::search::pattern::group_digits as group;
use orlo_project::encoding::Encoding;
use ropey::Rope;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Buffers longer than this many chars are counted off the UI thread.
pub const BIG: usize = 4_000_000;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Counts {
    pub chars: usize,
    pub graphemes: usize,
    pub words: usize,
    pub lines: usize,
}

/// The document's counts, its size as saved, and the totals over the non-empty selections
/// with how many there are.
pub struct Stats {
    pub doc: Counts,
    pub bytes: Result<usize, String>,
    pub encoding: Encoding,
    pub selection: Option<(Counts, usize)>,
}

impl Counts {
    fn plus(self, o: Counts) -> Counts {
        Counts {
            chars: self.chars + o.chars,
            graphemes: self.graphemes + o.graphemes,
            words: self.words + o.words,
            lines: self.lines + o.lines,
        }
    }

    fn rows(&self) -> [usize; 4] {
        [self.chars, self.graphemes, self.words, self.lines]
    }
}

pub fn counts(text: &str) -> Counts {
    let terminators = text.bytes().filter(|&b| b == b'\n').count();
    Counts {
        chars: text.chars().count(),
        graphemes: text.graphemes(true).count(),
        // UAX #29 segments that hold a letter or digit: not the spaces and punctuation between
        words: text.unicode_words().count(),
        lines: terminators + usize::from(!text.is_empty() && !text.ends_with('\n')),
    }
}

/// The encoded bytes a save writes; Unicode sizes are counted, legacy conversions validated.
pub fn saved_len(rope: &Rope, ending: &str, bom: bool, encoding: Encoding) -> std::io::Result<usize> {
    if !encoding.bom_bytes().is_empty() {
        // Two UTF-16 scalars can span a four-byte BOM, even without a requested BOM.
        orlo_project::encoding::encode(&rope.chars().take(2).collect::<String>(), encoding, bom)?;
    }
    let breaks: usize = (rope.chunks()).map(|c| c.bytes().filter(|&b| b == b'\n').count()).sum();
    Ok(match encoding {
        Encoding::Utf8 => rope.len_bytes() + breaks * (ending.len() - 1) + if bom { 3 } else { 0 },
        Encoding::Utf16Le | Encoding::Utf16Be => {
            2 * (rope.len_utf16_cu() + breaks * (ending.len() - 1)) + if bom { 2 } else { 0 }
        }
        Encoding::Utf32Le | Encoding::Utf32Be => {
            4 * (rope.len_chars() + breaks * (ending.len() - 1)) + if bom { 4 } else { 0 }
        }
        _ => crate::buffer::encoded(rope.to_string(), ending, encoding, bom)?.len(),
    })
}

/// `selected` may hold carets: they count for nothing.
pub fn stats(rope: &Rope, selected: &[Range<usize>], ending: &str, bom: bool, encoding: Encoding) -> Stats {
    let selected: Vec<_> = selected.iter().filter(|r| !r.is_empty()).collect();
    let totals =
        (selected.iter()).map(|r| counts(&rope.slice((*r).clone()).to_string())).fold(Counts::default(), Counts::plus);
    Stats {
        doc: counts(&rope.to_string()),
        bytes: saved_len(rope, ending, bom, encoding).map_err(|e| e.to_string()),
        encoding,
        selection: (!selected.is_empty()).then_some((totals, selected.len())),
    }
}

impl Stats {
    /// One line a count; the selection's total follows the document's when something is selected.
    pub fn detail(&self) -> String {
        let names = ["Characters", "Grapheme clusters", "Words", "Lines"];
        let mut lines = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let mut line = format!("{name}: {}", group(self.doc.rows()[i]));
            if let Some((sel, n)) = &self.selection {
                let total = group(sel.rows()[i]);
                line += &format!(" (selection {total} in {})", plural(*n, "selection"));
            }
            lines.push(line);
        }
        lines.push(match &self.bytes {
            Ok(bytes) => format!("{} bytes as saved: {}", self.encoding.name(), group(*bytes)),
            Err(error) => error.clone(),
        });
        lines.join("\n")
    }
}

/// A mark joins the cluster of the letter before it, whatever its script.
fn is_mark(c: char) -> bool {
    format!("a{c}").graphemes(true).count() == 1
}

fn kind(c: char) -> &'static str {
    match c {
        '\u{200D}' => "joiner",
        '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}' => "variation selector",
        // the invisible format characters people meet, not the whole Unicode Cf table;
        // generate one if another shows up as a symbol
        '\u{AD}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}' => "format",
        c if is_mark(c) => "combining mark",
        '\n' => "line break",
        // a tab is both; whitespace says more
        c if c.is_whitespace() => "whitespace",
        c if c.is_control() => "control",
        c if c.is_alphabetic() => "letter",
        c if c.is_numeric() => "digit",
        _ => "symbol/other",
    }
}

/// One line a scalar: code point, UTF-8 bytes and UTF-16 units in hex, and what it is.
pub fn char_info(cluster: &str) -> Vec<String> {
    cluster
        .chars()
        .map(|c| {
            let s = c.to_string();
            let utf8: Vec<_> = (s.bytes()).map(|b| format!("{b:02X}")).collect();
            let utf16: Vec<_> = (s.encode_utf16()).map(|u| format!("{u:04X}")).collect();
            let (utf8, utf16) = (utf8.join(" "), utf16.join(" "));
            format!("U+{:04X}  UTF-8 {utf8}  UTF-16 {utf16}  {}", c as u32, kind(c))
        })
        .collect()
}

fn plural(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// The alert text for the cluster after the caret (`None` at the end of the document): each
/// scalar, then the cluster's totals; a line break says how a save writes it.
pub fn char_report(cluster: Option<&str>, ending: &str) -> String {
    let Some(cluster) = cluster else {
        return "End of document".into();
    };
    let mut lines = char_info(cluster);
    if cluster == "\n" {
        let name = match ending {
            "\r\n" => "Windows (CRLF)",
            "\r" => "Mac OS 9 (CR)",
            _ => "Unix (LF)",
        };
        lines[0] += &format!(" (saved as {name})");
    }
    lines.push(format!(
        "Cluster: {}, {}, {}",
        plural(cluster.chars().count(), "scalar"),
        plural(cluster.len(), "UTF-8 byte"),
        plural(cluster.encode_utf16().count(), "UTF-16 unit"),
    ));
    lines.join("\n")
}

/// Saved size with physical boundary metadata; legacy callers retain single-ending behavior.
pub fn saved_len_with_endings(
    rope: &Rope,
    endings: Option<&crate::buffer::LineEndings>,
    preferred: &str,
    bom: bool,
    encoding: Encoding,
) -> std::io::Result<usize> {
    let Some(map) = endings else { return saved_len(rope, preferred, bom, encoding) };
    let counts = map.counts();
    if counts.iter().sum::<usize>() != rope.len_lines() - 1 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Line ending map does not match source"));
    }
    let width = match encoding {
        Encoding::Utf8 => 1,
        Encoding::Utf16Le | Encoding::Utf16Be => 2,
        Encoding::Utf32Le | Encoding::Utf32Be => 4,
        _ => {
            return orlo_project::encoding::encode_owned(map.text(rope, 0..rope.len_chars())?, encoding, bom)
                .map(|b| b.len());
        }
    };
    Ok(saved_len(rope, "\n", bom, encoding)? + counts[1] * width)
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    use crate::buffer::EditorState;

    fn c(chars: usize, graphemes: usize, words: usize, lines: usize) -> Counts {
        Counts { chars, graphemes, words, lines }
    }

    /// What `char_info` says scalar `i` of `s` is.
    fn kind_of(s: &str, i: usize) -> String {
        let line = &char_info(s)[i];
        line.rsplit("  ").next().unwrap().to_string()
    }

    #[test]
    fn legacy_statistics_and_checksums_refuse_unrepresentable_bytes() {
        let mut state = EditorState::from_text("café\n");
        state.set_encoding(Encoding::Windows1252, false);
        assert_eq!(saved_len(&state.doc.rope, "\r\n", false, Encoding::Windows1252).unwrap(), 6);
        state.insert("🦀");
        let location = "'🦀' (U+1F980) at line 1, column 1";
        assert!(state.bytes_as_saved(None).unwrap_err().to_string().contains(location));
        assert!(state.hash_selections(crate::editing::convert::Hash::Md5).unwrap_err().to_string().contains(location));
        let detail = stats(&state.doc.rope, &[], "\n", false, Encoding::Windows1252);
        assert!(detail.bytes.is_err());
        assert!(detail.detail().contains(location));
    }

    #[test]
    fn utf32_saved_size_counts_scalars_instead_of_utf16_units() {
        assert_eq!(saved_len(&Rope::from_str("A😀"), "\n", true, Encoding::Utf32Le).unwrap(), 12);
    }

    #[test]
    fn saved_size_rejects_bom_prefixes_spanning_utf16_scalars() {
        for (encoding, text) in [(Encoding::Utf16Be, "\0\u{feff}"), (Encoding::Utf16Le, "\0\u{fffe}")] {
            let mut state = EditorState::from_text(text);
            state.set_encoding(encoding, false);
            assert!(state.bytes_as_saved(None).is_err(), "{encoding:?}: serialization must refuse the BOM collision");
            assert!(
                saved_len(&state.doc.rope, "\n", false, encoding).is_err(),
                "{encoding:?}: size must agree with serialization"
            );
            assert!(stats(&state.doc.rope, &[], "\n", false, encoding).bytes.is_err());
            let valid = EditorState::from_text("\0x");
            assert_eq!(saved_len(&valid.doc.rope, "\n", false, encoding).unwrap(), 4);
        }
    }

    #[test]
    fn statistics_match_encoded_save_bytes() {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be, Encoding::Utf32Le, Encoding::Utf32Be] {
            let mut s = crate::buffer::EditorState::from_text("a🦀\nb\n");
            s.set_encoding(encoding, true);
            s.doc.line_ending = "\r\n";
            let stats = stats(&s.doc.rope, &[], s.doc.line_ending, s.doc.bom, encoding);
            assert_eq!(*stats.bytes.as_ref().unwrap(), s.bytes_as_saved(None).unwrap().len());
            assert!(stats.detail().contains(encoding.name()));
        }
    }

    #[test]
    fn lines_are_terminators_plus_an_unterminated_last_line() {
        for (text, lines) in [("", 0), ("a", 1), ("a\n", 1), ("a\nb", 2), ("\n", 1), ("\n\n", 2)] {
            assert_eq!(counts(text).lines, lines, "{text:?}");
        }
        assert_eq!(counts("hello world"), c(11, 11, 2, 1));
    }

    #[test]
    fn a_decomposed_letter_and_a_zwj_family_are_one_grapheme() {
        assert_eq!(counts("e\u{301}"), c(2, 1, 1, 1));
        assert_eq!(counts("👩\u{200d}👩\u{200d}👧"), c(5, 1, 0, 1));
    }

    #[test]
    fn ideographs_are_words_as_uax_29_splits_them() {
        let text = "日本語 text";
        let words = text.unicode_words().count();
        assert_eq!(counts(text).words, words);
        assert_eq!(words, 4);
        // punctuation and symbols alone are not words
        assert_eq!(counts("-- ... 42 ✓").words, 1);
    }

    #[test]
    fn saved_size_applies_the_line_ending_and_bom() {
        let mut s = EditorState::from_text("a\nbé\n\nc");
        assert_eq!(saved_len(&s.doc.rope, "\n", false, Encoding::Utf8).unwrap(), 8);
        assert_eq!(saved_len(&s.doc.rope, "\r\n", false, Encoding::Utf8).unwrap(), 11);
        assert_eq!(saved_len(&s.doc.rope, "\r\n", true, Encoding::Utf8).unwrap(), 14);
        // the same bytes a save writes
        for (ending, bom) in [("\r\n", true), ("\r", false)] {
            (s.doc.line_ending, s.doc.bom) = (ending, bom);
            let saved = s.bytes_as_saved(None).unwrap().len();
            assert_eq!(saved_len(&s.doc.rope, ending, bom, Encoding::Utf8).unwrap(), saved);
        }
        assert_eq!(saved_len(&Rope::new(), "\r\n", true, Encoding::Utf8).unwrap(), 3);
    }

    #[test]
    fn the_selection_column_totals_every_non_empty_selection() {
        let rope = Rope::from_str("ab cd\nef gh");
        let all = stats(&rope, &[0..2, 6..11, 4..4], "\n", false, Encoding::Utf8);
        assert_eq!(all.doc, c(11, 11, 4, 2));
        assert_eq!(all.bytes, Ok(11));
        // the caret is no selection
        assert_eq!(all.selection, Some((c(7, 7, 3, 2), 2)));
        let none = stats(&rope, &[3..3], "\n", false, Encoding::Utf8);
        assert_eq!(none.selection, None);
        assert!(!none.detail().contains("selection"));
        assert_eq!(
            all.detail(),
            "Characters: 11 (selection 7 in 2 selections)\n\
             Grapheme clusters: 11 (selection 7 in 2 selections)\n\
             Words: 4 (selection 3 in 2 selections)\n\
             Lines: 2 (selection 2 in 2 selections)\n\
             UTF-8 bytes as saved: 11"
        );
        let one = stats(&rope, &[0..2], "\n", false, Encoding::Utf8).detail();
        assert!(one.starts_with("Characters: 11 (selection 2 in 1 selection)\n"), "{one}");
    }

    #[test]
    fn numbers_have_thousands_separators() {
        let rope = Rope::from_str(&"x".repeat(1_234_567));
        let detail = stats(&rope, &[], "\n", false, Encoding::Utf8).detail();
        assert!(detail.starts_with("Characters: 1,234,567\n"), "{detail}");
        assert!(detail.ends_with("UTF-8 bytes as saved: 1,234,567"), "{detail}");
    }

    #[test]
    fn character_info_lists_every_scalar_of_a_zwj_sequence() {
        assert_eq!(
            char_info("👩\u{200d}👧"),
            [
                "U+1F469  UTF-8 F0 9F 91 A9  UTF-16 D83D DC69  symbol/other",
                "U+200D  UTF-8 E2 80 8D  UTF-16 200D  joiner",
                "U+1F467  UTF-8 F0 9F 91 A7  UTF-16 D83D DC67  symbol/other",
            ]
        );
    }

    #[test]
    fn character_info_names_the_kind_of_each_scalar() {
        assert_eq!(
            char_info("e\u{301}"),
            ["U+0065  UTF-8 65  UTF-16 0065  letter", "U+0301  UTF-8 CC 81  UTF-16 0301  combining mark",]
        );
        let s = "5 \t\u{fe0f}é";
        let kinds: Vec<String> = (0..char_info(s).len()).map(|i| kind_of(s, i)).collect();
        assert_eq!(kinds, ["digit", "whitespace", "whitespace", "variation selector", "letter"]);
        assert!(char_info("\u{7}")[0].ends_with("control"));
    }

    #[test]
    fn marks_of_every_script_are_combining_and_invisible_format_characters_are_format() {
        // Devanagari vowel sign, Arabic shadda, Hebrew point, Thai mai han-akat, kana voiced mark
        for (s, mark) in [
            ("\u{915}\u{93e}", 1),
            ("\u{628}\u{651}", 1),
            ("\u{5d0}\u{5b8}", 1),
            ("\u{e01}\u{e31}", 1),
            ("\u{304b}\u{3099}", 1),
            ("e\u{301}", 1),
        ] {
            assert_eq!(kind_of(s, mark), "combining mark", "{s:?}");
            assert_eq!(kind_of(s, 0), "letter", "{s:?}");
        }
        // the joiner and the selectors keep their own names
        assert_eq!(kind_of("\u{200d}", 0), "joiner");
        assert_eq!(kind_of("\u{fe0f}", 0), "variation selector");
        for c in ['\u{200b}', '\u{feff}', '\u{202e}', '\u{ad}', '\u{200e}', '\u{2060}'] {
            assert_eq!(kind_of(&c.to_string(), 0), "format", "{c:?}");
        }
        assert_eq!(kind_of("✓", 0), "symbol/other");
    }

    #[test]
    fn the_report_totals_the_cluster_and_notes_line_breaks_and_the_end() {
        let accent = char_report(Some("e\u{301}"), "\n");
        assert!(accent.ends_with("Cluster: 2 scalars, 3 UTF-8 bytes, 2 UTF-16 units"), "{accent}");
        assert!(char_report(Some("a"), "\n").ends_with("Cluster: 1 scalar, 1 UTF-8 byte, 1 UTF-16 unit"));
        let lf = char_report(Some("\n"), "\r\n");
        assert!(lf.starts_with("U+000A  UTF-8 0A  UTF-16 000A  line break (saved as Windows (CRLF))"), "{lf}");
        assert!(char_report(Some("\n"), "\n").contains("saved as Unix (LF)"));
        assert_eq!(char_report(None, "\n"), "End of document");
    }
}
