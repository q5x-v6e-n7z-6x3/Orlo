//! Copy as HTML: coloured lines as the HTML a rich editor pastes.

use crate::languages::highlight::{Look, Run};
use gpui::Hsla;
use ropey::Rope;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

/// `lines` (each one line's text without its break, and its runs: lengths in bytes, colours and
/// looks) as one `<pre>` in the page's colours, font and tab size. A run that differs from the
/// default is a `<span>`; text past the last run is plain. Taken a line at a time, so a big
/// document is never held twice. `size` is the editor's font size: AppKit renders a CSS point at
/// 4/3 of a point, so the page says three quarters of it and pastes at the size it was copied at.
pub fn to_html(
    lines: impl IntoIterator<Item = (String, Vec<Run>)>,
    fg: Hsla,
    bg: Hsla,
    font: &str,
    size: f32,
    tab: usize,
) -> String {
    let fg_hex = hex(fg, bg);
    let mut out = format!("<pre style=\"background-color:{};color:{fg_hex};font-family:'", hex(bg, bg));
    escape(&mut out, &css_string(font));
    // rounded: 12.1 * 0.75 is 9.075001 in f32
    let pt = (size * 0.75 * 1000.).round() / 1000.;
    out.push_str(&format!("',monospace;font-size:{pt}pt;tab-size:{tab};white-space:pre\">"));
    let body = out.len();
    for (i, (text, runs)) in lines.into_iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut at = 0;
        for (len, color, look) in runs {
            let end = (at + len).min(text.len());
            let style = style(color, look, &fg_hex, bg);
            if style.is_empty() {
                escape(&mut out, &text[at..end]);
            } else {
                out.push_str(&format!("<span style=\"{style}\">"));
                escape(&mut out, &text[at..end]);
                out.push_str("</span>");
            }
            at = end;
        }
        escape(&mut out, &text[at..]);
    }
    // an HTML parser drops a newline right after <pre>: a second keeps the first line break
    if out[body..].starts_with('\n') {
        out.insert(body, '\n');
    }
    out.push_str("</pre>");
    out
}

/// The most text, in bytes of UTF-8, Copy as HTML takes: its page is about 7 times as long, and
/// is built, held and written whole.
const MAX_TEXT: usize = 16 * 1024 * 1024;

/// Whether `bytes` of text are more than Copy as HTML takes.
pub fn too_big(bytes: usize) -> bool {
    bytes > MAX_TEXT
}

/// The bytes of UTF-8 in the text the char ranges `ranges` of `rope` copy as: their texts joined
/// by a line break.
pub fn text_len(rope: &Rope, ranges: &[Range<usize>]) -> usize {
    let texts: usize = (ranges.iter()).map(|r| rope.slice(r.clone()).len_bytes()).sum();
    texts + ranges.len().saturating_sub(1)
}

/// `#rrggbb` for `color` as it looks over `over`: a translucent one is mixed with it.
fn hex(color: Hsla, over: Hsla) -> String {
    let c = over.blend(color).to_rgb();
    let byte = |v: f32| (v * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
}

/// What a run adds to the page's look, as the declarations of a `style`: none for the default.
fn style(color: Hsla, look: Look, fg: &str, bg: Hsla) -> String {
    let mut out = String::new();
    let mut add = |declaration: &str| {
        if !out.is_empty() {
            out.push(';');
        }
        out.push_str(declaration);
    };
    let color = hex(color, bg);
    if color != fg {
        add(&format!("color:{color}"));
    }
    if look.bold() {
        add("font-weight:bold");
    }
    if look.italic() {
        add("font-style:italic");
    }
    if look.underline() {
        add("text-decoration:underline");
    }
    if let Some(background) = look.background {
        add(&format!("background-color:{}", hex(background, bg)));
    }
    out
}

/// `s` as the inside of a single-quoted CSS string: `\` and `'` get a backslash, and so does a
/// line break, written as its code, since a string cannot hold one. (HTML's entities come after:
/// a parser turns `&#39;` back into the `'` that would end the string.)
fn css_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '\'' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\a "),
            '\r' => out.push_str("\\d "),
            '\x0c' => out.push_str("\\c "),
            c => out.push(c),
        }
    }
    out
}

/// `s` onto `out`, with `& < > " '` as entities.
fn escape(out: &mut String, s: &str) {
    let mut rest = s;
    while let Some(i) = rest.find(['&', '<', '>', '"', '\'']) {
        out.push_str(&rest[..i]);
        out.push_str(match rest.as_bytes()[i] {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            _ => "&#39;",
        });
        rest = &rest[i + 1..];
    }
    out.push_str(rest);
}

/// The runs of one line cut to its bytes `a..b`.
fn cut(runs: Vec<Run>, a: usize, b: usize) -> Vec<Run> {
    let mut at = 0;
    let mut out = Vec::new();
    for (len, color, look) in runs {
        let (start, end) = (at.max(a), (at + len).min(b));
        if start < end {
            out.push((end - start, color, look));
        }
        at += len;
        if at >= b {
            break;
        }
    }
    out
}

/// The lines the char ranges `ranges` of `rope` cover (in order), as `to_html` takes them: each
/// line's text inside its range and `runs(line)` cut to match. Lines and ranges are joined by a
/// line break, as Copy joins selections. It ends early once `stop` is set, looked at before each
/// line.
pub fn lines<'a>(
    rope: &'a Rope,
    ranges: &'a [Range<usize>],
    stop: &'a AtomicBool,
    mut runs: impl FnMut(usize) -> Vec<Run> + 'a,
) -> impl Iterator<Item = (String, Vec<Run>)> + 'a {
    let mut ranges = ranges.iter();
    // the chars being taken, and the lines of them still to give
    let mut now = (0..0, 0..0);
    std::iter::from_fn(move || {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        while now.1.is_empty() {
            let r = ranges.next()?;
            now = (r.clone(), rope.char_to_line(r.start)..rope.char_to_line(r.end) + 1);
        }
        let line = now.1.next()?;
        Some(piece(rope, &now.0, line, runs(line)))
    })
}

/// Line `l` of `rope` within the chars `r`, with its `runs` cut to match.
fn piece(rope: &Rope, r: &Range<usize>, l: usize, runs: Vec<Run>) -> (String, Vec<Run>) {
    let line = rope.line(l);
    let start = rope.line_to_char(l);
    // the chars before its line break
    let mut content = line.len_chars();
    if content > 0 && line.char(content - 1) == '\n' {
        content -= 1;
        if content > 0 && line.char(content - 1) == '\r' {
            content -= 1;
        }
    }
    let a = r.start.saturating_sub(start).min(content);
    let b = r.end.saturating_sub(start).min(content);
    let runs = match (a, b) == (0, content) {
        true => runs,
        false => cut(runs, line.char_to_byte(a), line.char_to_byte(b)),
    };
    (line.slice(a..b).to_string(), runs)
}

#[cfg(test)]
// the tests hand `lines` one range as often as several
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    use crate::languages::highlight::{Look, Run};
    use gpui::{rgb, Hsla, Rgba};
    use ropey::Rope;
    use std::ops::Range;
    use syntect::highlighting::FontStyle;

    fn fg() -> Hsla {
        rgb(0xd8dee9).into()
    }

    fn bg() -> Hsla {
        rgb(0x303841).into()
    }

    fn plain(text: &str) -> (String, Vec<Run>) {
        (text.to_string(), Vec::new())
    }

    fn page(lines: Vec<(String, Vec<Run>)>) -> String {
        to_html(lines, fg(), bg(), "Menlo", 12., 4)
    }

    /// What is inside the `<pre>`.
    fn body(html: &str) -> &str {
        let open = html.find('>').expect("an open tag") + 1;
        html[open..].strip_suffix("</pre>").expect("a closed <pre>")
    }

    #[test]
    fn the_pre_carries_the_page_colours_font_size_and_tab_size() {
        // an editor size of 14 is 10.5 pt in CSS, which AppKit renders at 14
        let html = to_html(vec![], fg(), bg(), "Menlo", 14., 8);
        assert_eq!(
            html,
            "<pre style=\"background-color:#303841;color:#d8dee9;font-family:'Menlo',\
             monospace;font-size:10.5pt;tab-size:8;white-space:pre\"></pre>"
        );
        assert!(page(vec![]).contains("font-size:9pt;tab-size:4;"));
    }

    #[test]
    fn the_font_size_is_three_quarters_of_the_editors_without_float_noise() {
        let size = |editor: f32| {
            let html = to_html(vec![], fg(), bg(), "Menlo", editor, 4);
            let from = html.find("font-size:").expect("a size") + "font-size:".len();
            let to = from + html[from..].find(';').expect("its end");
            html[from..to].to_string()
        };
        assert_eq!(size(12.), "9pt");
        assert_eq!(size(14.), "10.5pt");
        assert_eq!(size(13.5), "10.125pt");
        assert_eq!(size(72.), "54pt");
        // 12.1 * 0.75 is 9.075001 and 9.9 * 0.75 is 7.4249997 in f32
        assert_eq!(size(12.1), "9.075pt");
        assert_eq!(size(9.9), "7.425pt");
    }

    #[test]
    fn the_text_to_copy_is_counted_in_utf_8_bytes_with_the_joins_between_ranges() {
        // é 2 bytes, 中 3, x 1, a line break 1, z 1, 😀 4
        let rope = Rope::from_str("é中x\nz😀");
        assert_eq!(text_len(&rope, &[0..6]), 12);
        assert_eq!(text_len(&rope, &[0..2, 4..6]), 5 + 1 + 5);
        assert_eq!(text_len(&rope, &[]), 0);
        // it is the length of what Copy puts on the clipboard: the ranges' texts joined by "\n"
        for ranges in [vec![0..6], vec![0..2, 4..6], vec![1..1, 2..5], vec![3..3]] {
            let texts: Vec<String> = (ranges.iter()).map(|r| rope.slice(r.clone()).to_string()).collect();
            assert_eq!(text_len(&rope, &ranges), texts.join("\n").len(), "{ranges:?}");
        }
    }

    #[test]
    fn text_over_16_mib_is_too_big_and_16_mib_is_not() {
        assert!(!too_big(0));
        assert!(!too_big(16 * 1024 * 1024));
        assert!(too_big(16 * 1024 * 1024 + 1));
        assert!(too_big(usize::MAX));
    }

    #[test]
    fn markup_characters_are_escaped_in_the_text_and_the_font_name() {
        let text = plain(r#"<a href="x">&'</a>"#);
        let html = to_html(vec![text], fg(), bg(), r#"Fira "Code""#, 12., 4);
        assert_eq!(body(&html), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;");
        assert!(html.contains("font-family:'Fira &quot;Code&quot;',monospace"), "{html}");
        // and inside a coloured run
        let red = (4, rgb(0xff0000).into(), Look::default());
        let html = page(vec![("a<b>".into(), vec![red])]);
        assert_eq!(body(&html), "<span style=\"color:#ff0000\">a&lt;b&gt;</span>");
    }

    #[test]
    fn spaces_tabs_and_line_breaks_stay_as_they_are() {
        let lines = vec![plain("\tif  x {"), plain("  \t "), plain(""), plain("end ")];
        let html = page(lines);
        assert_eq!(body(&html), "\tif  x {\n  \t \n\nend ");
        assert!(html.contains(";tab-size:4;white-space:pre\">"), "{html}");
    }

    #[test]
    fn a_first_blank_line_gets_a_second_newline_because_html_drops_the_first() {
        // checked on macOS: the HTML importer drops a newline right after <pre>
        assert_eq!(body(&page(vec![plain(""), plain("x")])), "\n\nx");
        assert_eq!(body(&page(vec![plain(""), plain("")])), "\n\n");
        // nothing follows, nothing to keep
        assert_eq!(body(&page(vec![plain("")])), "");
        assert_eq!(body(&page(vec![])), "");
        assert_eq!(body(&page(vec![plain("x"), plain("")])), "x\n");
    }

    #[test]
    fn runs_that_differ_from_the_default_are_spans_with_their_look() {
        let look = |font_style, background| Look { font_style, background };
        let none = FontStyle::empty();
        let (b, i, u) = (FontStyle::BOLD, FontStyle::ITALIC, FontStyle::UNDERLINE);
        let pink: Hsla = rgb(0xc695c6).into();
        let runs = vec![
            // the default: no span
            (2, fg(), Look::default()),
            (2, pink, Look::default()),
            (2, pink, look(b | i | u, None)),
            // the default colour but italic: the style alone
            (2, fg(), look(i, None)),
            (2, fg(), look(none, Some(rgb(0x596673).into()))),
            (2, rgb(0x99c794).into(), look(b, Some(rgb(0xec5f66).into()))),
        ];
        let html = page(vec![("aabbccddeeff".into(), runs)]);
        assert_eq!(
            body(&html),
            concat!(
                "aa",
                "<span style=\"color:#c695c6\">bb</span>",
                "<span style=\"color:#c695c6;font-weight:bold;font-style:italic;\
                 text-decoration:underline\">cc</span>",
                "<span style=\"font-style:italic\">dd</span>",
                "<span style=\"background-color:#596673\">ee</span>",
                "<span style=\"color:#99c794;font-weight:bold;background-color:#ec5f66\">ff</span>",
            )
        );
    }

    #[test]
    fn a_translucent_colour_is_mixed_with_the_page_colour() {
        // Mariana's markup.raw and markup.raw.inline: #596673 at alpha 97 and 128 (of 255)
        let raw = |alpha: f32| -> Hsla {
            Rgba { r: 0x59 as f32 / 255., g: 0x66 as f32 / 255., b: 0x73 as f32 / 255., a: alpha / 255. }.into()
        };
        for (alpha, mixed) in [(97., "#404954"), (128., "#454f5a"), (255., "#596673")] {
            let look = Look { background: Some(raw(alpha)), ..Look::default() };
            let html = page(vec![("x".into(), vec![(1, fg(), look)])]);
            let want = format!("<span style=\"background-color:{mixed}\">x</span>");
            assert_eq!(body(&html), want, "alpha {alpha}");
        }
    }

    #[test]
    fn colours_come_back_as_the_hex_they_started_from() {
        for v in 0..=255u32 {
            for shift in [16, 8, 0] {
                let hex_in = v << shift;
                assert_eq!(hex(rgb(hex_in).into(), bg()), format!("#{hex_in:06x}"));
            }
        }
        for hex_in in [0x000000, 0xffffff, 0x123456, 0xfedcba, 0x7f7f80, 0x0a0b0c] {
            assert_eq!(hex(rgb(hex_in).into(), bg()), format!("#{hex_in:06x}"));
        }
    }

    #[test]
    fn text_past_the_runs_is_plain_and_runs_past_the_text_are_clamped() {
        let red: Hsla = rgb(0xff0000).into();
        let html = page(vec![("abcd".into(), vec![(2, red, Look::default())])]);
        assert_eq!(body(&html), "<span style=\"color:#ff0000\">ab</span>cd");
        let html = page(vec![("ab".into(), vec![(9, red, Look::default())])]);
        assert_eq!(body(&html), "<span style=\"color:#ff0000\">ab</span>");
    }

    #[test]
    fn runs_are_cut_to_a_byte_range() {
        let (r, g): (Hsla, Hsla) = (rgb(0xff0000).into(), rgb(0x00ff00).into());
        let l = Look::default();
        let runs: Vec<Run> = vec![(3, r, l), (4, g, l), (3, r, l)];
        assert_eq!(cut(runs.clone(), 0, 10), runs);
        assert_eq!(cut(runs.clone(), 2, 8), [(1, r, l), (4, g, l), (1, r, l)]);
        assert_eq!(cut(runs.clone(), 3, 7), [(4, g, l)]);
        assert_eq!(cut(runs.clone(), 5, 5), []);
        assert_eq!(cut(runs.clone(), 8, 99), [(2, r, l)]);
        assert_eq!(cut(runs, 0, 0), []);
    }

    /// `lines` over `text`, every char its own run (lengths in bytes), and the lines asked for.
    fn pieces(text: &str, ranges: &[Range<usize>]) -> (Vec<(String, Vec<usize>)>, Vec<usize>) {
        let rope = Rope::from_str(text);
        let mut asked = Vec::new();
        let l = Look::default();
        let found = lines(&rope, ranges, &AtomicBool::new(false), |line| {
            asked.push(line);
            let content = rope.line(line).to_string();
            let content = content.trim_end_matches(['\n', '\r']);
            content.chars().map(|c| (c.len_utf8(), fg(), l)).collect()
        })
        .map(|(text, runs)| (text, runs.iter().map(|r| r.0).collect()))
        .collect();
        (found, asked)
    }

    fn piece(text: &str, runs: &[usize]) -> (String, Vec<usize>) {
        (text.to_string(), runs.to_vec())
    }

    #[test]
    fn the_lines_of_a_whole_document_are_all_its_lines_with_a_last_empty_one() {
        let (found, asked) = pieces("ab\n\ncd\n", &[0..7]);
        assert_eq!(found, [piece("ab", &[1, 1]), piece("", &[]), piece("cd", &[1, 1]), piece("", &[])]);
        assert_eq!(asked, [0, 1, 2, 3]);
        assert_eq!(pieces("", &[0..0]).0, [piece("", &[])]);
    }

    #[test]
    fn a_selection_gives_the_part_of_its_first_and_last_line_it_covers() {
        // "world\nsecond" of three lines
        let text = "hello world\nsecond line\nthird";
        let (found, asked) = pieces(text, &[6..18]);
        assert_eq!(found, [piece("world", &[1; 5]), piece("second", &[1; 6])]);
        assert_eq!(asked, [0, 1]);
        // inside one line
        assert_eq!(pieces(text, &[2..4]).0, [piece("ll", &[1, 1])]);
    }

    #[test]
    fn a_selection_ending_at_a_line_start_ends_with_an_empty_piece() {
        // its text is "ab\n"
        assert_eq!(pieces("ab\ncd", &[0..3]).0, [piece("ab", &[1, 1]), piece("", &[])]);
        // one that starts at a line break begins with an empty piece
        assert_eq!(pieces("ab\ncd", &[2..5]).0, [piece("", &[]), piece("cd", &[1, 1])]);
    }

    #[test]
    fn selections_follow_each_other_even_on_one_line() {
        let (found, asked) = pieces("abcdef\nxyz", &[0..2, 4..6, 7..9]);
        assert_eq!(found, [piece("ab", &[1, 1]), piece("ef", &[1, 1]), piece("xy", &[1, 1])]);
        assert_eq!(asked, [0, 0, 1]);
    }

    #[test]
    fn offsets_are_chars_and_runs_are_cut_by_bytes() {
        // é is 2 bytes, 中 3, x 1: chars 1..4 are "中x\n"
        let (found, _) = pieces("é中x\nz", &[1..4]);
        assert_eq!(found, [piece("中x", &[3, 1]), piece("", &[])]);
        assert_eq!(pieces("é中x\nz", &[0..2]).0, [piece("é中", &[2, 3])]);
    }

    #[test]
    fn the_lines_end_between_two_lines_once_told_to_stop() {
        let rope = Rope::from_str("a\nb\nc\nd\ne\n");
        let stop = AtomicBool::new(false);
        let mut asked = Vec::new();
        let found: Vec<_> = lines(&rope, &[0..rope.len_chars()], &stop, |line| {
            asked.push(line);
            // the stop comes while line 2 is being coloured
            stop.store(line == 2, Ordering::Relaxed);
            Vec::new()
        })
        .map(|(text, _)| text)
        .collect();
        assert_eq!(found, ["a", "b", "c"]);
        assert_eq!(asked, [0, 1, 2], "no line is coloured after it");
        // told before the first one: nothing at all
        let mut asked = Vec::new();
        let stop = AtomicBool::new(true);
        let count = lines(&rope, &[0..3], &stop, |line| {
            asked.push(line);
            Vec::new()
        })
        .count();
        assert_eq!((count, asked), (0, vec![]));
    }

    #[test]
    fn a_font_name_is_escaped_for_css_and_then_for_html() {
        let font = |name: &str| {
            let html = to_html(vec![], fg(), bg(), name, 12., 4);
            let from = html.find("font-family:").expect("a font") + "font-family:".len();
            let to = html.find(",monospace").expect("its end");
            html[from..to].to_string()
        };
        assert_eq!(font("Menlo"), "'Menlo'");
        // a ' or \ would end or break the CSS string, and an HTML parser turns &#39; back into '
        assert_eq!(font(r"O'Brien\Mono"), r"'O\&#39;Brien\\Mono'");
        // a line break cannot be in a CSS string either
        assert_eq!(font("Two\nLines\r\x0c"), r"'Two\a Lines\d \c '");
        // " is only HTML's to escape
        assert_eq!(font(r#"Fira "Code""#), "'Fira &quot;Code&quot;'");
        // and the rest of the style is still there
        let html = to_html(vec![], fg(), bg(), r"a'b\c", 12., 4);
        assert!(html.ends_with(",monospace;font-size:9pt;tab-size:4;white-space:pre\"></pre>"), "{html}");
    }

    #[test]
    fn a_crlf_is_not_part_of_a_lines_text() {
        let (found, _) = pieces("a\r\nb", &[0..4]);
        assert_eq!(found, [piece("a", &[1]), piece("b", &[1])]);
    }
}
