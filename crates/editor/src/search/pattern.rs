//! The regex dialect, the replacement templates and Preserve Case that every search
//! workflow shares: Find and Replace, Find in Files and Replace in Files, extraction, marking and
//! the playground. Pure, so it runs on any thread.
//!
//! A pattern the Rust `regex` syntax accepts runs on regex-automata's linear-time `meta` engine.
//! One it rejects (lookaround, backreferences, atomic groups, conditionals)
//! runs on fancy-regex's backtracking VM. There Orlo steps the start positions itself with
//! anchored searches, so `BACKTRACK_LIMIT` applies per start position and `stop` is asked between
//! them, and bounds every search with `RegexInput::range`, so a selection limits the match while
//! lookaround and anchors still see the text around it.

use crate::buffer::FindOpts;
use fancy_regex::{Expr, RegexBuilder, RegexInput};
use regex_automata::{meta, Input, PatternID};
use std::ops::Range;
use std::time::{Duration, Instant};

/// Backtracking steps a lookaround or backreference pattern may take at one start position.
pub const BACKTRACK_LIMIT: usize = 1_000_000;

/// Seconds a search on the UI thread may take (a small buffer's Find Next, Replace All).
pub const UI_SECONDS: u64 = 2;
/// Seconds a background search of one text may take (a big buffer, a file, the playground).
pub const BACKGROUND_SECONDS: u64 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchError {
    /// More than `BACKTRACK_LIMIT` backtracking steps at a start position on this line (1-based).
    Limit { line: usize },
    /// Over its time (seconds): stopped, with what it found so far dropped.
    TimedOut(u64),
    /// A newer search, a closed panel or the user stopped it: nothing to report.
    Cancelled,
}

impl SearchError {
    /// What the panel or results say instead of a match count.
    pub fn message(&self) -> String {
        match self {
            SearchError::Limit { line } => format!(
                "Search stopped: the pattern backtracked more than {} times at a position on \
                 line {line}. Simplify it (an atomic group or a possessive repeat helps).",
                group_digits(BACKTRACK_LIMIT)
            ),
            SearchError::TimedOut(s) => format!("Search stopped: it took longer than {s} s."),
            SearchError::Cancelled => "Search cancelled.".into(),
        }
    }
}

pub fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// When a search must give up: `passed` is its `stop`, and `explain` tells a timeout from a
/// cancellation.
pub struct Deadline {
    at: Instant,
    secs: u64,
}

impl Deadline {
    pub fn new(secs: u64) -> Self {
        Self { at: Instant::now() + Duration::from_secs(secs), secs }
    }

    pub fn passed(&self) -> bool {
        Instant::now() >= self.at
    }

    pub fn explain(&self, e: SearchError) -> SearchError {
        match e {
            SearchError::Cancelled if self.passed() => SearchError::TimedOut(self.secs),
            e => e,
        }
    }
}

/// A compiled Find query in regex mode.
pub struct Pattern {
    engine: Engine,
    /// Each group's name, group 0 (the match) first.
    names: Vec<Option<String>>,
}

enum Engine {
    Fast(meta::Regex),
    Fancy(fancy_regex::Regex),
}

/// The 1-based line of byte `at`.
fn line_of(hay: &str, at: usize) -> usize {
    1 + hay.as_bytes()[..at.min(hay.len())].iter().filter(|&&b| b == b'\n').count()
}

/// Whether regex-syntax's complaint is about a construct only the backtracking engine has, so
/// its error is the one to show.
fn wants_fancy(syntax_error: &str) -> bool {
    syntax_error.contains("look-around") || syntax_error.contains("backreference")
}

/// `\G` and `\K` assume the engine's own search loop: Orlo steps start positions itself.
fn unsupported(query: &str) -> Option<String> {
    fn walk(e: &Expr) -> Option<&'static str> {
        match e {
            Expr::KeepOut => Some(r"\K"),
            Expr::ContinueFromPreviousMatchEnd => Some(r"\G"),
            _ => e.children_iter().find_map(walk),
        }
    }
    let tree = Expr::parse_tree(query).ok()?;
    walk(&tree.expr).map(|w| format!("{w} is not supported in Orlo's searches"))
}

impl Pattern {
    pub fn new(query: &str, o: &FindOpts) -> Result<Self, String> {
        match o.hir(query) {
            Ok(hir) => {
                let re = FindOpts::engine().build_from_hir(&hir).map_err(|e| e.to_string())?;
                let names = (re.group_info().pattern_names(PatternID::ZERO)).map(|n| n.map(str::to_string)).collect();
                Ok(Self { engine: Engine::Fast(re), names })
            }
            Err(syntax) => Self::fancy(query, o, &syntax),
        }
    }

    /// The backtracking engine, for patterns the linear one cannot take (and, in tests, to show
    /// both agree on those it can).
    pub(crate) fn fancy(query: &str, o: &FindOpts, syntax_error: &str) -> Result<Self, String> {
        let build = |p: &str| {
            RegexBuilder::new(p)
                .case_insensitive(!o.case_sensitive)
                .multi_line(true)
                .backtrack_limit(BACKTRACK_LIMIT)
                .delegate_size_limit(10 << 20)
                .build()
        };
        let bare = build(query).map_err(|e| match e {
            fancy_regex::Error::ParseError(..) if !wants_fancy(syntax_error) => syntax_error.to_string(),
            e => e.to_string(),
        })?;
        if let Some(why) = unsupported(query) {
            return Err(why);
        }
        // Whole word puts its edges inside the engine, so a greedy match can still choose a
        // shorter end. Without it an empty alternative ends the pattern: fancy-regex otherwise
        // hands `easy(?=easy)` to regex-automata with the lookahead's text inside the match,
        // which a selection's end then cuts off. A trailing `(?x)` comment would swallow the
        // closing parenthesis without the newline.
        let edge = r"(?:\b{start-half}|\b{end-half})";
        let (head, tail) = match o.whole_word {
            true => (edge, edge),
            false => ("", "(?:|)"),
        };
        drop(bare);
        let re = build(&format!("{head}(?:{query}){tail}"))
            .or_else(|_| build(&format!("{head}(?:{query}\n){tail}")))
            .map_err(|e| e.to_string())?;
        let names = re.capture_names().map(|n| n.map(str::to_string)).collect();
        Ok(Self { engine: Engine::Fancy(re), names })
    }

    /// Lookaround or backreferences: the backtracking engine, which searches big texts on
    /// another thread.
    pub fn is_fancy(&self) -> bool {
        matches!(self.engine, Engine::Fancy(_))
    }

    /// Groups, the whole match (group 0) included.
    pub fn group_count(&self) -> usize {
        self.names.len()
    }

    /// Each group's name, group 0 first.
    pub fn names(&self) -> &[Option<String>] {
        &self.names
    }

    /// Non-overlapping, leftmost-first matches (bytes) inside each of `scopes` (bytes, sorted,
    /// disjoint), the text outside a scope still visible to anchors and lookaround. After an
    /// empty match the search moves on a character; an empty match where the previous match
    /// ended is skipped, as regex-automata's iterator does.
    pub fn find(
        &self,
        hay: &str,
        scopes: &[Range<usize>],
        stop: &dyn Fn() -> bool,
    ) -> Result<Vec<Range<usize>>, SearchError> {
        if stop() {
            return Err(SearchError::Cancelled);
        }
        let mut out = Vec::new();
        for scope in scopes {
            match &self.engine {
                Engine::Fast(re) => {
                    let found = re.find_iter(Input::new(hay).range(scope.clone()));
                    for (n, m) in found.enumerate() {
                        if (n + 1).is_multiple_of(1024) && stop() {
                            return Err(SearchError::Cancelled);
                        }
                        out.push(m.range());
                    }
                }
                Engine::Fancy(re) => fancy_scope(re, hay, scope.clone(), stop, &mut out, false)?,
            }
        }
        Ok(out)
    }

    /// `find` over the whole text.
    pub fn find_all(&self, hay: &str, stop: &dyn Fn() -> bool) -> Result<Vec<Range<usize>>, SearchError> {
        self.find(hay, std::slice::from_ref(&(0..hay.len())), stop)
    }

    /// Whether it matches anywhere in `hay` (a transformation's line test).
    pub fn is_match(&self, hay: &str) -> Result<bool, SearchError> {
        match &self.engine {
            Engine::Fast(re) => Ok(re.is_match(hay)),
            Engine::Fancy(re) => {
                let mut one = Vec::new();
                fancy_scope(re, hay, 0..hay.len(), &|| false, &mut one, true)?;
                Ok(!one.is_empty())
            }
        }
    }

    /// The groups (bytes) of the match found at exactly `m`, the text around it as context;
    /// `None` if the pattern does not match exactly there (the text changed).
    pub fn captures(&self, hay: &str, m: Range<usize>) -> Option<Vec<Option<Range<usize>>>> {
        match &self.engine {
            Engine::Fast(re) => {
                let mut caps = re.create_captures();
                re.captures(Input::new(hay).range(m.clone()), &mut caps);
                (caps.get_match()?.range() == m).then(|| groups(&caps))
            }
            Engine::Fancy(re) => {
                let input = RegexInput::new(hay).range(m.clone()).from_pos(m.start).anchored(true);
                let c = re.captures_input(input).ok()??;
                (c.get(0)?.range() == m).then(|| (0..c.len()).map(|i| c.get(i).map(|g| g.range())).collect())
            }
        }
    }

    /// The first match at or after `from` (bytes) inside `scope`, with its groups: the
    /// playground's capture inspector.
    pub fn captures_from(
        &self,
        hay: &str,
        scope: Range<usize>,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<Option<Range<usize>>>>, SearchError> {
        match &self.engine {
            Engine::Fast(re) => {
                let mut caps = re.create_captures();
                re.captures(Input::new(hay).range(scope), &mut caps);
                Ok(caps.is_match().then(|| groups(&caps)))
            }
            Engine::Fancy(re) => {
                let mut one = Vec::new();
                fancy_scope(re, hay, scope, stop, &mut one, true)?;
                Ok(one.pop().and_then(|m| self.captures(hay, m)))
            }
        }
    }
}

fn groups(caps: &regex_automata::util::captures::Captures) -> Vec<Option<Range<usize>>> {
    (0..caps.group_len()).map(|i| caps.get_group(i).map(|s| s.range())).collect()
}

fn next_char(hay: &str, at: usize) -> usize {
    at + hay[at..].chars().next().map_or(1, char::len_utf8)
}

/// One scope's matches on the backtracking engine, a start position at a time.
fn fancy_scope(
    re: &fancy_regex::Regex,
    hay: &str,
    scope: Range<usize>,
    stop: &dyn Fn() -> bool,
    out: &mut Vec<Range<usize>>,
    first: bool,
) -> Result<(), SearchError> {
    let (mut at, mut last_end, mut n) = (scope.start, None, 0u32);
    while at <= scope.end {
        n = n.wrapping_add(1);
        if n.is_multiple_of(256) && stop() {
            return Err(SearchError::Cancelled);
        }
        let input = RegexInput::new(hay).range(scope.clone()).from_pos(at).anchored(true);
        match re.find_input(input) {
            Ok(Some(m)) if !(m.start() == m.end() && last_end == Some(m.end())) => {
                out.push(m.start()..m.end());
                if first {
                    return Ok(());
                }
                last_end = Some(m.end());
                at = match m.end() > m.start() {
                    true => m.end(),
                    false => next_char(hay, at),
                };
                continue;
            }
            Ok(_) => {}
            Err(_) => return Err(SearchError::Limit { line: line_of(hay, at) }),
        }
        at = next_char(hay, at);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaseOp {
    /// `\U`: upper case until `\E` or `\L`.
    Upper,
    /// `\L`: lower case until `\E` or `\U`.
    Lower,
    /// `\E`: as written again.
    End,
    /// `\u`: the next character upper case.
    NextUpper,
    /// `\l`: the next character lower case.
    NextLower,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Text(String),
    Group(usize),
    Case(CaseOp),
}

/// A replacement, parsed and checked against its pattern before anything is replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    parts: Vec<Part>,
    /// It holds `\U`, `\L`, `\E`, `\u` or `\l`: explicit case wins over Preserve Case.
    cased: bool,
}

fn cap_letter(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Template {
    /// Literal mode: the text exactly as typed.
    pub fn literal(text: &str) -> Self {
        Self { parts: vec![Part::Text(text.to_string())], cased: false }
    }

    /// Regex mode: `$1`, `$name`, `${name}` and `$$` as regex-automata reads them, a group the
    /// pattern does not have being an error; `\U`, `\L`, `\E`, `\u`, `\l`, `\n`, `\t` and `\\`;
    /// any other backslash is itself.
    pub fn parse(text: &str, p: &Pattern) -> Result<Self, String> {
        let (b, mut i) = (text.as_bytes(), 0);
        let mut parts = Vec::new();
        let mut lit = String::new();
        let resolve = |name: &str| -> Result<usize, String> {
            if let Ok(n) = name.parse::<usize>() {
                return match n < p.group_count() {
                    true => Ok(n),
                    false => Err(format!(
                        "The replacement refers to group {n}, but the pattern has {} group{}",
                        p.group_count() - 1,
                        if p.group_count() == 2 { "" } else { "s" }
                    )),
                };
            }
            (p.names.iter())
                .position(|n| n.as_deref() == Some(name))
                .ok_or_else(|| format!("The pattern has no group named \"{name}\" (write $$ for a literal $)"))
        };
        let flush = |lit: &mut String, parts: &mut Vec<Part>| {
            if !lit.is_empty() {
                parts.push(Part::Text(std::mem::take(lit)));
            }
        };
        while i < b.len() {
            match b[i] {
                b'$' => {
                    // a group's name and where the text after it starts
                    let (name, next) = match b.get(i + 1) {
                        Some(b'{') => {
                            let Some(close) = text[i + 2..].find('}') else {
                                return Err(format!(
                                    "\"${{\" at character {} has no closing }} (write $$ for a literal $)",
                                    text[..i].chars().count() + 1
                                ));
                            };
                            (&text[i + 2..i + 2 + close], i + 3 + close)
                        }
                        Some(&c) if cap_letter(c) => {
                            let end = (i + 1..b.len()).find(|&j| !cap_letter(b[j])).unwrap_or(b.len());
                            (&text[i + 1..end], end)
                        }
                        // `$$` is one `$`, and any other `$` is itself
                        after => {
                            lit.push('$');
                            i += 1 + usize::from(after == Some(&b'$'));
                            continue;
                        }
                    };
                    let g = resolve(name)?;
                    flush(&mut lit, &mut parts);
                    parts.push(Part::Group(g));
                    i = next;
                }
                b'\\' => {
                    let op = match b.get(i + 1) {
                        Some(b'U') => Some(CaseOp::Upper),
                        Some(b'L') => Some(CaseOp::Lower),
                        Some(b'E') => Some(CaseOp::End),
                        Some(b'u') => Some(CaseOp::NextUpper),
                        Some(b'l') => Some(CaseOp::NextLower),
                        _ => None,
                    };
                    let text_for = match b.get(i + 1) {
                        Some(b'n') => Some('\n'),
                        Some(b't') => Some('\t'),
                        Some(b'\\') => Some('\\'),
                        _ => None,
                    };
                    if let Some(op) = op {
                        flush(&mut lit, &mut parts);
                        parts.push(Part::Case(op));
                        i += 2;
                    } else if let Some(c) = text_for {
                        lit.push(c);
                        i += 2;
                    } else {
                        lit.push('\\');
                        i += 1;
                    }
                }
                _ => {
                    let c = text[i..].chars().next().unwrap_or_default();
                    lit.push(c);
                    i += c.len_utf8();
                }
            }
        }
        flush(&mut lit, &mut parts);
        let cased = parts.iter().any(|p| matches!(p, Part::Case(_)));
        Ok(Self { parts, cased })
    }

    /// The replacement for a match whose text is `matched`; `group(i)` is group i's text, `None`
    /// when it took no part in the match (it expands to nothing, as an empty group does).
    pub fn expand<'a>(&self, group: &dyn Fn(usize) -> Option<&'a str>, matched: &str, preserve: bool) -> String {
        let mut out = String::new();
        let (mut mode, mut once) = (CaseOp::End, None);
        let mut push = |s: &str, mode: CaseOp, once: &mut Option<CaseOp>| {
            if mode == CaseOp::End && once.is_none() {
                out.push_str(s);
                return;
            }
            for c in s.chars() {
                match once.take().unwrap_or(mode) {
                    CaseOp::Upper | CaseOp::NextUpper => out.extend(c.to_uppercase()),
                    CaseOp::Lower | CaseOp::NextLower => out.extend(c.to_lowercase()),
                    CaseOp::End => out.push(c),
                }
            }
        };
        for p in &self.parts {
            match p {
                Part::Text(t) => push(t, mode, &mut once),
                Part::Group(g) => push(group(*g).unwrap_or(""), mode, &mut once),
                Part::Case(op @ (CaseOp::NextUpper | CaseOp::NextLower)) => once = Some(*op),
                Part::Case(op) => mode = *op,
            }
        }
        match preserve && !self.cased {
            true => preserve_case(matched, &out),
            false => out,
        }
    }
}

/// Preserve Case: the replacement takes the matched text's case when that is all lower case,
/// all upper case (two cased letters or more) or capitalised (first cased letter upper, the
/// rest lower: its first cased letter is upper cased, the rest kept as written). Mixed case, or
/// no cased letters, leaves it as written. Unicode case mappings may change the length
/// (`ß` upper cases to `SS`).
pub fn preserve_case(matched: &str, replacement: &str) -> String {
    let is_cased = |c: &char| c.is_lowercase() || c.is_uppercase();
    let cased: Vec<char> = matched.chars().filter(is_cased).collect();
    let Some(first) = cased.first() else {
        return replacement.to_string();
    };
    if cased.iter().all(|c| c.is_lowercase()) {
        return replacement.to_lowercase();
    }
    if cased.len() > 1 && cased.iter().all(|c| c.is_uppercase()) {
        return replacement.to_uppercase();
    }
    let to_upper = replacement.char_indices().find(|(_, c)| is_cased(c));
    match to_upper {
        Some((i, c)) if first.is_uppercase() && cased[1..].iter().all(|c| c.is_lowercase()) => {
            let rest = &replacement[i + c.len_utf8()..];
            format!("{}{}{rest}", &replacement[..i], c.to_uppercase())
        }
        _ => replacement.to_string(),
    }
}

/// A query and its replacement, both checked: what Replace, Replace All, Replace in Files and
/// every preview use, so a preview is exactly what applying writes.
pub struct Replacer {
    pattern: Option<Pattern>,
    template: Template,
    preserve: bool,
}

impl Replacer {
    pub fn new(query: &str, replacement: &str, o: &FindOpts, preserve: bool) -> Result<Self, String> {
        let (pattern, template) = match o.regex {
            true => {
                let p = Pattern::new(query, o)?;
                let template = Template::parse(replacement, &p)?;
                (Some(p), template)
            }
            false => (None, Template::literal(replacement)),
        };
        Ok(Self { pattern, template, preserve })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pattern(&self) -> Option<&Pattern> {
        self.pattern.as_ref()
    }

    /// The text to put in place of the match at `m` (bytes of `hay`); `None` when the pattern
    /// no longer matches exactly there.
    pub fn replacement(&self, hay: &str, m: Range<usize>) -> Option<String> {
        let matched = hay.get(m.clone())?;
        let Some(p) = &self.pattern else {
            return Some(self.template.expand(&|_| None, matched, self.preserve));
        };
        let groups = p.captures(hay, m)?;
        let group = |i: usize| groups.get(i).cloned().flatten().map(|r| &hay[r]);
        Some(self.template.expand(&group, matched, self.preserve))
    }

    /// A literal replacement without Preserve Case is the same text for every match.
    pub fn fixed(&self) -> Option<&str> {
        match (&self.pattern, self.preserve, self.template.parts.as_slice()) {
            (None, false, [Part::Text(t)]) => Some(t),
            (None, false, []) => Some(""),
            _ => None,
        }
    }
}

/// A match in UTF-8 bytes and Unicode scalar offsets.
pub type MatchLocation = (Range<usize>, Range<usize>);

/// A Find query, literal or regex, compiled once for a run over many texts.
pub enum Matcher {
    Literal { query: String, opts: FindOpts },
    Regex(Pattern),
}

impl Matcher {
    pub fn new(query: &str, o: &FindOpts) -> Result<Self, String> {
        match o.regex {
            true => Pattern::new(query, o).map(Matcher::Regex),
            false => Ok(Matcher::Literal { query: query.to_string(), opts: *o }),
        }
    }

    /// Every match in `hay` as (bytes, chars), in order.
    #[allow(clippy::type_complexity)]
    pub fn find(&self, hay: &str, stop: &dyn Fn() -> bool) -> Result<Vec<(Range<usize>, Range<usize>)>, SearchError> {
        match self {
            Matcher::Literal { query, opts } => {
                let chars = crate::buffer::find_in_str_until(hay, query, opts, stop)?;
                let bytes = crate::buffer::char_to_byte_ranges(hay, &chars);
                Ok(bytes.into_iter().zip(chars).collect())
            }
            Matcher::Regex(p) => {
                let bytes = p.find_all(hay, stop)?;
                let chars = crate::buffer::byte_to_char_ranges(hay, &bytes);
                Ok(bytes.into_iter().zip(chars).collect())
            }
        }
    }

    /// Complete matches inside character intervals, with full-source anchors and word context.
    pub fn find_scoped(
        &self,
        hay: &str,
        scopes: &[Range<usize>],
        stop: &dyn Fn() -> bool,
    ) -> Result<Vec<MatchLocation>, SearchError> {
        match self {
            Self::Literal { query, opts } => {
                let rope = ropey::Rope::from_str(hay);
                let chars = crate::buffer::find_in_rope_until(&rope, query, opts, Some(scopes), stop)?;
                let bytes = crate::buffer::char_to_byte_ranges(hay, &chars);
                Ok(bytes.into_iter().zip(chars).collect())
            }
            Self::Regex(pattern) => {
                let scopes = crate::buffer::char_to_byte_ranges(hay, scopes);
                let bytes = pattern.find(hay, &scopes, stop)?;
                let chars = crate::buffer::byte_to_char_ranges(hay, &bytes);
                Ok(bytes.into_iter().zip(chars).collect())
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn opts(case_sensitive: bool, whole_word: bool) -> FindOpts {
        FindOpts { regex: true, case_sensitive, whole_word, ..FindOpts::default() }
    }

    fn texts(hay: &str, ranges: &[Range<usize>]) -> Vec<String> {
        ranges.iter().map(|r| hay[r.clone()].to_string()).collect()
    }

    fn find(q: &str, hay: &str, o: &FindOpts) -> Vec<String> {
        let p = Pattern::new(q, o).unwrap();
        texts(hay, &p.find_all(hay, &|| false).unwrap())
    }

    #[test]
    fn lookarounds_and_backreferences_match() {
        let o = opts(true, false);
        assert_eq!(find(r"\w+(?=!)", "so fancy! even!", &o), ["fancy", "even"]);
        assert_eq!(find(r"(?<!AU)\$\d+", "AU$10 $20", &o), ["$20"]);
        assert_eq!(find(r"(?<=\$)\d+", "AU$10 $20", &o), ["10", "20"]);
        assert_eq!(find(r"foo(?!bar)", "foobar foobaz", &o), ["foo"]);
        assert_eq!(find(r"\b(\w+) \1\b", "the the cat cat dog", &o), ["the the", "cat cat"]);
        assert_eq!(find(r"(?<q>['\x22]).*?\k<q>", r#"a 'b' "c" 'd"#, &o), ["'b'", "\"c\""]);
        assert_eq!(find(r"(?P<w>a)(?P=w)", "aaa", &o), ["aa"]);
        // a variable-length lookbehind of easy parts is accepted
        assert_eq!(find(r"(?<=a+)b", "aaab b", &o), ["b"]);
        let ci = opts(false, false);
        assert_eq!(find(r"(\w+) \1", "The the", &ci), ["The the"]);
        assert!(Pattern::new(r"\w+(?=!)", &o).unwrap().is_fancy());
        assert!(!Pattern::new(r"\w+!", &o).unwrap().is_fancy());
    }

    #[test]
    fn whole_word_applies_to_backtracking_patterns() {
        let o = opts(true, true);
        assert_eq!(find(r"foo(?=\s)", "xfoo foo bar", &o), ["foo"]);
        assert_eq!(find(r"(?x) (\w) \1 # pair", "xaa bb", &o), ["bb"]);
        assert_eq!(find(r"(a)\1", "aaa aa", &o), ["aa"]);
    }

    #[test]
    fn bad_patterns_explain_themselves() {
        let o = opts(true, false);
        let e = Pattern::new("[", &o).err().unwrap();
        assert!(e.contains("unclosed character class"), "{e}");
        let e = Pattern::new(r"(?<=a)[", &o).err().unwrap();
        assert!(e.contains("character class"), "{e}");
        let e = Pattern::new(r"(a)\2", &o).err().unwrap();
        assert!(e.contains("back reference") || e.contains("backref"), "{e}");
        for q in [r"\Gfoo", r"foo\K bar", r"(?=a\K)a"] {
            let e = Pattern::new(q, &o).err().unwrap();
            assert!(e.contains("not supported"), "{q}: {e}");
        }
    }

    #[test]
    fn a_runaway_pattern_stops_with_an_error_naming_its_line() {
        let o = opts(true, false);
        let hay = format!("ok\n{}c", "a".repeat(40));
        let p = Pattern::new(r"^(a|a)*\1b", &o).unwrap();
        let e = p.find_all(&hay, &|| false).unwrap_err();
        assert_eq!(e, SearchError::Limit { line: 2 });
        assert!(e.message().contains("1,000,000") && e.message().contains("line 2"));
        // a plain pattern over a large text is not stopped by the limit: it applies per start
        let big = "ab ".repeat(700_000) + "foo!";
        let p = Pattern::new(r"(?<=ab )foo(?=!)", &o).unwrap();
        assert_eq!(p.find_all(&big, &|| false).unwrap().len(), 1);
    }

    #[test]
    fn captures_from_gives_the_first_match_in_scope_with_its_groups() {
        let hay = "ab 12- 34-x";
        for q in [r"(\d+)-(x)?", r"(\d+)-(x)?(?!y)"] {
            let p = Pattern::new(q, &opts(true, false)).unwrap();
            let from = |a: usize| p.captures_from(hay, a..hay.len(), &|| false).unwrap();
            assert_eq!(from(4), Some(vec![Some(4..6), Some(4..5), None]), "{q}");
            assert_eq!(from(6), Some(vec![Some(7..11), Some(7..9), Some(10..11)]));
            assert_eq!(from(10), None);
        }
    }

    #[test]
    fn a_stop_cancels_and_a_deadline_says_it_timed_out() {
        let o = opts(true, false);
        let p = Pattern::new(r"(?<=a)b", &o).unwrap();
        let hay = "ab".repeat(10_000);
        assert_eq!(p.find_all(&hay, &|| true), Err(SearchError::Cancelled));
        let d = Deadline::new(0);
        let e = d.explain(p.find_all(&hay, &|| d.passed()).unwrap_err());
        assert_eq!(e, SearchError::TimedOut(0));
    }

    #[test]
    fn scopes_bound_backtracking_matches_without_inventing_boundaries() {
        let o = opts(true, false);
        let hay = "zaaaa!";
        let scoped = |q: &str| {
            let p = Pattern::new(q, &o).unwrap();
            p.find(hay, std::slice::from_ref(&(2..4)), &|| false).unwrap()
        };
        assert!(scoped(r"a+(?!a)").is_empty(), "the a after the scope is seen");
        assert_eq!(scoped(r"(a+)(?=a)"), [2..4], "the lookahead sees the a after the scope");
        assert_eq!(scoped(r"a+(?=a*!)"), [2..4]);
        assert_eq!(scoped(r"(?=a)"), [2..2, 3..3, 4..4]);
        assert!(scoped(r"(?<=z)a").is_empty(), "the z before the scope is seen");
        assert_eq!(scoped(r"(?<=a)a(?=a)"), [2..3, 3..4]);
        assert!(scoped(r"a(?=$)").is_empty());
    }

    /// Patterns both engines take: the linear one Orlo uses for them, and the backtracking one
    /// used when lookaround or backreferences are present, give the same matches.
    #[test]
    fn both_engines_agree_on_the_patterns_both_accept() {
        let hays = [
            "foo Foo FOO fooo foo_bar barfoo\nfoo",
            "é😀 id12 ID٣\nid4\n\nend",
            "a\nb\n\nc\n",
            "e\u{301} café Ǆ ǆ ß straße",
            "",
            "aaa",
            "x = foo + foo;\n  foo();\nfoo",
        ];
        let patterns = [
            "foo",
            r"\bfoo\b",
            r"\w+",
            r"\d+",
            r"\p{L}+",
            r"(?i)foo",
            "^",
            "$",
            "^$",
            r"^\w",
            r"\w$",
            "a*",
            "a*?",
            "",
            r"(f)(o+)",
            r"(?<w>\w)\w*",
            ".",
            "(?s).",
            r"\n",
            r"[^\n]+",
            r"\s+",
            r"\B",
            r"\b",
            "a|aa",
            "aa|a",
            r"foo\nfoo",
            r"\A\w+",
            r"\w+\z",
            r"[a-z&&[^aeiou]]+",
            r"\b{start}\w",
            r"(?x) f o # comment",
            r"e\p{M}*",
        ];
        let mut skipped = std::collections::BTreeSet::new();
        for hay in hays {
            for q in patterns {
                for (cs, ww) in [(true, false), (false, false), (true, true), (false, true)] {
                    let o = opts(cs, ww);
                    let fast = Pattern::new(q, &o).unwrap();
                    assert!(!fast.is_fancy(), "{q}");
                    let Ok(fancy) = Pattern::fancy(q, &o, "") else {
                        // fancy-regex does not parse every Rust class operator: not compared
                        skipped.insert(q);
                        continue;
                    };
                    let a = fast.find_all(hay, &|| false).unwrap();
                    let b = fancy.find_all(hay, &|| false).unwrap();
                    assert_eq!(a, b, "{q:?} on {hay:?}, case {cs}, word {ww}");
                    let len = hay.len();
                    let scope = [hay.floor_char_boundary(len / 3)..hay.floor_char_boundary(len * 2 / 3)];
                    let a = fast.find(hay, &scope, &|| false).unwrap();
                    let b = fancy.find(hay, &scope, &|| false).unwrap();
                    assert_eq!(a, b, "scoped {q:?} on {hay:?}, case {cs}, word {ww}");
                    for m in &a {
                        let x = fast.captures(hay, m.clone());
                        let y = fancy.captures(hay, m.clone());
                        assert_eq!(x, y, "captures of {q:?} at {m:?} in {hay:?}");
                    }
                }
            }
        }
        assert!(skipped.len() <= 1, "not compared: {skipped:?}");
    }

    fn expand(q: &str, rep: &str, hay: &str, preserve: bool) -> String {
        let o = opts(true, false);
        let r = Replacer::new(q, rep, &o, preserve).unwrap();
        let p = r.pattern().unwrap();
        let mut out = String::new();
        let mut last = 0;
        for m in p.find_all(hay, &|| false).unwrap() {
            out.push_str(&hay[last..m.start]);
            out.push_str(&r.replacement(hay, m.clone()).unwrap());
            last = m.end;
        }
        out + &hay[last..]
    }

    #[test]
    fn templates_keep_regex_automata_expansion() {
        // every template valid for the pattern expands as interpolate_string_into did
        let q = r"(?<word>\p{L}+)(\d+)?";
        let hay = "é12 abc x9 ";
        let re = opts(true, false).pattern(q).unwrap();
        for rep in
            ["$2:${word}", "$$", "$$1", "${1}x", "$1$2", "a$", "$ b", "${word}${2}", "$word-", "", "plain", "${01}"]
        {
            let mut want = String::new();
            let mut last = 0;
            let mut caps = re.create_captures();
            for m in re.find_iter(hay) {
                want.push_str(&hay[last..m.start()]);
                re.captures(Input::new(hay).range(m.range()), &mut caps);
                caps.interpolate_string_into(hay, rep, &mut want);
                last = m.end();
            }
            want.push_str(&hay[last..]);
            assert_eq!(expand(q, rep, hay, false), want, "{rep:?}");
        }
    }

    #[test]
    fn templates_reject_groups_the_pattern_does_not_have() {
        let o = opts(true, false);
        let p = Pattern::new(r"(a)(?<b>b)", &o).unwrap();
        for (t, why) in [
            ("$3", "group 3"),
            ("${nope}", "no group named \"nope\""),
            ("$1a", "no group named \"1a\""),
            ("x${1", "no closing"),
        ] {
            let e = Template::parse(t, &p).unwrap_err();
            assert!(e.contains(why), "{t}: {e}");
        }
        assert!(Template::parse("$2${b}$0$$", &p).is_ok());
        assert!(Replacer::new("[", "x", &o, false).is_err());
        // literal mode takes the text as it is
        let lit = FindOpts::default();
        let r = Replacer::new("a", r"$9 \U${x}", &lit, false).unwrap();
        assert_eq!(r.fixed(), Some(r"$9 \U${x}"));
    }

    #[test]
    fn case_escapes_newlines_and_unmatched_groups() {
        assert_eq!(expand(r"(\w+) (\w+)", r"\U$1\E $2", "ab cd", false), "AB cd");
        assert_eq!(expand(r"(\w+)", r"\u$1", "élan", false), "Élan");
        assert_eq!(expand(r"(\w+)", r"\L\u$1", "hELLO", false), "Hello");
        assert_eq!(expand(r"(\w+)", r"\u\L$1", "hELLO", false), "Hello");
        assert_eq!(expand(r"(\w+)", r"\l$1", "ABC", false), "aBC");
        assert_eq!(expand(r"(\w+)", r"\U$1", "straße", false), "STRASSE");
        assert_eq!(expand("(a),(b)", r"$2\n$1\t\\", "a,b", false), "b\na\t\\");
        // a backslash before U is an escape: `\\` writes one
        assert_eq!(expand("x", r"C:\\Users\x", "x", false), r"C:\Users\x");
        assert_eq!(expand("x", r"C:\Users", "x", false), "C:SERS");
        // \u with an unmatched group applies to the next character written
        assert_eq!(expand(r"(x)?(y)", r"\u$1$2", "y", false), "Y");
        // an unmatched group and an empty one both expand to nothing
        assert_eq!(expand(r"(x)?(z*)y", "[$1|$2]", "y", false), "[|]");
        // a lone backslash at the end, and other escapes, stay as typed
        assert_eq!(expand("a", r"\d\", "a", false), r"\d\");
    }

    #[test]
    fn preserve_case_follows_the_match() {
        for (matched, rep, want) in [
            ("foo", "Bar", "bar"),
            ("FOO", "bar", "BAR"),
            ("Foo", "bar", "Bar"),
            ("Foo", "iPhone", "IPhone"),
            ("fOO", "bar", "bar"),
            ("FoO", "bar", "bar"),
            ("123", "Bar", "Bar"),
            ("A", "bar", "Bar"),
            ("STRASSE", "straße", "STRASSE"),
            ("Élan", "été", "Été"),
            ("ΟΔΟΣ", "δρόμος", "ΔΡΌΜΟΣ"),
            ("οδος", "ΔΡΟΜΟΣ", "δρομος"),
            ("foo-bar", "Baz-Qux", "baz-qux"),
        ] {
            assert_eq!(preserve_case(matched, rep), want, "{matched} → {rep}");
        }
        // explicit case escapes win over Preserve Case; without them it applies to the result
        assert_eq!(expand(r"(\w+)", r"\L${1}x", "FOO", true), "foox");
        assert_eq!(expand(r"(\w+)", r"${1}x", "FOO", true), "FOOX");
        let lit = FindOpts::default();
        let r = Replacer::new("foo", "bar", &lit, true).unwrap();
        assert_eq!(r.replacement("Foo", 0..3).as_deref(), Some("Bar"));
        assert_eq!(r.fixed(), None);
    }
}
