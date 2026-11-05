//! CSV and TSV columns: a lenient RFC 4180 parser over char offsets, delimiter detection,
//! and the column commands as selections or edits. Field text never leaves the document except
//! to be moved as it is, so quoting, multi-line records and line breaks survive.

use std::collections::HashMap;
use std::ops::Range;

/// Records read to pick a delimiter; a bigger file is not read further.
pub const DETECT_RECORDS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delim {
    Comma,
    Tab,
    Semicolon,
    Pipe,
}

/// Detection's order, which also settles ties.
const ORDER: [Delim; 4] = [Delim::Tab, Delim::Comma, Delim::Semicolon, Delim::Pipe];

impl Delim {
    pub fn ch(self) -> char {
        match self {
            Delim::Comma => ',',
            Delim::Tab => '\t',
            Delim::Semicolon => ';',
            Delim::Pipe => '|',
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Delim::Comma => "Comma",
            Delim::Tab => "Tab",
            Delim::Semicolon => "Semicolon",
            Delim::Pipe => "Pipe",
        }
    }
}

/// One field's raw text (quotes included) as a range of char offsets into the parsed text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub raw: Range<usize>,
}

struct Parsed {
    records: Vec<Vec<Field>>,
    quoted: usize,
    /// The line the record with an unterminated quote starts on; the records before it are kept.
    unclosed: Option<usize>,
}

/// The first `max` records of `text`. A quote opens a quoted field only at its start; a
/// quoted field runs to its closing quote, and what follows it up to the delimiter stays in
/// the field. A `\r` right before a line break is part of the break. A final line break ends
/// the last record instead of starting one.
fn parse(text: &str, d: Delim, max: usize) -> Parsed {
    let delim = d.ch();
    let mut it = text.chars().peekable();
    let (mut pos, mut line) = (0, 1);
    let mut records = Vec::new();
    let mut quoted = 0;
    while it.peek().is_some() && records.len() < max {
        let first_line = line;
        let mut fields = Vec::new();
        loop {
            let start = pos;
            if it.next_if_eq(&'"').is_some() {
                pos += 1;
                loop {
                    match it.next() {
                        None => return Parsed { records, quoted, unclosed: Some(first_line) },
                        Some('"') => {
                            pos += 1;
                            if it.next_if_eq(&'"').is_none() {
                                break;
                            }
                            pos += 1;
                        }
                        Some(c) => {
                            pos += 1;
                            line += (c == '\n') as usize;
                        }
                    }
                }
                quoted += 1;
            }
            let mut end = pos;
            let mut cr = false;
            while let Some(c) = it.next_if(|&c| c != delim && c != '\n') {
                pos += 1;
                end = pos;
                cr = c == '\r';
            }
            if cr && it.peek() == Some(&'\n') {
                end -= 1;
            }
            fields.push(Field { raw: start..end });
            match it.next() {
                Some('\n') => {
                    pos += 1;
                    line += 1;
                    break;
                }
                Some(_) => pos += 1,
                None => break,
            }
        }
        records.push(fields);
    }
    Parsed { records, quoted, unclosed: None }
}

/// Every record of `text`, split at `d`. An unterminated quote is an error naming the line its
/// record starts on.
pub fn records(text: &str, d: Delim) -> Result<Vec<Vec<Field>>, String> {
    let parsed = parse(text, d, usize::MAX);
    match parsed.unclosed {
        Some(line) => Err(format!("Unclosed quote in the record starting on line {line}")),
        None => Ok(parsed.records),
    }
}

/// A field's value: quotes taken off, `""` made `"`.
pub fn value(raw: &str) -> String {
    let Some(inner) = raw.strip_prefix('"') else {
        return raw.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if chars.next_if_eq(&'"').is_none() => {
                // what follows the closing quote stays as it is
                out.extend(chars);
                break;
            }
            c => out.push(c),
        }
    }
    out
}

struct Structure {
    common: usize,
    filled: usize,
    quoted: usize,
}

/// Record counts and valid quoted fields for one candidate separator.
fn structure(text: &str, d: Delim) -> Structure {
    let parsed = parse(text, d, DETECT_RECORDS);
    let quoted = if parsed.unclosed.is_none() { parsed.quoted } else { 0 };
    let mut counts: HashMap<usize, usize> = HashMap::new();
    let mut filled = 0;
    for r in parsed.records {
        if r.len() == 1 && r[0].raw.is_empty() {
            continue;
        }
        filled += 1;
        if r.len() > 1 {
            *counts.entry(r.len()).or_default() += 1;
        }
    }
    Structure { common: counts.values().copied().max().unwrap_or(0), filled, quoted }
}

/// Hide the chosen candidate's quoted contents from competing separator counts.
fn detection_text(text: &str, d: Delim) -> String {
    let mut out = String::new();
    let mut it = text.chars().peekable();
    let (mut field_start, mut records) = (true, 0);
    while let Some(c) = it.next() {
        out.push(c);
        if c == '"' && field_start {
            while let Some(c) = it.next() {
                if c == '"' {
                    if it.next_if_eq(&'"').is_some() {
                        continue;
                    }
                    out.push('"');
                    break;
                }
            }
            field_start = false;
        } else {
            if c == '\n' {
                records += 1;
                if records == DETECT_RECORDS {
                    break;
                }
            }
            field_start = c == '\n' || d.ch() == c;
        }
    }
    out
}

/// The delimiter of `text` (its first `DETECT_RECORDS` records are read, no more) and the file
/// extension it came from, if any. `.tsv` and `.tab` mean tab; `.csv` means comma when most
/// records agree on a comma field count, else detection decides. No structure at all is comma.
pub fn detect(text: &str, ext: Option<&str>) -> Delim {
    let ext = ext.map(|e| e.trim_start_matches('.').to_ascii_lowercase());
    if matches!(ext.as_deref(), Some("tsv" | "tab")) {
        return Delim::Tab;
    }
    let raw = ORDER.map(|d| (d, structure(text, d)));
    // Logical records matter here: quoted newlines must not give another separator more votes.
    let quoted_by = raw
        .iter()
        .max_by(|(_, a), (_, b)| {
            (a.common * b.filled.max(1)).cmp(&(b.common * a.filled.max(1))).then(a.quoted.cmp(&b.quoted))
        })
        .filter(|(_, s)| s.quoted > 0);
    let masked = quoted_by.map(|(d, _)| detection_text(text, *d));
    let text = masked.as_deref().unwrap_or(text);
    let scored = ORDER.map(|d| (d, structure(text, d)));
    if ext.as_deref() == Some("csv") {
        let (best, filled) =
            (scored.iter()).find(|(d, _)| *d == Delim::Comma).map_or((0, 0), |(_, s)| (s.common, s.filled));
        if best > 0 && best * 2 >= filled {
            return Delim::Comma;
        }
    }
    // the most common field count wins, the earlier in `ORDER` on a tie
    (scored.into_iter().rev())
        .filter(|(_, s)| s.common > 0)
        .max_by_key(|(_, s)| s.common)
        .map_or(Delim::Comma, |(d, _)| d)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnOp {
    Select,
    Copy,
    MoveLeft,
    MoveRight,
    Delete,
    Clear,
}

/// The column to work on and the records (indices) to work on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub col: usize,
    pub rows: Range<usize>,
}

fn start_of(r: &[Field]) -> usize {
    r[0].raw.start
}

fn end_of(r: &[Field]) -> usize {
    r[r.len() - 1].raw.end
}

/// The field holding `caret` gives the column. `scope`, a selection across lines, limits the
/// records to those it touches.
pub fn target(recs: &[Vec<Field>], caret: usize, scope: Option<Range<usize>>) -> Option<Target> {
    if recs.is_empty() {
        return None;
    }
    // the last record starting at or before the caret; past the final line break that is the
    // last record, whose last field then counts
    let rec = &recs[recs.partition_point(|r| start_of(r) <= caret).saturating_sub(1)];
    let col = (rec.iter().position(|f| caret <= f.raw.end)).unwrap_or(rec.len() - 1);
    let rows = match scope {
        None => 0..recs.len(),
        Some(s) => {
            let from = recs.partition_point(|r| end_of(r) < s.start);
            from..recs.partition_point(|r| start_of(r) < s.end).max(from)
        }
    };
    Some(Target { col, rows })
}

fn rows<'a>(recs: &'a [Vec<Field>], t: &Target) -> &'a [Vec<Field>] {
    &recs[t.rows.clone()]
}

pub fn select(recs: &[Vec<Field>], t: &Target) -> Vec<Range<usize>> {
    (rows(recs, t).iter()).filter_map(|r| r.get(t.col)).map(|f| f.raw.clone()).collect()
}

/// Field values, one a line; a record without the column gives an empty line.
pub fn copy(recs: &[Vec<Field>], t: &Target, slice: &dyn Fn(Range<usize>) -> String) -> String {
    let values: Vec<String> =
        (rows(recs, t).iter()).map(|r| r.get(t.col).map_or(String::new(), |f| value(&slice(f.raw.clone())))).collect();
    values.join("\n")
}

pub type Edits = Vec<(Range<usize>, String)>;

/// A trailing literal CR must be quoted before it can become the last field beside an LF.
fn quote_trailing_cr(raw: &str) -> Option<String> {
    raw.ends_with('\r').then(|| format!("\"{}\"", value(raw).replace('"', "\"\"")))
}

/// Fields `a` and `a + 1` of every record that has both, swapped without changing their values.
fn swap(recs: &[Vec<Field>], t: &Target, a: usize, d: Delim, slice: &dyn Fn(Range<usize>) -> String) -> Edits {
    (rows(recs, t).iter())
        .filter(|r| r.len() > a + 1)
        .map(|r| {
            let (x, y) = (&r[a].raw, &r[a + 1].raw);
            let left = slice(x.clone());
            let last = a + 2 == r.len();
            let left = (last.then(|| quote_trailing_cr(&left)).flatten()).unwrap_or(left);
            let swapped = format!("{}{}{}", slice(y.clone()), d.ch(), left);
            (x.start..y.end, swapped)
        })
        .collect()
}

pub fn move_left(recs: &[Vec<Field>], t: &Target, d: Delim, slice: &dyn Fn(Range<usize>) -> String) -> Edits {
    match t.col.checked_sub(1) {
        Some(a) => swap(recs, t, a, d, slice),
        None => Vec::new(),
    }
}

pub fn move_right(recs: &[Vec<Field>], t: &Target, d: Delim, slice: &dyn Fn(Range<usize>) -> String) -> Edits {
    swap(recs, t, t.col, d, slice)
}

/// The field and the delimiter after it, or before it for the last field; a lone field just
/// empties.
pub fn delete(recs: &[Vec<Field>], t: &Target, slice: &dyn Fn(Range<usize>) -> String) -> Edits {
    let k = t.col;
    (rows(recs, t).iter())
        .filter(|r| r.len() > k)
        .map(|r| {
            if k > 0 && k + 1 == r.len() {
                let previous = &r[k - 1].raw;
                if !previous.is_empty() && slice(previous.end - 1..previous.end) == "\r" {
                    if let Some(quoted) = quote_trailing_cr(&slice(previous.clone())) {
                        return (previous.start..r[k].raw.end, quoted);
                    }
                }
            }
            let range = match (k + 1 < r.len(), k > 0) {
                (true, _) => r[k].raw.start..r[k + 1].raw.start,
                (false, true) => r[k - 1].raw.end..r[k].raw.end,
                (false, false) => r[k].raw.clone(),
            };
            (range, String::new())
        })
        .collect()
}

pub fn clear(recs: &[Vec<Field>], t: &Target) -> Edits {
    (rows(recs, t).iter())
        .filter_map(|r| r.get(t.col))
        .filter(|f| !f.raw.is_empty())
        .map(|f| (f.raw.clone(), String::new()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str, r: Range<usize>) -> String {
        text.chars().skip(r.start).take(r.len()).collect()
    }

    /// The records as their raw field texts.
    fn raws(text: &str, d: Delim) -> Vec<Vec<String>> {
        records(text, d).unwrap().iter().map(|r| r.iter().map(|f| chars(text, f.raw.clone())).collect()).collect()
    }

    #[test]
    fn column_edits_preserve_a_literal_cr_when_its_cell_becomes_last() {
        for ending in ["\n", "\r\n", ""] {
            for (text, col, op, expected) in [
                ("a\r,b", 0, ColumnOp::MoveRight, vec!["b", "a\r"]),
                ("x,a\r,b", 2, ColumnOp::MoveLeft, vec!["x", "b", "a\r"]),
                ("a\r,b", 1, ColumnOp::Delete, vec!["a\r"]),
                ("a\"q\r,b", 0, ColumnOp::MoveRight, vec!["b", "a\"q\r"]),
                ("\"a\"\r,b", 1, ColumnOp::Delete, vec!["a\r"]),
            ] {
                let text = format!("{text}{ending}");
                let recs = records(&text, Delim::Comma).unwrap();
                let target = Target { col, rows: 0..1 };
                let slice = slicer(&text);
                let edits = match op {
                    ColumnOp::MoveRight => move_right(&recs, &target, Delim::Comma, &slice),
                    ColumnOp::MoveLeft => move_left(&recs, &target, Delim::Comma, &slice),
                    _ => delete(&recs, &target, &slice),
                };
                let changed = apply(&text, &edits);
                let values: Vec<_> = raws(&changed, Delim::Comma)[0].iter().map(|raw| value(raw)).collect();
                assert_eq!(values, expected, "{text:?}, {op:?} -> {changed:?}");
                assert!(changed.ends_with(ending));
            }
        }
    }

    fn apply(text: &str, edits: &Edits) -> String {
        let mut cs: Vec<char> = text.chars().collect();
        for (r, t) in edits.iter().rev() {
            cs.splice(r.clone(), t.chars());
        }
        cs.into_iter().collect()
    }

    /// A document with every awkward field: escapes, a multi-line field, empty fields, a short
    /// row and a blank line.
    const DOC: &str = "a,\"x,\"\"y\"\"\",c\n1,,3\nshort\n\"multi\nline\",2,3\n\nlast,,";

    fn at(text: &str, caret_after: &str) -> usize {
        text.find(caret_after).map(|b| text[..b].chars().count()).unwrap() + caret_after.chars().count()
    }

    fn target_at(text: &str, caret: usize) -> (Vec<Vec<Field>>, Target) {
        let recs = records(text, Delim::Comma).unwrap();
        let t = target(&recs, caret, None).unwrap();
        (recs, t)
    }

    fn slicer(text: &str) -> impl Fn(Range<usize>) -> String + '_ {
        move |r| chars(text, r)
    }

    #[test]
    fn records_hold_char_offsets_of_raw_fields() {
        assert_eq!(
            raws(DOC, Delim::Comma),
            [
                vec!["a", "\"x,\"\"y\"\"\"", "c"],
                vec!["1", "", "3"],
                vec!["short"],
                vec!["\"multi\nline\"", "2", "3"],
                vec![""],
                vec!["last", "", ""],
            ]
        );
        // offsets count chars, not bytes
        let text = "é,日\nb,c";
        assert_eq!(raws(text, Delim::Comma), [vec!["é", "日"], vec!["b", "c"]]);
        assert_eq!(records(text, Delim::Comma).unwrap()[1][1].raw, 6..7);
    }

    #[test]
    fn a_final_line_break_makes_no_record_and_a_blank_line_does() {
        assert_eq!(raws("a,b\n", Delim::Comma), [vec!["a", "b"]]);
        assert_eq!(raws("a\n\n", Delim::Comma), [vec!["a"], vec![""]]);
        assert!(records("", Delim::Comma).unwrap().is_empty());
    }

    #[test]
    fn a_quote_inside_an_unquoted_field_is_literal_and_text_after_a_closing_quote_stays() {
        assert_eq!(raws("ab\"c,d", Delim::Comma), [vec!["ab\"c", "d"]]);
        assert_eq!(raws("\"ab\"cd,e", Delim::Comma), [vec!["\"ab\"cd", "e"]]);
    }

    #[test]
    fn a_tab_in_a_quoted_comma_field_is_not_a_delimiter() {
        assert_eq!(raws("a,\"x\ty\",b", Delim::Comma), [vec!["a", "\"x\ty\"", "b"]]);
        assert_eq!(raws("a,\"x\ty\",b", Delim::Tab), [vec!["a,\"x", "y\",b"]]);
    }

    #[test]
    fn a_carriage_return_before_a_line_break_is_not_in_the_field() {
        assert_eq!(raws("a,b\r\nc,\"d\"\r\n", Delim::Comma), [vec!["a", "b"], vec!["c", "\"d\""]]);
    }

    #[test]
    fn an_unclosed_quote_names_the_line_its_record_starts_on() {
        let err = records("a,b\n1,\"two\nthree\n", Delim::Comma).unwrap_err();
        assert_eq!(err, "Unclosed quote in the record starting on line 2");
        let err = records("\"x", Delim::Comma).unwrap_err();
        assert_eq!(err, "Unclosed quote in the record starting on line 1");
        // after a multi-line record the count goes on
        let err = records("\"a\nb\"\nc,\"d", Delim::Comma).unwrap_err();
        assert_eq!(err, "Unclosed quote in the record starting on line 3");
    }

    #[test]
    fn values_lose_their_quotes_and_unescape() {
        assert_eq!(value("plain"), "plain");
        assert_eq!(value("\"x,\"\"y\"\"\""), "x,\"y\"");
        assert_eq!(value("\"multi\nline\""), "multi\nline");
        assert_eq!(value("\"\""), "");
        assert_eq!(value(""), "");
        assert_eq!(value("a\"b"), "a\"b");
        assert_eq!(value("\"ab\"cd"), "abcd");
    }

    #[test]
    fn detection_ignores_a_tab_inside_a_quoted_csv_field() {
        assert_eq!(detect("a,\"x\ty\",b\n1,2,3\n", None), Delim::Comma);
        assert_eq!(detect("a,\"x\ty\",b\n1,\"2\t3\",4\n", None), Delim::Comma);
        assert_eq!(detect("a;\"x,\"\"y\"\"\nz\";b\n1;\"2,3\";4\n", None), Delim::Semicolon);
    }

    #[test]
    fn detection_keeps_quotes_that_are_literal_for_the_stronger_candidate() {
        assert_eq!(detect("a,\"x;y\n1;2\n", None), Delim::Semicolon);
        assert_eq!(detect("a,\"x;y\"\n1;2\n", None), Delim::Semicolon);
        assert_eq!(detect("a,\"x;y;z\"\nb,\"u;v\";w,extra\n", None), Delim::Semicolon);
    }

    #[test]
    fn detection_ignores_competing_delimiters_across_quoted_line_breaks() {
        assert_eq!(detect("a;\"x,y\nz,w\";b\n1;\"2,3\n4,5\";6\n", None), Delim::Semicolon);
        assert_eq!(detect("a,\"x\ty\",b\n1,\"2\t3\",4,5\n", Some("csv")), Delim::Comma);
    }

    #[test]
    fn detection_picks_the_delimiter_most_records_agree_on() {
        assert_eq!(detect("a;b;c\n1;2;3\n", None), Delim::Semicolon);
        assert_eq!(detect("a|b\n1|2\n3|4\n", None), Delim::Pipe);
        assert_eq!(detect("a\tb\n1\t2\n", None), Delim::Tab);
        // commas in prose do not beat a consistent semicolon table
        assert_eq!(detect("x;y\n1;2\n3;4\n5;6\nnote, a comment\n", None), Delim::Semicolon);
        // ties go to tab, comma, semicolon, pipe
        assert_eq!(detect("a\tb,c\n", None), Delim::Tab);
        assert_eq!(detect("a,b;c\n", None), Delim::Comma);
        assert_eq!(detect("a;b|c\n", None), Delim::Semicolon);
    }

    #[test]
    fn a_document_with_no_structure_is_comma() {
        assert_eq!(detect("just\nwords\n", None), Delim::Comma);
        assert_eq!(detect("", None), Delim::Comma);
    }

    #[test]
    fn the_extension_decides_unless_a_csv_has_no_comma_structure() {
        assert_eq!(detect("a,b\n1,2\n", Some("tsv")), Delim::Tab);
        assert_eq!(detect("a,b\n1,2\n", Some("TAB")), Delim::Tab);
        assert_eq!(detect("a;b\n1;2\n", Some("csv")), Delim::Semicolon);
        assert_eq!(detect("a\tb\n1\t2\n", Some("csv")), Delim::Tab);
        assert_eq!(detect("a,b\n1,2\n", Some("csv")), Delim::Comma);
        assert_eq!(detect("one\ntwo\n", Some("csv")), Delim::Comma);
        // another extension is no hint
        assert_eq!(detect("a;b\n1;2\n", Some("txt")), Delim::Semicolon);
    }

    #[test]
    fn detection_reads_only_the_first_records() {
        let mut text = "a,b,c\n".repeat(DETECT_RECORDS);
        text.push_str(&"x|y|z\n".repeat(DETECT_RECORDS * 3));
        assert_eq!(detect(&text, None), Delim::Comma);
    }

    #[test]
    fn select_takes_the_raw_field_of_every_record_that_has_it() {
        let (recs, t) = target_at(DOC, at(DOC, "a,\"x"));
        assert_eq!(t.col, 1);
        let got: Vec<String> = select(&recs, &t).into_iter().map(|r| chars(DOC, r)).collect();
        // the empty fields of "1,,3" and "last,," are carets; "short" has no such column
        assert_eq!(got, ["\"x,\"\"y\"\"\"", "", "2", ""]);
        let first = select(&recs, &Target { col: 0, ..t });
        assert_eq!(chars(DOC, first[3].clone()), "\"multi\nline\"");
    }

    #[test]
    fn copy_gives_unescaped_values_one_a_line() {
        let (recs, t) = target_at(DOC, at(DOC, "a,\"x"));
        assert_eq!(copy(&recs, &t, &slicer(DOC)), "x,\"y\"\n\n\n2\n\n");
        let first = Target { col: 0, ..t };
        assert_eq!(copy(&recs, &first, &slicer(DOC)), "a\n1\nshort\nmulti\nline\n\nlast");
    }

    #[test]
    fn the_caret_picks_the_field_even_at_its_edges_and_inside_a_multi_line_record() {
        for (after, col) in
            [("a", 0), ("a,", 1), ("a,\"x,\"\"y\"\"\"", 1), ("a,\"x,\"\"y\"\"\",", 2), ("\"multi\nline\",", 1)]
        {
            let (_, t) = target_at(DOC, at(DOC, after));
            assert_eq!(t.col, col, "{after:?}");
        }
        // inside the second line of a multi-line field
        let (recs, t) = target_at(DOC, at(DOC, "\"multi\nli"));
        assert_eq!((t.col, t.rows.clone()), (0, 0..recs.len()));
    }

    #[test]
    fn a_selection_across_lines_limits_the_records() {
        let recs = records(DOC, Delim::Comma).unwrap();
        // from inside record 1 to inside record 2
        let scope = at(DOC, "1,")..at(DOC, "shor");
        let t = target(&recs, at(DOC, "1,"), Some(scope)).unwrap();
        assert_eq!(t.rows, 1..3);
        // a whole line selected with its break touches that line only
        let line = 0..at(DOC, "c\n");
        let t = target(&recs, 0, Some(line)).unwrap();
        assert_eq!(t.rows, 0..1);
    }

    #[test]
    fn move_right_and_left_swap_raw_fields_where_both_exist() {
        let (recs, t) = target_at(DOC, at(DOC, "a,\"x"));
        let right = move_right(&recs, &t, Delim::Comma, &slicer(DOC));
        assert_eq!(apply(DOC, &right), "a,c,\"x,\"\"y\"\"\"\n1,3,\nshort\n\"multi\nline\",3,2\n\nlast,,");
        let left = move_left(&recs, &t, Delim::Comma, &slicer(DOC));
        assert_eq!(apply(DOC, &left), "\"x,\"\"y\"\"\",a,c\n,1,3\nshort\n2,\"multi\nline\",3\n\n,last,");
        // the first column has nothing to its left; the last nothing to its right
        let mut first = t.clone();
        first.col = 0;
        assert!(move_left(&recs, &first, Delim::Comma, &slicer(DOC)).is_empty());
        let last = Target { col: 2, ..t };
        assert!(move_right(&recs, &last, Delim::Comma, &slicer(DOC)).is_empty());
    }

    #[test]
    fn delete_removes_the_field_and_one_delimiter() {
        let (recs, t) = target_at(DOC, at(DOC, "a,\"x"));
        assert_eq!(apply(DOC, &delete(&recs, &t, &slicer(DOC))), "a,c\n1,3\nshort\n\"multi\nline\",3\n\nlast,");
        // the last column takes the delimiter before it; a lone field just empties
        let mut last = t.clone();
        last.col = 2;
        assert_eq!(
            apply(DOC, &delete(&recs, &last, &slicer(DOC))),
            "a,\"x,\"\"y\"\"\"\n1,\nshort\n\"multi\nline\",2\n\nlast,"
        );
        let first = Target { col: 0, ..t };
        let got = apply(DOC, &delete(&recs, &first, &slicer(DOC)));
        assert_eq!(got, "\"x,\"\"y\"\"\",c\n,3\n\n2,3\n\n,");
    }

    #[test]
    fn clear_empties_the_field_without_quotes() {
        let (recs, t) = target_at(DOC, at(DOC, "a,\"x"));
        assert_eq!(apply(DOC, &clear(&recs, &t)), "a,,c\n1,,3\nshort\n\"multi\nline\",,3\n\nlast,,");
        let first = Target { col: 0, ..t };
        assert_eq!(apply(DOC, &clear(&recs, &first)), ",\"x,\"\"y\"\"\",c\n,,3\n\n,2,3\n\n,,");
    }

    /// Random text from the characters that mean something to the parser.
    fn scramble(state: &mut u64, len: usize) -> String {
        const PARTS: [&str; 9] = ["a", ",", "\"", "\n", "\r\n", "é", " ", "\"\"", "日"];
        (0..len)
            .map(|_| {
                *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                PARTS[(*state >> 33) as usize % PARTS.len()]
            })
            .collect()
    }

    #[test]
    fn records_tile_the_text_and_column_edits_keep_it_parseable() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..4000 {
            let len = 1 + (state >> 40) as usize % 24;
            let text = scramble(&mut state, len);
            let Ok(recs) = records(&text, Delim::Comma) else {
                continue;
            };
            let total = text.chars().count();
            // fields are in order, one delimiter apart, and records one line break apart
            let mut at = 0;
            for (i, rec) in recs.iter().enumerate() {
                for (j, f) in rec.iter().enumerate() {
                    assert_eq!(f.raw.start, at, "{text:?} record {i} field {j}");
                    assert!(f.raw.start <= f.raw.end && f.raw.end <= total, "{text:?}");
                    at = f.raw.end;
                    if j + 1 < rec.len() {
                        assert_eq!(chars(&text, at..at + 1), ",", "{text:?}");
                        at += 1;
                    }
                }
                let gap = chars(&text, at..(at + 2).min(total));
                at += if gap.starts_with("\r\n") {
                    2
                } else if gap.starts_with('\n') {
                    1
                } else {
                    0
                };
            }
            assert_eq!(at, total, "{text:?}");

            // moving a column right and then back is the text again; delete and clear keep the
            // record count and the other columns' text
            let width = recs.iter().map(Vec::len).max().unwrap();
            for col in 0..width {
                let t = Target { col, rows: 0..recs.len() };
                let slice = slicer(&text);
                let moved = apply(&text, &move_right(&recs, &t, Delim::Comma, &slice));
                let again = records(&moved, Delim::Comma).unwrap();
                assert_eq!(again.len(), recs.len(), "{text:?} -> {moved:?}");
                let mut back = t.clone();
                back.col += 1;
                let slice = slicer(&moved);
                let restored = apply(&moved, &move_left(&again, &back, Delim::Comma, &slice));
                assert_eq!(restored, text, "column {col} of {text:?}");

                for edits in [delete(&recs, &t, &slicer(&text)), clear(&recs, &t)] {
                    let edited = apply(&text, &edits);
                    let after = records(&edited, Delim::Comma)
                        .unwrap_or_else(|e| panic!("{text:?} col {col} -> {edited:?}: {e}"));
                    // a last record emptied to nothing is the line break before it, ending the text
                    let ended = after.len() + 1 == recs.len() && (edited.is_empty() || edited.ends_with('\n'));
                    assert!(after.len() == recs.len() || ended, "{text:?} col {col} -> {edited:?}");
                }
            }
        }
    }

    #[test]
    fn a_caret_past_the_last_record_works_on_its_last_field() {
        let text = "a,b\n1,2\n";
        let recs = records(text, Delim::Comma).unwrap();
        let t = target(&recs, text.chars().count(), None).unwrap();
        assert_eq!(t.col, 1);
        assert!(target(&[], 0, None).is_none());
    }
}
