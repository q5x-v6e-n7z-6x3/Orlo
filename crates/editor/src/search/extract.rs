//! Matches or matching lines as text, for the clipboard or a new document. Pure.
//!
//! Sources are listed in order and each one's matches in document order. A match's text goes
//! out whole, line breaks and all, followed by a line break; an empty match has no text and
//! gives nothing. In line mode every line a match touches goes out once (a match ending just
//! after a line break does not touch the next line), so an empty match gives its line.
//! Locations are `name:line:column: ` for matches and `name:line: ` for lines, 1-based, the
//! column in characters.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Matches,
    Lines,
}

/// One text and its matches (bytes, sorted).
pub struct Source<'a> {
    pub name: String,
    pub text: &'a str,
    pub matches: Vec<Range<usize>>,
}

/// The extraction; `None` when nothing came out (no matches, or only empty ones).
pub fn extract(sources: &[Source], what: What, locations: bool) -> Option<String> {
    let mut out = String::new();
    for s in sources {
        let starts: Vec<usize> = std::iter::once(0).chain(s.text.match_indices('\n').map(|(i, _)| i + 1)).collect();
        let line_of = |b: usize| starts.partition_point(|&s| s <= b) - 1;
        let line = |l: usize| {
            let end = starts.get(l + 1).map_or(s.text.len(), |e| e - 1);
            &s.text[starts[l]..end]
        };
        let mut last_line = None;
        // (line, byte, its column): matches come in order, so a column counts on from the last
        let mut at: Option<(usize, usize, usize)> = None;
        for m in &s.matches {
            match what {
                What::Matches => {
                    if m.is_empty() {
                        continue;
                    }
                    if locations {
                        let l = line_of(m.start);
                        let col = match at {
                            Some((line, byte, col)) if line == l && byte <= m.start => {
                                col + s.text[byte..m.start].chars().count()
                            }
                            _ => s.text[starts[l]..m.start].chars().count(),
                        };
                        at = Some((l, m.start, col));
                        out += &format!("{}:{}:{}: ", s.name, l + 1, col + 1);
                    }
                    out += &s.text[m.clone()];
                    out.push('\n');
                }
                What::Lines => {
                    let first = line_of(m.start);
                    let mut last = line_of(m.end);
                    if m.end > m.start && last > first && starts[last] == m.end {
                        last -= 1;
                    }
                    for l in first..=last {
                        if last_line.is_some_and(|p| p >= l) {
                            continue;
                        }
                        last_line = Some(l);
                        if locations {
                            out += &format!("{}:{}: ", s.name, l + 1);
                        }
                        out += line(l);
                        out.push('\n');
                    }
                }
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    #[test]
    fn match_columns_count_on_along_a_long_line() {
        let text = "ab é".repeat(50_000);
        let matches: Vec<_> = (0..50_000).map(|i| i * 5..i * 5 + 2).collect();
        let t = std::time::Instant::now();
        let out = extract(&[Source { name: "f".into(), text: &text, matches }], What::Matches, true).unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
        assert!(out.starts_with("f:1:1: ab\nf:1:5: ab\n"), "{}", &out[..40]);
        assert!(out.ends_with("f:1:199997: ab\n"));
    }

    fn src<'a>(name: &str, text: &'a str, m: &[Range<usize>]) -> Source<'a> {
        Source { name: name.into(), text, matches: m.to_vec() }
    }

    #[test]
    fn matches_and_lines_in_order_each_line_once() {
        let text = "a1 b2\nc3\nnone\nd4 e5\n";
        let m = [1..2, 4..5, 7..8, 15..16, 18..19];
        let s = [src("f.txt", text, &m)];
        assert_eq!(extract(&s, What::Matches, false).unwrap(), "1\n2\n3\n4\n5\n");
        assert_eq!(extract(&s, What::Lines, false).unwrap(), "a1 b2\nc3\nd4 e5\n");
        assert_eq!(
            extract(&s, What::Matches, true).unwrap(),
            "f.txt:1:2: 1\nf.txt:1:5: 2\nf.txt:2:2: 3\nf.txt:4:2: 4\nf.txt:4:5: 5\n"
        );
        assert_eq!(extract(&s, What::Lines, true).unwrap(), "f.txt:1: a1 b2\nf.txt:2: c3\nf.txt:4: d4 e5\n");
    }

    #[test]
    fn multiline_and_empty_matches() {
        let text = "ab\ncd\nef";
        // "b\nc", "d\n" (ends after a break), an empty match on the last line
        let m = [1..4, 4..6, 7..7];
        let s = [src("x", text, &m)];
        assert_eq!(extract(&s, What::Matches, false).unwrap(), "b\nc\nd\n\n", "whole, breaks kept");
        assert_eq!(extract(&s, What::Lines, false).unwrap(), "ab\ncd\nef\n");
        let only_empty = [src("x", text, &[0..0])];
        assert_eq!(extract(&only_empty, What::Matches, false), None);
        assert_eq!(extract(&only_empty, What::Lines, false).unwrap(), "ab\n");
        assert_eq!(extract(&[src("x", text, &[])], What::Lines, false), None);
    }

    #[test]
    fn columns_count_characters_and_sources_stay_in_order() {
        let a = "é😀 x";
        let s = [src("a", a, &[7..8]), src("b", "x", &[0..1])];
        assert_eq!(extract(&s, What::Matches, true).unwrap(), "a:1:4: x\nb:1:1: x\n");
    }
}
