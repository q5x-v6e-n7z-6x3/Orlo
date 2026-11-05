//! The text transformations, pure functions over `&str` shared by the Edit > Transform
//! commands and pipeline steps. Line-based ones keep a final line break as it was.

use crate::buffer::FindOpts;
use crate::search::pattern::Pattern;
use regex_automata::meta;
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::HashMap;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
use unicode_segmentation::UnicodeSegmentation;

/// The lines of `text`, and whether it ends with a line break (which starts no further line).
pub(crate) fn split(text: &str) -> (Vec<&str>, bool) {
    if text.is_empty() {
        return (Vec::new(), false);
    }
    let (body, ends) = match text.strip_suffix('\n') {
        Some(body) => (body, true),
        None => (text, false),
    };
    (body.split('\n').collect(), ends)
}

pub(crate) fn join<S: AsRef<str>>(lines: &[S], ends: bool) -> String {
    let mut out = (lines.iter()).map(|l| l.as_ref()).collect::<Vec<_>>().join("\n");
    if ends && !lines.is_empty() {
        out.push('\n');
    }
    out
}

/// Each line followed by its line break, so a blank line is still a line (`"\n"`) and no lines
/// at all is `""`.
fn terminated(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

/// `s` lowercased when case is ignored, so that keys compare.
fn cased(s: &str, case_insensitive: bool) -> String {
    match case_insensitive {
        true => s.to_lowercase(),
        false => s.to_string(),
    }
}

fn map_lines(text: &str, f: impl Fn(&str) -> String) -> String {
    let (lines, ends) = split(text);
    join(&lines.into_iter().map(f).collect::<Vec<_>>(), ends)
}

/// A pattern as the Find panel reads it, lookaround and backreferences included;
/// `literal` text is escaped first.
fn compile(pattern: &str, literal: bool, case_insensitive: bool) -> Result<Pattern, String> {
    if pattern.is_empty() {
        return Err("Enter a pattern".into());
    }
    let find = FindOpts { regex: true, case_sensitive: !case_insensitive, ..FindOpts::default() };
    match literal {
        true => Pattern::new(&regex_syntax::escape(pattern), &find),
        false => Pattern::new(pattern, &find),
    }
}

/// A line's key: capture group 1 if the pattern has one, else the whole match.
fn key<'a>(re: &Pattern, line: &'a str) -> Result<Option<&'a str>, String> {
    capture_key(re, line, usize::from(re.group_count() != 1))
}

fn capture_key<'a>(re: &Pattern, line: &'a str, group: usize) -> Result<Option<&'a str>, String> {
    let groups = (re.captures_from(line, 0..line.len(), &|| false)).map_err(|e| e.message())?;
    let Some(groups) = groups else {
        return Ok(None);
    };
    // A valid group that took no part in the match retains the legacy empty key.
    let at = groups.get(group).cloned().flatten();
    Ok(Some(at.map_or("", |r| &line[r])))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Lexical,
    Natural,
    Locale,
    Length,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SortOpts {
    /// The key is capture group 1 if there is one, else the whole match.
    pub pattern: String,
    /// Explicit capture index; 0 selects the full match, None keeps the legacy key.
    pub capture: Option<usize>,
    pub order: SortOrder,
    /// Keys compare ignoring case; the pattern itself is read as typed.
    pub case_insensitive: bool,
    /// Legacy finite f64 order, including lexical fallback for nonnumbers.
    pub numeric: bool,
    pub reverse: bool,
    pub keep_header: bool,
}

/// Stable key order in either direction; unmatched lines always stay last in source order.
/// The first line can remain a header. The caller supplies each selected line block separately.
pub fn sort_by_pattern(text: &str, o: &SortOpts) -> Result<String, String> {
    let re = compile(&o.pattern, false, false)?;
    let group = o.capture.unwrap_or_else(|| usize::from(re.group_count() != 1));
    if group >= re.group_count() {
        return Err(format!("Invalid capture index {group}; use 0 through {}", re.group_count() - 1));
    }
    if o.numeric && o.order != SortOrder::Lexical {
        return Err("Numeric cannot be combined with a nonlexical order".into());
    }
    #[cfg(target_os = "macos")]
    let locale = (o.order == SortOrder::Locale).then(orlo_utils::collation::Collator::current).transpose()?;
    #[cfg(not(target_os = "macos"))]
    if o.order == SortOrder::Locale {
        return Err("Locale sorting requires macOS".into());
    }
    let (lines, ends) = split(text);
    let mut lines = lines.into_iter();
    let header = o.keep_header.then(|| lines.next()).flatten();
    // Compute numeric/text/length/native keys once, rather than inside the comparator.
    let (mut keyed, mut unmatched) = (Vec::new(), Vec::new());
    for line in lines {
        let Some(k) = capture_key(&re, line, group)? else {
            unmatched.push(line);
            continue;
        };
        let number = o.numeric.then(|| k.trim().parse::<f64>().ok()).flatten().filter(|n| n.is_finite());
        let text = cased(k, o.case_insensitive);
        let length = if o.order == SortOrder::Length { k.graphemes(true).count() } else { 0 };
        #[cfg(target_os = "macos")]
        let native = locale.as_ref().map(|_| orlo_utils::collation::Key::new(k)).transpose()?;
        keyed.push((
            number,
            text,
            length,
            line,
            #[cfg(target_os = "macos")]
            native,
        ));
    }
    keyed.sort_by(|a, b| {
        let order = if o.numeric {
            match (a.0, b.0) {
                (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => a.1.cmp(&b.1),
            }
        } else {
            match o.order {
                SortOrder::Lexical => a.1.cmp(&b.1),
                SortOrder::Natural => orlo_utils::text::natural_cmp(a.1.as_bytes(), b.1.as_bytes()),
                SortOrder::Length => a.2.cmp(&b.2),
                SortOrder::Locale => {
                    #[cfg(target_os = "macos")]
                    {
                        locale.as_ref().unwrap().compare(
                            a.4.as_ref().unwrap(),
                            b.4.as_ref().unwrap(),
                            o.case_insensitive,
                        )
                    }
                    #[cfg(not(target_os = "macos"))]
                    unreachable!("locale was refused before collecting keys")
                }
            }
        };
        if o.reverse {
            order.reverse()
        } else {
            order
        }
    });
    let sorted: Vec<&str> = header.into_iter().chain(keyed.into_iter().map(|k| k.3)).chain(unmatched).collect();
    Ok(join(&sorted, ends))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compare {
    #[default]
    Exact,
    IgnoreCase,
    IgnoreSpace,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DupAction {
    #[default]
    KeepFirst,
    KeepLast,
    DeleteAll,
    KeepOnlyDuplicated,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DupOpts {
    /// Empty compares whole lines; else the key is capture group 1 or the whole match, and a
    /// line the pattern does not match is never a duplicate.
    pub pattern: String,
    pub compare: Compare,
    pub action: DupAction,
}

/// The text left, and the lines taken out of it, each ending in a line break.
pub fn process_duplicates(text: &str, o: &DupOpts) -> Result<(String, String), String> {
    let re = match o.pattern.is_empty() {
        true => None,
        false => Some(compile(&o.pattern, false, false)?),
    };
    let (lines, ends) = split(text);
    let mut keys: Vec<Option<String>> = Vec::with_capacity(lines.len());
    for &line in &lines {
        let k = match &re {
            Some(re) => key(re, line)?,
            None => Some(line),
        };
        keys.push(k.map(|k| match o.compare {
            Compare::Exact => k.to_string(),
            Compare::IgnoreCase => k.to_lowercase(),
            Compare::IgnoreSpace => k.trim().to_string(),
        }));
    }
    // per key: first line, last line, how many
    let mut seen: HashMap<&str, (usize, usize, usize)> = HashMap::new();
    for (i, k) in keys.iter().enumerate() {
        if let Some(k) = k {
            let e = seen.entry(k).or_insert((i, i, 0));
            (e.1, e.2) = (i, e.2 + 1);
        }
    }
    let (mut kept, mut removed) = (Vec::new(), Vec::new());
    for (i, line) in lines.into_iter().enumerate() {
        let keep = match keys[i].as_deref().map(|k| seen[k]) {
            None => o.action != DupAction::KeepOnlyDuplicated,
            Some((first, last, n)) => match o.action {
                DupAction::KeepFirst => first == i,
                DupAction::KeepLast => last == i,
                DupAction::DeleteAll => n == 1,
                DupAction::KeepOnlyDuplicated => n > 1 && first == i,
            },
        };
        match keep {
            true => kept.push(line),
            false => removed.push(line),
        }
    }
    Ok((join(&kept, ends), terminated(&removed)))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchAction {
    #[default]
    Keep,
    Delete,
    Copy,
    Extract,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MatchOpts {
    pub pattern: String,
    /// Else the pattern is literal text.
    pub regex: bool,
    pub case_insensitive: bool,
    /// Act on the lines that do not match.
    pub invert: bool,
    pub action: MatchAction,
}

/// The text the document keeps after the action (all of it for Copy and Extract), and the
/// lines the pattern selected, each ending in a line break.
pub fn process_matching(text: &str, o: &MatchOpts) -> Result<(String, String), String> {
    let re = compile(&o.pattern, !o.regex, o.case_insensitive)?;
    let (lines, ends) = split(text);
    let (mut hit, mut rest): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for line in lines {
        match re.is_match(line).map_err(|e| e.message())? != o.invert {
            true => hit.push(line),
            false => rest.push(line),
        }
    }
    let kept = match o.action {
        MatchAction::Keep => join(&hit, ends),
        MatchAction::Delete => join(&rest, ends),
        MatchAction::Copy | MatchAction::Extract => text.to_string(),
    };
    Ok((kept, terminated(&hit)))
}

fn blank(line: &str) -> bool {
    line.trim().is_empty()
}

pub fn prefix(text: &str, p: &str, skip_blank: bool) -> String {
    map_lines(text, |l| match skip_blank && blank(l) {
        true => l.to_string(),
        false => format!("{p}{l}"),
    })
}

pub fn suffix(text: &str, s: &str, skip_blank: bool) -> String {
    map_lines(text, |l| match skip_blank && blank(l) {
        true => l.to_string(),
        false => format!("{l}{s}"),
    })
}

/// Only lines that start with `p` change, and by one `p`.
pub fn remove_prefix(text: &str, p: &str) -> String {
    map_lines(text, |l| l.strip_prefix(p).unwrap_or(l).to_string())
}

pub fn remove_suffix(text: &str, s: &str) -> String {
    map_lines(text, |l| l.strip_suffix(s).unwrap_or(l).to_string())
}

pub fn straighten_quotes(text: &str) -> String {
    (text.chars())
        .map(|c| match c {
            '‘' | '’' | '‚' | '‛' | '′' => '\'',
            '“' | '”' | '„' | '‟' | '″' => '"',
            c => c,
        })
        .collect()
}

/// A quote opens after the start of the text, white space or `( [ { < —`, or straight after an
/// opening quote of the other kind (`"'hi'"`), else it closes; a `'` between letters is an
/// apostrophe. (A selection's own start counts as the text's.)
pub fn smart_quotes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut before: Option<char> = None;
    while let Some(c) = chars.next() {
        let opens = before.is_none_or(|p| p.is_whitespace() || "([{<—".contains(p))
            || matches!((before, c), (Some('“'), '\'') | (Some('‘'), '"'));
        let apostrophe = before.is_some_and(char::is_alphabetic) && chars.peek().is_some_and(|n| n.is_alphabetic());
        let written = match (c, opens) {
            ('"', true) => '“',
            ('"', false) => '”',
            ('\'', _) if apostrophe => '’',
            ('\'', true) => '‘',
            ('\'', false) => '’',
            _ => c,
        };
        out.push(written);
        before = Some(written);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NormForm {
    Nfc,
    Nfd,
    Nfkc,
    Nfkd,
}

pub fn normalize(text: &str, form: NormForm) -> String {
    match form {
        NormForm::Nfc => text.nfc().collect(),
        NormForm::Nfd => text.nfd().collect(),
        NormForm::Nfkc => text.nfkc().collect(),
        NormForm::Nfkd => text.nfkd().collect(),
    }
}

/// NFD, the combining marks dropped, then NFC.
pub fn strip_diacritics(text: &str) -> String {
    text.nfd().filter(|c| !is_combining_mark(*c)).nfc().collect()
}

/// The transformations with no options, which Edit > Transform runs on any selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOp {
    StraightenQuotes,
    SmartQuotes,
    Normalize(NormForm),
    StripDiacritics,
}

impl TextOp {
    pub fn apply(self, text: &str) -> String {
        match self {
            TextOp::StraightenQuotes => straighten_quotes(text),
            TextOp::SmartQuotes => smart_quotes(text),
            TextOp::Normalize(form) => normalize(text, form),
            TextOp::StripDiacritics => strip_diacritics(text),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
pub struct TableOpts {
    /// Rows are regexes applied in order, each a full pass; else literal, in one pass.
    pub regex: bool,
    pub case_insensitive: bool,
    pub whole_word: bool,
}

/// A validated substitution table, ready to apply.
#[derive(Debug)]
pub struct Table(Rows);

#[derive(Debug)]
enum Rows {
    /// One regex of every find, longest first; a match's pattern number picks its replacement.
    Literal(meta::Regex, Vec<String>),
    Regex(Vec<(meta::Regex, String)>),
}

impl Table {
    /// Rows are `find<TAB>replace`; `#` lines and blank lines are skipped. Every problem comes
    /// back, each naming its line.
    pub fn parse(text: &str, o: &TableOpts) -> Result<Table, Vec<String>> {
        let rows = text.lines().enumerate().filter_map(|(i, line)| {
            let n = i + 1;
            // a row of white space and a tab is a real find
            if line.starts_with('#') || (blank(line) && !line.contains('\t')) {
                return None;
            }
            let mut cols = line.split('\t');
            Some((
                n,
                match (cols.next(), cols.next(), cols.next()) {
                    (Some(find), Some(replace), None) => Ok((find, replace)),
                    _ => Err(format!("Line {n}: a row is the find text, one tab, the replacement")),
                },
            ))
        });
        Self::build(rows, o)
    }

    /// A table of rows given as they are (no tabs or `#` to mind); a problem names its row
    /// as a "Line".
    pub fn from_rows(rows: &[(String, String)], o: &TableOpts) -> Result<Table, Vec<String>> {
        let rows = rows.iter().enumerate();
        let rows = rows.map(|(i, (find, replace))| (i + 1, Ok((find.as_str(), replace.as_str()))));
        Self::build(rows, o)
    }

    /// The rows (line number, find and replace or what is wrong with the row) checked and compiled.
    fn build<'a>(
        rows: impl Iterator<Item = (usize, Result<(&'a str, &'a str), String>)>,
        o: &TableOpts,
    ) -> Result<Table, Vec<String>> {
        let find_opts =
            FindOpts { regex: o.regex, case_sensitive: !o.case_insensitive, whole_word: o.whole_word, wrap: true };
        let engine = FindOpts::engine();
        let (mut literal, mut regex, mut errors) = (Vec::new(), Vec::new(), Vec::new());
        let mut first_at: HashMap<String, usize> = HashMap::new();
        for (n, row) in rows {
            let Ok((find, replace)) = row.map_err(|e| errors.push(e)) else {
                continue;
            };
            if find.is_empty() {
                errors.push(format!("Line {n}: the find text is empty"));
                continue;
            }
            let same = cased(find, o.case_insensitive);
            if let Some(first) = first_at.get(&same) {
                errors.push(format!("Line {n}: \"{find}\" is already a find on line {first}"));
                continue;
            }
            first_at.insert(same, n);
            let query = if o.regex { find.to_string() } else { regex_syntax::escape(find) };
            let hir = find_opts.hir(&query);
            let Ok(hir) = hir.map_err(|e| errors.push(format!("Line {n}: invalid regex: {e}"))) else {
                continue;
            };
            if !o.regex {
                literal.push((find, hir, replace));
                continue;
            }
            // built here, not after the loop: a failure (a pattern over the size limit) names its line
            match engine.build_from_hir(&hir) {
                Ok(re) => regex.push((re, replace.to_string())),
                Err(e) => errors.push(format!("Line {n}: invalid regex: {e}")),
            }
        }
        if errors.is_empty() && literal.is_empty() && regex.is_empty() {
            errors.push("The table has no rows".into());
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let rows = if o.regex {
            Rows::Regex(regex)
        } else {
            // the engine prefers the earlier pattern at one position: longest find first
            literal.sort_by_key(|(find, ..)| std::cmp::Reverse(find.chars().count()));
            let hirs: Vec<_> = literal.iter().map(|(_, hir, _)| hir).collect();
            // the finds are built together, so one too big for the engine together is
            // reported without a line; upgrade if a real table gets near the 10 MB limit
            let re =
                engine.build_many_from_hir(&hirs).map_err(|e| vec![format!("The table is too big to compile: {e}")])?;
            Rows::Literal(re, literal.iter().map(|r| r.2.to_string()).collect())
        };
        Ok(Table(rows))
    }

    pub fn apply(&self, text: &str) -> String {
        match &self.0 {
            Rows::Literal(re, replacements) => {
                let (mut out, mut last) = (String::new(), 0);
                for m in re.find_iter(text) {
                    out.push_str(&text[last..m.start()]);
                    out.push_str(&replacements[m.pattern().as_usize()]);
                    last = m.end();
                }
                out.push_str(&text[last..]);
                out
            }
            Rows::Regex(rows) => (rows.iter()).fold(text.to_string(), |t, (re, replace)| {
                let (mut out, mut last) = (String::new(), 0);
                for caps in re.captures_iter(&t) {
                    let Some(m) = caps.get_match() else { continue };
                    out.push_str(&t[last..m.start()]);
                    caps.interpolate_string_into(&t, replace, &mut out);
                    last = m.end();
                }
                out.push_str(&t[last..]);
                out
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sort(text: &str, pattern: &str, ci: bool, numeric: bool, reverse: bool) -> String {
        let o = SortOpts { pattern: pattern.into(), case_insensitive: ci, numeric, reverse, ..SortOpts::default() };
        sort_by_pattern(text, &o).unwrap()
    }

    #[test]
    fn sort_by_pattern_keys_on_the_capture_or_the_match() {
        let text = "b 2\na 10\nc 1\nx";
        // numeric: 1 < 2 < 10, and the line without a key goes last
        assert_eq!(sort(text, r"(\d+)", false, true, false), "c 1\nb 2\na 10\nx");
        // as text "10" < "2"; no group: the whole match is the key
        assert_eq!(sort(text, r"\d+", false, false, false), "c 1\na 10\nb 2\nx");
        // reverse still leaves the unmatched line last
        assert_eq!(sort(text, r"(\d+)", false, true, true), "a 10\nb 2\nc 1\nx");
        // keys that are not numbers follow the numbers, among themselves by text
        assert_eq!(sort("k: z\nk: 5\nk: abc\nk: 2", r"k: (\w+)", false, true, false), "k: 2\nk: 5\nk: abc\nk: z");
        assert_eq!(
            sort("k: nan\nk: 3", r"k: (\w+)", false, true, false),
            "k: 3\nk: nan",
            "nan is a word, not a number"
        );
        // ignoring case, A2 and a3 tie and keep their order (stable)
        assert_eq!(sort("b1\nA2\na3\nB4", r"(\w)", true, false, false), "A2\na3\nb1\nB4");
        assert_eq!(sort("b1\nA2\na3\nB4", r"(\w)", false, false, false), "A2\nB4\na3\nb1");
        // a final line break stays
        assert_eq!(sort("b 2\na 1\n", r"(\d)", false, true, false), "a 1\nb 2\n");
    }

    #[test]
    fn sort_by_pattern_refuses_a_bad_pattern() {
        let o = |p: &str| SortOpts { pattern: p.into(), ..SortOpts::default() };
        assert!(sort_by_pattern("a", &o("(")).unwrap_err().contains("regex"));
        assert!(sort_by_pattern("a", &o("")).is_err());
    }

    fn n30_sort(text: &str, options: serde_json::Value) -> Result<String, String> {
        let options: SortOpts = serde_json::from_value(options).unwrap();
        sort_by_pattern(text, &options)
    }

    #[test]
    fn n30_natural_sort_compares_all_integer_chunks_and_delays_zero_ties() {
        let options = serde_json::json!({"pattern":".*", "order":"natural", "case_insensitive":true});
        assert_eq!(
            n30_sort("v2-part10\nv2-part9\nv02-part9\nV2-part9\nv2-part0009", options.clone()).unwrap(),
            "v2-part9\nV2-part9\nv2-part0009\nv02-part9\nv2-part10"
        );
        assert_eq!(
            n30_sort("v999999999999999999999999999999\nv1000000000000000000000000000000\nv9", options).unwrap(),
            "v9\nv999999999999999999999999999999\nv1000000000000000000000000000000"
        );
        assert_eq!(
            n30_sort("a2\nA2\na10", serde_json::json!({"pattern":".*", "order":"natural"})).unwrap(),
            "A2\na2\na10"
        );
    }

    #[test]
    fn n30_length_counts_grapheme_clusters_and_keeps_reverse_ties_stable() {
        let text = "bb\ne\u{301}\n👩‍💻\n🇨🇦\na\n👨‍👩‍👧‍👦\nccc";
        let mut options = serde_json::json!({"pattern":".*", "order":"length"});
        assert_eq!(n30_sort(text, options.clone()).unwrap(), "e\u{301}\n👩‍💻\n🇨🇦\na\n👨‍👩‍👧‍👦\nbb\nccc");
        options["reverse"] = true.into();
        assert_eq!(n30_sort(text, options).unwrap(), "ccc\nbb\ne\u{301}\n👩‍💻\n🇨🇦\na\n👨‍👩‍👧‍👦");
    }

    #[test]
    fn n30_capture_header_reverse_and_unmatched_use_source_order_for_ties() {
        let text = "header:0\nb:2\nmissing1\na:10\nc:2\nmissing2\n";
        let options = serde_json::json!({
            "pattern":r"^([^:]+):(\d+)$", "capture":2, "numeric":true, "keep_header":true, "reverse":true
        });
        assert_eq!(n30_sort(text, options).unwrap(), "header:0\na:10\nb:2\nc:2\nmissing1\nmissing2\n");
        let options = serde_json::json!({"pattern":r"([^:]+):(\d+)", "capture":0});
        assert_eq!(n30_sort("b:1\na:9", options).unwrap(), "a:9\nb:1");
        let options = serde_json::json!({"pattern":r"^(a)?b", "capture":1});
        assert_eq!(n30_sort("ab\nb\nz", options).unwrap(), "b\nab\nz", "nonparticipating valid group is an empty key");
        let options = serde_json::json!({"pattern":".*", "keep_header":true});
        assert_eq!(n30_sort("only", options.clone()).unwrap(), "only");
        assert_eq!(n30_sort("", options).unwrap(), "");
    }

    #[test]
    fn n30_rejects_invalid_capture_and_ambiguous_modes_before_reading_lines() {
        for text in ["", "a\nb"] {
            let options = serde_json::json!({"pattern":"(a)", "capture":2});
            assert!(n30_sort(text, options).unwrap_err().contains("capture"));
            let options = serde_json::json!({"pattern":".*", "numeric":true, "order":"natural"});
            assert!(n30_sort(text, options).unwrap_err().contains("Numeric"));
        }
        assert!(serde_json::from_value::<SortOpts>(serde_json::json!({"capture":-1})).is_err());
    }

    #[test]
    fn n30_legacy_f64_numbers_keep_fraction_exponent_fallback_and_reverse_ties() {
        assert_eq!(
            sort("x:1e2\ny:-2.5\nz: 3\nw:NaN\nq:inf\na:2.5", r"^.: ?(.+)$", false, true, false),
            "y:-2.5\na:2.5\nz: 3\nx:1e2\nw:NaN\nq:inf"
        );
        assert_eq!(sort("x:2.0\ny:2\nz:+2", r":(.+)$", false, true, true), "x:2.0\ny:2\nz:+2");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn n30_locale_sort_uses_canonical_equivalence_and_full_unicode_keys() {
        // No region-specific accent-order assumption: canonical spellings tie in every locale.
        let options = serde_json::json!({"pattern":".*", "order":"locale"});
        assert_eq!(n30_sort("é\ne\u{301}", options.clone()).unwrap(), "é\ne\u{301}");
        assert_eq!(n30_sort("e\u{301}\né", options.clone()).unwrap(), "e\u{301}\né");
        assert_eq!(n30_sort("z\na\nm", options.clone()).unwrap(), "a\nm\nz");
        assert_eq!(n30_sort("a\0z\na\0b", options).unwrap(), "a\0b\na\0z");
    }

    fn dups(text: &str, pattern: &str, compare: Compare, action: DupAction) -> (String, String) {
        let o = DupOpts { pattern: pattern.into(), compare, action };
        process_duplicates(text, &o).unwrap()
    }

    #[test]
    fn each_duplicate_action() {
        let t = "a\nb\na\nc\nb";
        let run = |action| dups(t, "", Compare::Exact, action);
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(run(DupAction::KeepFirst), s("a\nb\nc", "a\nb\n"));
        assert_eq!(run(DupAction::KeepLast), s("a\nc\nb", "a\nb\n"));
        assert_eq!(run(DupAction::DeleteAll), s("c", "a\nb\na\nb\n"));
        assert_eq!(run(DupAction::KeepOnlyDuplicated), s("a\nb", "a\nc\nb\n"));
        // a final line break stays
        assert_eq!(dups("a\na\n", "", Compare::Exact, DupAction::KeepFirst).0, "a\n");
    }

    #[test]
    fn duplicates_compare_by_case_space_or_key() {
        let first = DupAction::KeepFirst;
        assert_eq!(dups("a\nA", "", Compare::Exact, first).0, "a\nA");
        assert_eq!(dups("a\nA", "", Compare::IgnoreCase, first).0, "a");
        assert_eq!(dups(" a\na \nb", "", Compare::Exact, first).0, " a\na \nb");
        assert_eq!(dups(" a\na \nb", "", Compare::IgnoreSpace, first).0, " a\nb");
        // the key is capture 1; a line without one is never a duplicate
        let t = "id1 x\nid2 y\nno key\nid1 z\nno key";
        assert_eq!(
            dups(t, r"(id\d)", Compare::Exact, first),
            ("id1 x\nid2 y\nno key\nno key".into(), "id1 z\n".into())
        );
        let o = DupOpts { pattern: "(".into(), ..DupOpts::default() };
        assert!(process_duplicates("a", &o).is_err());
    }

    fn matching(text: &str, pattern: &str, regex: bool, invert: bool, action: MatchAction) -> (String, String) {
        let o = MatchOpts { pattern: pattern.into(), regex, invert, action, ..MatchOpts::default() };
        process_matching(text, &o).unwrap()
    }

    #[test]
    fn each_matching_action() {
        let t = "apple\nbanana\ncherry\navocado";
        let (a, b) = ("apple\navocado", "banana\ncherry");
        for (invert, action, kept, lines) in [
            (false, MatchAction::Keep, a, a),
            (false, MatchAction::Delete, b, a),
            // copy and extract leave the document alone
            (false, MatchAction::Copy, t, a),
            (false, MatchAction::Extract, t, a),
            // inverted: the lines that do not match are the ones acted on
            (true, MatchAction::Keep, b, b),
            (true, MatchAction::Delete, a, b),
        ] {
            let want = (kept.to_string(), format!("{lines}\n"));
            assert_eq!(matching(t, "^a", true, invert, action), want, "{action:?}");
        }
        assert_eq!(matching("a\nb\n", "a", true, false, MatchAction::Keep).0, "a\n");
    }

    #[test]
    fn a_blank_line_is_a_line_to_remove_or_to_match() {
        let (_, removed) = dups("a\n\nb\n\nc", "", Compare::Exact, DupAction::KeepFirst);
        assert_eq!(removed, "\n", "the second blank line was removed");
        let (_, matched) = matching("a\n\nb", "^$", true, false, MatchAction::Copy);
        assert_eq!(matched, "\n");
        // no lines at all is not a blank line
        let (_, removed) = dups("a\nb", "", Compare::Exact, DupAction::KeepFirst);
        assert_eq!(removed, "");
        assert_eq!(matching("a", "z", true, false, MatchAction::Copy).1, "");
    }

    #[test]
    fn an_empty_text_has_no_lines_to_copy_or_remove() {
        for action in [MatchAction::Keep, MatchAction::Delete, MatchAction::Copy, MatchAction::Extract] {
            assert_eq!(matching("", "^$", true, false, action), (String::new(), String::new()));
        }
        for action in [DupAction::KeepFirst, DupAction::KeepLast, DupAction::DeleteAll, DupAction::KeepOnlyDuplicated] {
            assert_eq!(dups("", "", Compare::Exact, action), (String::new(), String::new()));
        }
        assert_eq!(prefix("", "> ", false), "");
        assert_eq!(suffix("", ";", false), "");
    }

    #[test]
    fn literal_and_case_options() {
        let t = "a.b\nacb\nA.B";
        let kept = |regex, ci| {
            let o = MatchOpts { pattern: "a.b".into(), regex, case_insensitive: ci, ..MatchOpts::default() };
            process_matching(t, &o).unwrap().0
        };
        assert_eq!(kept(false, false), "a.b", "a dot is a dot");
        assert_eq!(kept(true, false), "a.b\nacb");
        assert_eq!(kept(false, true), "a.b\nA.B");
        let bad = MatchOpts { pattern: "(".into(), regex: true, ..MatchOpts::default() };
        assert!(process_matching(t, &bad).is_err());
        assert!(process_matching(t, &MatchOpts::default()).is_err(), "no pattern");
    }

    #[test]
    fn prefix_and_suffix() {
        assert_eq!(prefix("a\n\nb", "> ", true), "> a\n\n> b");
        assert_eq!(prefix("a\n  \nb", "> ", true), "> a\n  \n> b", "blank means white space");
        assert_eq!(prefix("a\n\nb", "> ", false), "> a\n> \n> b");
        assert_eq!(prefix("a\nb\n", "- ", false), "- a\n- b\n");
        assert_eq!(suffix("a\n\nb", ";", true), "a;\n\nb;");
        assert_eq!(suffix("a\n\nb", ";", false), "a;\n;\nb;");
        // only lines that start or end with it change, and only once
        assert_eq!(remove_prefix("> a\nb\n> > c\n", "> "), "a\nb\n> c\n");
        assert_eq!(remove_suffix("a;\nb\nc;;", ";"), "a\nb\nc;");
    }

    #[test]
    fn quotes_both_ways() {
        assert_eq!(straighten_quotes("‘a’ ‚b‛ ′ “c” „d‟ ″"), "'a' 'b' ' \"c\" \"d\" \"");
        assert_eq!(smart_quotes("\"hi\" it's ('x')"), "“hi” it’s (‘x’)");
        assert_eq!(smart_quotes("x—\"y\" [\"z\"] {'w'} <\"v\">"), "x—“y” [“z”] {‘w’} <“v”>");
        assert_eq!(smart_quotes("a\n\"b\" 'c'"), "a\n“b” ‘c’", "after a line break opens");
        assert_eq!(smart_quotes("rock'n'roll 5'10\""), "rock’n’roll 5’10”");
        assert_eq!(smart_quotes(&straighten_quotes("“a” it’s")), "“a” it’s");
    }

    #[test]
    fn a_quote_straight_after_an_opening_quote_of_the_other_kind_opens_too() {
        assert_eq!(smart_quotes("\"'hi' there\""), "“‘hi’ there”");
        assert_eq!(smart_quotes("'\"hi\" there'"), "‘“hi” there’");
        assert_eq!(smart_quotes("say \"'a' and 'b'\"!"), "say “‘a’ and ‘b’”!");
        // the same kind straight after an opening quote is an empty pair
        assert_eq!(smart_quotes("a \"\" b '' c"), "a “” b ‘’ c");
        // a curly quote already in the text counts as it is
        assert_eq!(smart_quotes("“'hi'”"), "“‘hi’”");
    }

    #[test]
    fn unicode_forms_and_diacritics() {
        let (composed, decomposed) = ("\u{e9}", "e\u{301}");
        assert_eq!(normalize(composed, NormForm::Nfd), decomposed);
        assert_eq!(normalize(decomposed, NormForm::Nfc), composed);
        assert_eq!(normalize("\u{fb01}", NormForm::Nfc), "\u{fb01}", "NFC keeps the ligature");
        assert_eq!(normalize("\u{fb01}", NormForm::Nfkc), "fi");
        assert_eq!(normalize("\u{2460}\u{e9}", NormForm::Nfkd), "1e\u{301}");
        assert_eq!(strip_diacritics("Cr\u{e8}me br\u{fb}l\u{e9}e \u{f1} e\u{301}"), "Creme brulee n e");
        assert_eq!(strip_diacritics("plain"), "plain");
    }

    fn table(text: &str, regex: bool, ci: bool, word: bool) -> Result<Table, Vec<String>> {
        Table::parse(text, &TableOpts { regex, case_insensitive: ci, whole_word: word })
    }

    fn lit(rows: &str, text: &str) -> String {
        table(rows, false, false, false).unwrap().apply(text)
    }

    #[test]
    fn case_insensitive_literal_tables_prefer_the_longest_unicode_match() {
        let t = table("K\tshort\nkz\tlong", false, true, false).unwrap();
        assert_eq!(t.apply("kz Kz"), "long long");
    }

    #[test]
    fn table_validation_reports_every_problem_with_its_line() {
        let errors = |t: &str, regex| table(t, regex, false, false).err().unwrap();
        let e = errors("x", false);
        assert_eq!(e.len(), 1);
        assert!(e[0].starts_with("Line 1:") && e[0].contains("tab"), "{e:?}");
        let e = errors("# note\n\na\tb\n\tc\nd\te\tf\ng\th\ng\ti", false);
        assert_eq!(e.len(), 3, "{e:?}");
        assert!(e[0].starts_with("Line 4:") && e[0].contains("empty"), "{e:?}");
        assert!(e[1].starts_with("Line 5:"), "{e:?}");
        assert!(e[2].starts_with("Line 7:") && e[2].contains("line 6"), "{e:?}");
        let e = errors("ok\t1\n(\t2\n[\t3", true);
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e[0].starts_with("Line 2:") && e[0].contains("regex"), "{e:?}");
        assert!(e[1].starts_with("Line 3:"), "{e:?}");
        // a bracket is only a bracket in literal mode
        assert!(table("(\t2", false, false, false).is_ok());
        assert!(errors("# nothing\n", false)[0].contains("no rows"));
        // ignoring case, A and a are one find
        assert!(table("A\t1\na\t2", false, true, false).is_err());
        assert!(table("A\t1\na\t2", false, false, false).is_ok());
    }

    #[test]
    fn every_regex_that_cannot_be_built_names_its_line() {
        // far over the engine's size limit, twice, around a good row
        let big = "((a{100}){100}){100}";
        let rows = format!("{big}\tx\nok\ty\n{big}b\tz");
        let e = table(&rows, true, false, false).err().unwrap();
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e[0].starts_with("Line 1:") && e[0].contains("regex"), "{e:?}");
        assert!(e[1].starts_with("Line 3:"), "{e:?}");
    }

    #[test]
    fn literal_tables_make_one_pass_longest_find_first() {
        assert_eq!(lit("a\tb\nb\tc", "ab"), "bc", "no chaining");
        assert_eq!(lit("a\t1\nab\t2", "ab a"), "2 1", "longest first");
        assert_eq!(lit("x\t", "axb"), "ab", "the replacement may be empty");
        assert_eq!(lit("# c\n\n  \nx\ty\r\n", "x"), "y", "comments, blanks, CRLF");
        assert_eq!(lit("a.b\t!", "a.b axb"), "! axb", "literal, not regex");
        assert_eq!(lit("é\tE", "caf\u{e9}"), "cafE");
        assert_eq!(table("cat\tdog", false, true, false).unwrap().apply("Cat cat"), "dog dog");
        assert_eq!(table("cat\tdog", false, false, true).unwrap().apply("cat concat cat."), "dog concat dog.");
    }

    /// Scramble helper for the differential tests below.
    fn pick(state: &mut u64, below: usize) -> usize {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*state >> 33) as usize % below
    }

    #[test]
    fn a_literal_table_replaces_the_longest_find_at_each_position() {
        let alphabet = ["a", "b", "A", " ", "é", "É", "ab", "."];
        let mut state = 0x1234_5678_9abc_def1u64;
        for case in 0..300 {
            let ci = case % 2 == 0;
            let mut finds: Vec<String> = Vec::new();
            for _ in 0..1 + pick(&mut state, 4) {
                let find: String =
                    (0..1 + pick(&mut state, 3)).map(|_| alphabet[pick(&mut state, alphabet.len())]).collect();
                let same = |f: &String| match ci {
                    true => f.to_lowercase() == find.to_lowercase(),
                    false => *f == find,
                };
                if !finds.iter().any(same) {
                    finds.push(find);
                }
            }
            let rows: String = finds.iter().enumerate().map(|(i, f)| format!("{f}\t<{i}>\n")).collect();
            let text: String = (0..pick(&mut state, 12)).map(|_| alphabet[pick(&mut state, alphabet.len())]).collect();
            let table = table(&rows, false, ci, false).unwrap();
            // reference: scan left to right, the longest find that matches here wins
            let fold = |s: &str| cased(s, ci);
            let chars: Vec<char> = text.chars().collect();
            let mut want = String::new();
            let mut at = 0;
            while at < chars.len() {
                let best = finds
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| {
                        let n = f.chars().count();
                        at + n <= chars.len() && fold(&chars[at..at + n].iter().collect::<String>()) == fold(f)
                    })
                    .max_by_key(|(_, f)| f.chars().count());
                match best {
                    Some((i, f)) => {
                        want += &format!("<{i}>");
                        at += f.chars().count();
                    }
                    None => {
                        want.push(chars[at]);
                        at += 1;
                    }
                }
            }
            assert_eq!(table.apply(&text), want, "ci={ci} finds={finds:?} text={text:?}");
        }
    }

    #[test]
    fn regex_tables_apply_rows_in_order_with_captures() {
        let rx = |rows: &str, text: &str| table(rows, true, false, false).unwrap().apply(text);
        assert_eq!(rx("a\tb\nb\tc", "ab"), "cc", "each row is a full pass");
        assert_eq!(rx("(\\w+)@(\\w+)\t$2 at $1", "me@home x@y"), "home at me y at x");
        assert_eq!(rx("^\\s+\t", "  a\n b"), "a\nb", "anchors work per line");
        assert_eq!(table("CAT\tdog", true, true, true).unwrap().apply("Cat concat"), "dog concat");
    }
}

#[cfg(test)]
mod lookaround_tests {
    use super::*;

    #[test]
    fn transformations_take_the_shared_dialect() {
        let o = MatchOpts { pattern: r"\d+(?= apples)".into(), regex: true, ..MatchOpts::default() };
        let (kept, _) = process_matching("3 apples\n4 pears\n5 apples\n", &o).unwrap();
        assert_eq!(kept, "3 apples\n5 apples\n");
        let s = SortOpts { pattern: r"(?<=id=)(\d+)".into(), numeric: true, ..SortOpts::default() };
        assert_eq!(sort_by_pattern("x id=10\ny id=2\nz", &s).unwrap(), "y id=2\nx id=10\nz");
        let d = DupOpts { pattern: r"(\w)\1".into(), ..DupOpts::default() };
        assert_eq!(process_duplicates("aa1\nbb\naa2\n", &d).unwrap().0, "aa1\nbb\n");
    }
}
