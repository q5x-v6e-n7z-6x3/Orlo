//! Completion: the words, keywords and paths the list offers at the caret, and their order.
//! The editor gathers them and draws the list (`editor::popup`).

use crate::highlight::Class;
use orlo_editor::text::EditorState;
use ropey::{Rope, RopeSlice};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use syntect::parsing::syntax_definition::{MatchPattern, Pattern};

/// What a candidate is: its badge, and how ranking treats it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Word,
    Keyword,
    Path,
    Snippet,
    /// A language server's item or an index symbol of this kind ("function", "struct" ...).
    Symbol(String),
}

/// What committing a candidate puts in place of the prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Insert {
    Text(String),
    /// The snippet of this file, expanded: the store's indexes go with a reload.
    Snippet(std::path::PathBuf),
    /// The language server's item at this index: the workspace keeps the items, and resolves and
    /// applies the one committed.
    Server(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub label: String,
    pub kind: Kind,
    /// Beside the label: a snippet's description, a symbol's file.
    pub detail: String,
    pub insert: Insert,
}

impl Candidate {
    /// A candidate that inserts its label.
    pub fn text(label: String, kind: Kind) -> Self {
        Self { insert: Insert::Text(label.clone()), label, kind, detail: String::new() }
    }
}

/// Where the list is: what is typed, around it, and the scope it is typed in.
#[derive(Clone, Debug)]
pub struct Context {
    /// What is typed of the word (or of a path's last part) being completed.
    pub prefix: String,
    /// The char before the prefix.
    pub before: Option<char>,
    /// `(` follows the caret.
    pub after_paren: bool,
    pub class: Class,
    /// After `receiver.` or `receiver->`, the receiver.
    pub receiver: Option<String>,
}

/// What ranking knows beyond the context.
#[derive(Clone, Debug, Default)]
pub struct Signals {
    pub local: HashMap<String, (usize, u32)>,
    pub project: HashMap<Arc<str>, u32>,
    pub members: HashMap<String, u32>,
    /// Candidate detail (file) to distance: 0 current buffer, 1 same directory, 2 elsewhere.
    pub locality: HashMap<String, u8>,
    /// Last 50 commits, newest first, including repeated labels.
    pub recent: Vec<String>,
}

/// How much text one harvest reads: the lines nearest the caret, or the start of another buffer.
pub const HARVEST_BYTES: usize = 1 << 20;

/// Scopes (and the scopes inside them) whose patterns name a language's keywords.
const KEYWORD_SCOPES: [&str; 5] =
    ["keyword", "storage.type", "storage.modifier", "constant.language", "support.function.builtin"];

/// The keywords of syntax `name` (as View > Syntax names it), from its bundled grammar; TypeScript
/// and TypeScript (TSX), which have none, take JavaScript's. Every grammar is read at the first call (about
/// 0.2 s, so on a background thread) and the lists are kept.
pub fn keywords(name: &str) -> Arc<Vec<String>> {
    static ALL: OnceLock<HashMap<String, Arc<Vec<String>>>> = OnceLock::new();
    let all = ALL.get_or_init(|| {
        let set = crate::highlight::bundled_syntaxes().into_builder();
        (set.syntaxes().iter())
            .map(|s| {
                let patterns = (s.contexts.values().flat_map(|c| &c.patterns)).filter_map(|p| match p {
                    Pattern::Match(m) => Some(m),
                    Pattern::Include(_) => None,
                });
                (s.name.clone(), Arc::new(grammar_keywords(patterns)))
            })
            .collect()
    });
    let typescript = matches!(name, "TypeScript" | "TypeScript (TSX)");
    let javascript = typescript.then(|| all.get("JavaScript")).flatten();
    all.get(name).or(javascript).cloned().unwrap_or_default()
}

/// The keywords a grammar's patterns name: a pattern scoped as a keyword gives the identifier
/// alternatives of itself and of its groups, a capture scoped as one those of its group. A group
/// glued to letters on either side holds parts of words and gives none.
fn grammar_keywords<'a>(patterns: impl Iterator<Item = &'a MatchPattern>) -> Vec<String> {
    let mut out = Vec::new();
    for m in patterns {
        let re = m.regex.regex_str();
        let groups = groups(re);
        let free = |i: usize| !groups[i].look && !glued(re, &groups[i]);
        let mut texts: Vec<&str> = Vec::new();
        if keyword_scope(&m.scope) {
            texts.push(re);
            texts.extend((0..groups.len()).filter(|&i| free(i)).map(|i| &re[groups[i].content.clone()]));
        }
        for (n, _) in (m.captures.iter().flatten()).filter(|(_, s)| keyword_scope(s)) {
            let Some(g) = groups.iter().position(|x| x.number == Some(*n)) else {
                continue;
            };
            // the group and the groups inside it, which open after it
            let inside = |mut i: usize| loop {
                match groups[i].parent {
                    _ if i == g => return true,
                    Some(p) => i = p,
                    None => return false,
                }
            };
            let within = (g..groups.len()).filter(|&i| inside(i) && free(i));
            texts.extend(within.map(|i| &re[groups[i].content.clone()]));
        }
        // `(?x)`: the pattern's white space is not matched
        let extended = (re.strip_prefix("(?").and_then(|r| r.split_once(')')))
            .is_some_and(|(f, _)| f.contains('x') && f.chars().all(|c| c.is_ascii_alphabetic()));
        for t in texts {
            let alts = alternatives(t).into_iter();
            out.extend(alts.filter_map(|a| keyword(if extended { a.trim() } else { a })));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whether one of `scopes` is a keyword scope or inside one.
fn keyword_scope(scopes: &[syntect::parsing::Scope]) -> bool {
    scopes.iter().any(|s| {
        let s = s.build_string();
        (KEYWORD_SCOPES.iter()).any(|k| s.strip_prefix(k).is_some_and(|r| r.is_empty() || r.starts_with('.')))
    })
}

/// A group of a regex: its number when it captures, whether it only looks around (or is a
/// comment), where its content is, and the group it is in.
struct Group {
    number: Option<usize>,
    look: bool,
    content: Range<usize>,
    parent: Option<usize>,
}

/// The groups of regex `re`, in the order they open; inline flags (`(?i)`) are none.
fn groups(re: &str) -> Vec<Group> {
    let b = re.as_bytes();
    let (mut out, mut open): (Vec<Group>, Vec<usize>) = (Vec::new(), Vec::new());
    let (mut n, mut i, mut class) = (0, 0, false);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'[' if !class => class = true,
            b']' if class => class = false,
            b'(' if !class => {
                let (number, look, head) = match re[i + 1..].strip_prefix('?') {
                    None => {
                        n += 1;
                        (Some(n), false, 0)
                    }
                    Some(r) if r.starts_with(['=', '!', '#']) => (None, true, 2),
                    Some(r) if r.starts_with("<=") || r.starts_with("<!") => (None, true, 3),
                    // `(?<name>`, `(?P<name>`
                    Some(r) if r.starts_with(['<', 'P']) => {
                        n += 1;
                        (Some(n), false, 1 + r.find('>').map_or(r.len(), |e| e + 1))
                    }
                    // `(?:`, `(?i:`, `(?>`; `(?x)` alone sets flags
                    Some(r) => {
                        let flags = r.find(|c: char| !c.is_ascii_alphabetic() && c != '-').unwrap_or(r.len());
                        if r[flags..].starts_with(')') {
                            i += flags + 3;
                            continue;
                        }
                        (None, false, flags + 2)
                    }
                };
                let start = (i + 1 + head).min(b.len());
                let parent = open.last().copied();
                out.push(Group { number, look, content: start..start, parent });
                open.push(out.len() - 1);
                i = start;
                continue;
            }
            b')' if !class => {
                if let Some(g) = open.pop() {
                    out[g].content.end = i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    // a group left open (an invalid pattern) runs to the end
    for g in open {
        out[g].content.end = b.len();
    }
    out
}

/// A group that runs on into letters or digits on either side: its alternatives are parts of
/// words (`f(?:or|unc)`, `\d+(st|nd)`, `(?:un)?signed`, `(include)(_once)?`). A group before it
/// that may be absent (`(?:(async)\s+)?(def)`) does not glue.
fn glued(re: &str, g: &Group) -> bool {
    let b = re.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let escaped = |i: usize| b[..i].iter().rev().take_while(|&&c| c == b'\\').count() % 2 == 1;
    let mut end = re[..g.content.start].rfind('(').unwrap_or(0);
    let quantified = end > 0 && matches!(b[end - 1], b'+' | b'*' | b'?' | b'}');
    if quantified {
        end -= 1;
        if b[end] == b'}' {
            end = b[..end].iter().rposition(|&c| c == b'{').unwrap_or(end);
        }
    }
    let before = end > 0
        && match (b[end - 1], escaped(end - 1)) {
            (b'd' | b'w', true) => true,
            (b']' | b'.', false) => true,
            (b')', false) => !quantified,
            (c, false) => word(c),
            _ => false,
        };
    let mut after = b.get(g.content.end + 1..).unwrap_or_default();
    match after.first() {
        Some(b'?' | b'*' | b'+') => {
            after = &after[1..];
            if let Some(b'?' | b'+') = after.first() {
                after = &after[1..];
            }
        }
        Some(b'{') => after = &after[after.iter().position(|&c| c == b'}').map_or(after.len(), |e| e + 1)..],
        _ => {}
    }
    let after = match after {
        [b'\\', c, ..] => matches!(c, b'd' | b'w'),
        [c, ..] => word(*c) || matches!(c, b'[' | b'.'),
        [] => false,
    };
    before || after
}

/// `text` cut at its own `|`s, not those inside its groups or classes.
fn alternatives(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let (mut out, mut depth, mut from, mut i, mut class) = (Vec::new(), 0, 0, 0, false);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'[' if !class => class = true,
            b']' if class => class = false,
            b'(' if !class => depth += 1,
            b')' if !class => depth -= 1,
            b'|' if !class && depth == 0 => {
                out.push(&text[from..i]);
                from = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(text.get(from..).unwrap_or_default());
    out
}

/// An alternative that is a keyword: two or more identifier chars after an optional `@`, `#` or
/// `$` (escaped or not), between optional word boundaries.
fn keyword(alt: &str) -> Option<String> {
    let alt = alt.strip_prefix("\\b").unwrap_or(alt);
    let alt = alt.strip_suffix("\\b").unwrap_or(alt);
    let alt = (alt.strip_prefix('\\')).filter(|r| r.starts_with(['$', '#', '@'])).unwrap_or(alt);
    let mut body = alt.strip_prefix(['$', '#', '@']).unwrap_or(alt).chars();
    let ok = body.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && body.all(|c| c.is_ascii_alphanumeric() || c == '_');
    (ok && alt.len() >= 2).then(|| alt.to_string())
}

#[cfg(test)]
thread_local! {
    /// Lines the harvests on this thread started reading.
    static LINES_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The most bytes a word being read held.
    static LONGEST_WORD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The longest word offered, in chars.
const MAX_WORD: usize = 100;

/// The words of `line` (2 to 100 letters, digits and `_`, not a number), each with the columns it
/// takes, as `f` is given them. False when `budget` (bytes) could not take the next char: reading
/// stops there, dropping the word it would cut, and the harvest reads no more lines.
pub fn line_words(line: RopeSlice, budget: &mut usize, f: impl FnMut(&str, Range<usize>)) -> bool {
    line_words_until(line, budget, || false, f)
}

fn line_words_until(
    line: RopeSlice,
    budget: &mut usize,
    mut stop: impl FnMut() -> bool,
    mut f: impl FnMut(&str, Range<usize>),
) -> bool {
    let mut word = String::new();
    let (mut start, mut len) = (0, 0);
    let worth = |w: &str, len| (2..=MAX_WORD).contains(&len) && !w.starts_with(char::is_numeric);
    #[cfg(test)]
    LINES_READ.set(LINES_READ.get() + 1);
    for (i, c) in line.chars().enumerate() {
        if i % 1024 == 0 && stop() {
            return false;
        }
        let Some(left) = budget.checked_sub(c.len_utf8()) else {
            return false;
        };
        *budget = left;
        if EditorState::is_word_char(c) {
            if len == 0 {
                start = i;
            }
            // past the longest word it is no word: a 99 MB run of letters is not held whole
            if len < MAX_WORD {
                word.push(c);
            }
            #[cfg(test)]
            LONGEST_WORD.set(LONGEST_WORD.get().max(word.len()));
            len += 1;
            continue;
        }
        if worth(&word, len) {
            f(&word, start..start + len);
        }
        word.clear();
        len = 0;
    }
    if worth(&word, len) {
        f(&word, start..start + len);
    }
    true
}

/// The words of the lines within `limit` of the caret's (nearest lines first, until HARVEST_BYTES
/// are read), each with the lines to its nearest occurrence and how often it occurs; the word the
/// caret is in is left out.
pub fn buffer_words(rope: &Rope, caret: usize, limit: usize) -> Vec<(String, usize, u32)> {
    let caret = caret.min(rope.len_chars());
    let at = rope.char_to_line(caret);
    let col = caret - rope.line_to_char(at);
    let last = rope.len_lines() - 1;
    let mut found: HashMap<String, (usize, u32)> = HashMap::new();
    let mut budget = HARVEST_BYTES;
    'lines: for d in 0..=limit {
        if at < d && at + d > last {
            break;
        }
        let below = Some(at + d).filter(|&l| d > 0 && l <= last);
        for line in at.checked_sub(d).into_iter().chain(below) {
            let read = line_words(rope.line(line), &mut budget, |w, cols| {
                if line == at && cols.start <= col && col <= cols.end {
                    return;
                }
                match found.get_mut(w) {
                    Some(e) => e.1 += 1,
                    None => drop(found.insert(w.to_string(), (d, 1))),
                }
            });
            if !read {
                break 'lines;
            }
        }
    }
    (found.into_iter()).map(|(w, (d, n))| (w, d, n)).collect()
}

/// The words of the start of another buffer (its first HARVEST_BYTES), each once, in the order
/// they first occur.
pub fn words(rope: &Rope) -> Vec<String> {
    let (mut seen, mut out) = (HashSet::new(), Vec::new());
    let mut budget = HARVEST_BYTES;
    for line in rope.lines() {
        let read = line_words(line, &mut budget, |w, _| {
            if seen.insert(w.to_string()) {
                out.push(w.to_string());
            }
        });
        if !read {
            break;
        }
    }
    out
}

/// Syntax variants which share identifiers and symbols.
pub fn same_syntax(a: &str, b: &str) -> bool {
    fn family(s: &str) -> &str {
        match s {
            "JavaScript" | "JavaScript (JSX)" | "TypeScript" | "TypeScript (TSX)" => "JavaScript",
            _ => s,
        }
    }
    family(a) == family(b)
}

/// The operator and bounded identifier before a prefix, also used to count lexical members.
pub fn receiver_before(rope: RopeSlice, start: usize) -> (Option<char>, Option<String>) {
    let mut back = rope.chars_at(start).reversed().peekable();
    // at most MAX_WORD spaces are skipped on either side of the operator
    let skip_spaces = |back: &mut std::iter::Peekable<_>| {
        for _ in 0..MAX_WORD {
            if back.next_if(|c: &char| c.is_whitespace()).is_none() {
                break;
            }
        }
    };
    skip_spaces(&mut back);
    let operator = match back.next() {
        Some('.') => '.',
        Some('>') if back.next() == Some('-') => '>',
        Some(':') if back.next() == Some(':') => return (Some(':'), None),
        _ => return (None, None),
    };
    skip_spaces(&mut back);
    let name: Vec<_> = back.take(MAX_WORD + 1).take_while(|c| EditorState::is_word_char(*c)).collect();
    let receiver = (!name.is_empty() && name.len() <= MAX_WORD).then(|| name.into_iter().rev().collect());
    (Some(operator), receiver)
}

/// Qualified member counts, bounded for a popup, cancellable for a whole index entry.
pub fn member_counts(
    rope: &Rope,
    lines: impl Iterator<Item = usize>,
    mut budget: usize,
    stop: impl Fn() -> bool,
) -> HashMap<String, u32> {
    let mut counts = HashMap::new();
    for line in lines {
        if stop() {
            return HashMap::new();
        }
        let text = rope.line(line);
        let mut cancelled = false;
        let read = line_words_until(
            text,
            &mut budget,
            || {
                cancelled = stop();
                cancelled
            },
            |word, cols| {
                if let (_, Some(receiver)) = receiver_before(text, cols.start) {
                    *counts.entry(format!("{receiver}.{word}")).or_insert(0) += 1;
                }
            },
        );
        if cancelled {
            return HashMap::new();
        }
        if !read {
            break;
        }
    }
    counts
}

/// What a path's folder offers: the entries the prefix matches, folders with a trailing
/// `/`, hidden ones only for a prefix that starts with `.`, at most 500, by name. Reads the disk.
pub fn path_entries(dir: &Path, prefix: &str) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let hidden = prefix.starts_with('.');
    let mut names: Vec<String> = (entries.flatten())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if name.starts_with('.') && !hidden || orlo_utils::fuzzy::score(prefix, &name).is_none() {
                return None;
            }
            Some(match e.path().is_dir() {
                true => format!("{name}/"),
                false => name,
            })
        })
        .collect();
    names.sort();
    names.truncate(500);
    (names.into_iter()).map(|n| Candidate::text(n, Kind::Path)).collect()
}

const PREFIX_EXACT: f64 = 20.;
const PREFIX_FOLDED: f64 = 10.;
const PROXIMITY: f64 = 4.;
const FREQUENCY: f64 = 4.;
const PROJECT_FREQUENCY: f64 = 0.75;
const PROSE: f64 = 45.;
const MEMBER_SEEN: f64 = 120.;
const MEMBER_KIND: f64 = 25.;
const MEMBER_DEMOTE: f64 = 30.;
const NAMESPACE: f64 = 30.;
const CALL: f64 = 35.;
const CURRENT_FILE: f64 = 30.;
const SAME_DIRECTORY: f64 = 15.;
const RECENCY: f64 = 15.;

/// Matching candidates, scored by their project-context signals, then shorter label and alphabetical order.
/// Duplicate labels keep the best context; server edits and snippets keep source precedence.
pub fn rank(cands: Vec<Candidate>, ctx: &Context, signals: &Signals) -> Vec<Candidate> {
    let folded = ctx.prefix.to_lowercase();
    let recent: HashMap<&str, usize> =
        signals.recent.iter().take(50).enumerate().rev().map(|(i, label)| (label.as_str(), i)).collect();
    let preferred: HashSet<_> =
        cands.iter().filter(|c| !matches!(c.insert, Insert::Text(_))).map(|c| c.label.clone()).collect();
    let mut seen = HashSet::new();
    let mut scored: Vec<(f64, usize, Candidate)> = (cands.into_iter())
        .filter(|c| ctx.class == Class::Code || c.kind != Kind::Keyword)
        .filter(|c| ctx.class == Class::String || c.kind != Kind::Path)
        .filter(|c| !matches!(&c.insert, Insert::Text(t) if *t == ctx.prefix))
        .filter(|c| {
            !preferred.contains(&c.label) || (!matches!(c.insert, Insert::Text(_)) && seen.insert(c.label.clone()))
        })
        .filter_map(|c| {
            let mut score = f64::from(orlo_utils::fuzzy::score(&ctx.prefix, &c.label)?);
            if !ctx.prefix.is_empty() {
                score += if c.label.starts_with(&ctx.prefix) {
                    PREFIX_EXACT
                } else if c.label.to_lowercase().starts_with(&folded) {
                    PREFIX_FOLDED
                } else {
                    0.
                };
            }
            if let Some(&(distance, count)) = signals.local.get(&c.label) {
                score += FREQUENCY * (1. + f64::from(count)).log2() - PROXIMITY * (1. + distance as f64).log2();
            }
            if let Some(&count) = signals.project.get(c.label.as_str()) {
                score += PROJECT_FREQUENCY * (1. + f64::from(count)).log2();
            }
            let kind = match &c.kind {
                Kind::Symbol(k) => k.as_str(),
                _ => "",
            };
            let callable = matches!(kind, "function" | "method");
            let ty = matches!(
                kind,
                "struct" | "class" | "enum" | "type" | "interface" | "trait" | "impl" | "namespace" | "module"
            );
            if ctx.class != Class::Code {
                score += match c.kind {
                    Kind::Word => PROSE,
                    Kind::Symbol(_) => -PROSE,
                    _ => 0.,
                };
            } else {
                match ctx.before {
                    Some('.' | '>') => {
                        if ctx.receiver.is_some() {
                            if let Some(&count) = signals.members.get(&c.label) {
                                score += MEMBER_SEEN * (1. + f64::from(count)).log2();
                            }
                        }
                        if callable || matches!(kind, "field" | "property") {
                            score += MEMBER_KIND;
                        }
                        if ty || c.kind == Kind::Keyword {
                            score -= MEMBER_DEMOTE;
                        }
                    }
                    Some(':') if callable || ty || matches!(kind, "constant" | "enum member") => score += NAMESPACE,
                    _ => {}
                }
                if ctx.after_paren && callable {
                    score += CALL;
                }
                if matches!(c.kind, Kind::Symbol(_)) {
                    score += match signals.locality.get(&c.detail) {
                        Some(0) => CURRENT_FILE,
                        Some(1) => SAME_DIRECTORY,
                        _ => 0.,
                    };
                }
            }
            if let Some(&age) = recent.get(c.label.as_str()) {
                score += RECENCY * (50 - age) as f64 / 50.;
            }
            Some((score, c.label.chars().count(), c))
        })
        .collect();
    scored.sort_by(|(a, n, x), (b, m, y)| b.total_cmp(a).then_with(|| n.cmp(m)).then_with(|| x.label.cmp(&y.label)));
    seen.clear();
    scored.into_iter().filter(|(_, _, c)| seen.insert(c.label.clone())).map(|(_, _, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;

    fn identifier(k: &str) -> bool {
        let body = k.strip_prefix(['@', '#', '$']).unwrap_or(k);
        let mut c = body.chars();
        c.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    #[test]
    fn every_syntaxs_keywords_are_identifiers_and_the_known_ones_are_there() {
        let mut total = 0;
        for name in Highlighter::syntax_names() {
            let k = keywords(&name);
            total += k.len();
            for w in k.iter() {
                assert!(identifier(w) && w.len() >= 2, "{name}: {w:?}");
            }
        }
        assert!(total > 1_000, "{total} keywords in all");
        let has = |syntax: &str, words: &[&str]| {
            let k = keywords(syntax);
            for w in words {
                assert!(k.iter().any(|k| k == w), "{syntax} lacks {w}: {k:?}");
            }
        };
        has("Rust", &["fn", "match", "let", "impl"]);
        has("Python", &["def", "lambda", "class", "return"]);
        has("JavaScript", &["function", "return", "const"]);
        has("HolyC", &["switch", "lastclass", "noargpop"]);
        // no grammar of their own: JavaScript's
        assert_eq!(keywords("TypeScript"), keywords("JavaScript"));
        assert_eq!(keywords("TypeScript (TSX)"), keywords("JavaScript"));
        // regex pieces are not words: Go's `\b(?:b(?:reak)|s(?:elect|witch)…)` and PHP's
        // `(include|require)(_once)?` give no `witch` or `_once`
        for (syntax, piece) in [("Go", "witch"), ("Go", "allthrough"), ("PHP Source", "_once")] {
            assert!(!keywords(syntax).iter().any(|k| k == piece), "{piece}");
        }
        assert!(keywords("Plain Text").is_empty());
        assert!(keywords("No Such Syntax").is_empty());
    }

    fn sorted(mut v: Vec<(String, usize, u32)>) -> Vec<(String, usize, u32)> {
        v.sort();
        v
    }

    #[test]
    fn buffer_words_count_each_word_and_how_near_it_is() {
        let text = "alpha beta\nalpha gam|ma 9lives x\n\n__init__ beta\n";
        let caret = text.find('|').unwrap();
        let text = text.replace('|', "");
        let rope = Rope::from_str(&text);
        let w = |s: &str, d, n| (s.to_string(), d, n);
        // the word at the caret is not offered; one-letter words and numbers are not words
        assert_eq!(sorted(buffer_words(&rope, caret, 5_000)), [w("__init__", 2, 1), w("alpha", 0, 2), w("beta", 1, 2)]);
        // only lines within the limit
        assert_eq!(sorted(buffer_words(&rope, caret, 0)), [w("alpha", 0, 1)]);
    }

    #[test]
    fn a_huge_buffer_harvests_the_10_001_lines_nearest_the_caret() {
        // 4M chars in 500,000 lines, each its own word
        let text: String = (0..500_000).map(|i| format!("w{i:06}\n")).collect();
        assert_eq!(text.len(), 4_000_000);
        let rope = Rope::from_str(&text);
        let caret = rope.line_to_char(250_000);
        let found = buffer_words(&rope, caret, 5_000);
        // lines 245,000 to 255,000, less the caret's own word
        assert_eq!(found.len(), 10_000);
        assert!(!found.iter().any(|w| w.0 == "w250000"));
        let lines: Vec<usize> = found.iter().map(|w| w.0[1..].parse().unwrap()).collect();
        assert_eq!(lines.iter().min(), Some(&245_000));
        assert_eq!(lines.iter().max(), Some(&255_000));
        assert!(found.iter().all(|(w, d, _)| w[1..].parse::<usize>().unwrap().abs_diff(250_000) == *d));
        // one 4M-char line: the harvest stops after a megabyte of it
        let line: String = (0..400_000).map(|i| format!("w{i:08} ")).collect();
        let found = buffer_words(&Rope::from_str(&line), 0, 5_000);
        let read = found.iter().map(|w| w.0.len() + 1).sum::<usize>();
        assert!(read <= HARVEST_BYTES && read > HARVEST_BYTES - 100, "{read}");
        assert!(!found.iter().any(|w| w.0 == "w00399999"));
    }

    #[test]
    fn a_run_of_word_chars_past_100_is_no_word_and_is_not_held_whole() {
        let line = format!("{} ok {}x", "a".repeat(1_000_000), "b".repeat(100));
        let (mut got, mut budget) = (Vec::new(), usize::MAX);
        LONGEST_WORD.set(0);
        line_words(Rope::from_str(&line).line(0), &mut budget, |w, cols| got.push((w.to_string(), cols)));
        assert_eq!(got, [("ok".to_string(), 1_000_001..1_000_003)]);
        assert!(LONGEST_WORD.get() <= 100, "{}", LONGEST_WORD.get());
        // a word of exactly 100 is still one
        let w100 = "c".repeat(100);
        assert_eq!(words(&Rope::from_str(&format!("{w100} d1"))), [w100, "d1".into()]);
    }

    #[test]
    fn a_harvest_reads_no_further_lines_once_its_budget_cannot_take_the_next_char() {
        // 2^20 is no multiple of 3: after 3,518 lines of 99 three-byte chars and 70 more, the 2
        // bytes left take no char
        let rope = Rope::from_str(&format!("{}\n", "雪".repeat(99)).repeat(8_000));
        LINES_READ.set(0);
        assert_eq!(words(&rope), ["雪".repeat(99)]);
        assert_eq!(LINES_READ.get(), 3_519, "another buffer's harvest");
        LINES_READ.set(0);
        assert_eq!(buffer_words(&rope, 0, 5_000).len(), 1);
        assert_eq!(LINES_READ.get(), 3_519, "this buffer's harvest");
    }

    #[test]
    fn another_buffer_gives_the_words_of_its_first_megabyte_once_each() {
        assert_eq!(words(&Rope::from_str("delta epsilon delta x 7up")), ["delta", "epsilon"]);
        let big: String = (0..200_000).map(|i| format!("w{i:06} ")).collect();
        let found = words(&Rope::from_str(&big));
        assert_eq!(found.len(), HARVEST_BYTES / 8);
        assert_eq!(found.last().map(String::as_str), Some("w131071"));
    }

    fn labels(c: &[Candidate]) -> Vec<&str> {
        c.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn path_entries_list_a_folder_with_its_folders_marked() {
        let dir = std::env::temp_dir().join(format!("orlo-complete-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        for f in ["main.rs", "Makefile", ".hidden"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let all = path_entries(&dir, "");
        assert_eq!(labels(&all), ["Makefile", "main.rs", "src/"]);
        assert!(all.iter().all(|c| c.kind == Kind::Path));
        assert_eq!(all[2].insert, Insert::Text("src/".into()));
        // hidden entries only for a prefix that starts with a dot
        assert_eq!(labels(&path_entries(&dir, ".h")), [".hidden"]);
        assert_eq!(labels(&path_entries(&dir, "ma")), ["Makefile", "main.rs"]);
        assert!(path_entries(&dir.join("missing"), "").is_empty());
        // at most 500, of those the prefix matches
        for i in 0..600 {
            std::fs::write(dir.join(format!("f{i:03}")), "").unwrap();
        }
        assert_eq!(path_entries(&dir, "").len(), 500);
        // the cap is after the match: by name `src/` comes after the 600
        assert_eq!(labels(&path_entries(&dir, "src")), ["src/"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn ctx(prefix: &str, class: Class) -> Context {
        Context { prefix: prefix.into(), before: None, after_paren: false, class, receiver: None }
    }

    fn cands(words: &[(&str, Kind)]) -> Vec<Candidate> {
        (words.iter()).map(|(w, k)| Candidate::text(w.to_string(), k.clone())).collect()
    }

    #[test]
    fn member_scan_cancels_inside_single_line_tokens_and_whitespace() {
        for text in ["obj.length ".repeat(20_000), "x".repeat(4_000_000), " ".repeat(4_000_000) + "obj.length"] {
            let rope = Rope::from_str(&text);
            let polls = std::cell::Cell::new(0);
            let counts = member_counts(&rope, 0..rope.len_lines(), usize::MAX, || {
                polls.set(polls.get() + 1);
                polls.get() >= 3
            });
            assert!(counts.is_empty(), "cancelled scan published partial member counts");
            assert_eq!(polls.get(), 3, "cancellation was not polled within the long line");
        }
    }

    #[test]
    fn receivers_are_bounded_unicode_aware_and_require_member_operators() {
        for (text, want) in
            [("obj.", Some("obj")), ("obj->", Some("obj")), ("雪. ", Some("雪")), ("obj : ", None), ("Type::", None)]
        {
            let rope = Rope::from_str(text);
            assert_eq!(receiver_before(rope.slice(..), rope.len_chars()).1.as_deref(), want, "{text}");
        }
        let rope = Rope::from_str(&format!("{}.", "a".repeat(4_000_000)));
        assert_eq!(receiver_before(rope.slice(..), rope.len_chars()).1, None);
        assert!(same_syntax("TypeScript (TSX)", "JavaScript"));
        assert!(!same_syntax("Rust", "Python"));
    }

    fn first(c: Vec<Candidate>, context: &Context, signals: &Signals) -> String {
        rank(c, context, signals)[0].label.clone()
    }

    #[test]
    fn proximity_changes_order() {
        let c = cands(&[("alpha", Kind::Word), ("bravo", Kind::Word)]);
        let mut signals =
            Signals { local: [("alpha".into(), (100, 1)), ("bravo".into(), (0, 1))].into(), ..Signals::default() };
        assert_eq!(first(c.clone(), &ctx("", Class::Code), &signals), "bravo");
        signals.local = [("alpha".into(), (0, 1)), ("bravo".into(), (100, 1))].into();
        assert_eq!(first(c, &ctx("", Class::Code), &signals), "alpha");
    }

    #[test]
    fn local_and_project_frequency_change_order() {
        let c = cands(&[("alpha", Kind::Word), ("bravo", Kind::Word)]);
        let context = ctx("", Class::Code);
        let mut signals = Signals::default();
        signals.local.insert("bravo".into(), (0, 8));
        assert_eq!(first(c.clone(), &context, &signals), "bravo");
        signals.local.clear();
        signals.project.insert("alpha".into(), 8);
        assert_eq!(first(c.clone(), &context, &signals), "alpha");
        signals.project.clear();
        signals.project.insert("bravo".into(), 8);
        assert_eq!(first(c, &context, &signals), "bravo");
    }

    #[test]
    fn scope_prioritizes_prose_and_paths_are_only_in_strings() {
        let c = cands(&[
            ("abacus", Kind::Word),
            ("about", Kind::Symbol("function".into())),
            ("ab", Kind::Keyword),
            ("ab/path", Kind::Path),
        ]);
        let signals = Signals::default();
        assert_eq!(first(c.clone(), &ctx("a", Class::Code), &signals), "ab");
        for class in [Class::Comment, Class::String] {
            let rows = rank(c.clone(), &ctx("a", class), &signals);
            assert_eq!(rows[0].label, "abacus");
            assert!(!rows.iter().any(|c| c.kind == Kind::Keyword));
            assert_eq!(rows.iter().any(|c| c.kind == Kind::Path), class == Class::String);
        }
    }

    #[test]
    fn member_and_namespace_context_change_order() {
        let c = cands(&[
            ("len", Kind::Word),
            ("length", Kind::Word),
            ("lexeme", Kind::Symbol("method".into())),
            ("legacy", Kind::Symbol("struct".into())),
        ]);
        let mut context = ctx("le", Class::Code);
        let mut signals = Signals::default();
        signals.members.insert("length".into(), 1);
        assert_eq!(first(c.clone(), &context, &signals), "len");
        for before in ['.', '>'] {
            context.before = Some(before);
            context.receiver = Some("obj".into());
            assert_eq!(first(c.clone(), &context, &signals), "length");
        }
        signals.members.clear();
        assert_eq!(first(c.clone(), &context, &signals), "lexeme");
        context.before = Some(':');
        assert_eq!(first(c, &context, &signals), "legacy");
    }

    #[test]
    fn call_context_changes_order() {
        let c = cands(&[("alpha", Kind::Symbol("variable".into())), ("bravo", Kind::Symbol("function".into()))]);
        let mut context = ctx("", Class::Code);
        assert_eq!(first(c.clone(), &context, &Signals::default()), "alpha");
        context.after_paren = true;
        assert_eq!(first(c, &context, &Signals::default()), "bravo");
    }

    #[test]
    fn locality_changes_order() {
        let mut c = cands(&[("alpha", Kind::Symbol("function".into())), ("bravo", Kind::Symbol("function".into()))]);
        c[0].detail = "/else/a.rs".into();
        c[1].detail = "/here/b.rs".into();
        let mut signals =
            Signals { locality: [("/else/a.rs".into(), 2), ("/here/b.rs".into(), 0)].into(), ..Signals::default() };
        assert_eq!(first(c.clone(), &ctx("", Class::Code), &signals), "bravo");
        signals.locality.insert("/else/a.rs".into(), 0);
        signals.locality.insert("/here/b.rs".into(), 1);
        assert_eq!(first(c, &ctx("", Class::Code), &signals), "alpha");
    }

    #[test]
    fn recency_changes_order_newest_first() {
        let c = cands(&[("alpha", Kind::Word), ("bravo", Kind::Word)]);
        let mut signals = Signals::default();
        assert_eq!(first(c.clone(), &ctx("", Class::Code), &signals), "alpha");
        signals.recent = vec!["bravo".into(), "alpha".into()];
        assert_eq!(first(c.clone(), &ctx("", Class::Code), &signals), "bravo");
        signals.recent.reverse();
        assert_eq!(first(c, &ctx("", Class::Code), &signals), "alpha");
    }

    #[test]
    fn duplicate_symbols_use_locality_and_prose_words_keep_their_kind() {
        let mut c = cands(&[
            ("alpha", Kind::Symbol("function".into())),
            ("alpha", Kind::Symbol("function".into())),
            ("alpha", Kind::Word),
        ]);
        c[0].detail = "far".into();
        c[1].detail = "current".into();
        let signals = Signals { locality: [("far".into(), 2), ("current".into(), 0)].into(), ..Signals::default() };
        let code = rank(c.clone(), &ctx("a", Class::Code), &signals);
        assert_eq!(code.len(), 1);
        assert_eq!(code[0].detail, "current");
        assert_eq!(rank(c.clone(), &ctx("a", Class::Comment), &signals)[0].kind, Kind::Word);
        let mut server = Candidate::text("alpha".into(), Kind::Symbol("variable".into()));
        server.insert = Insert::Server(3);
        c.push(server);
        assert_eq!(rank(c, &ctx("a", Class::Code), &signals)[0].insert, Insert::Server(3));
    }

    #[test]
    fn case_sensitive_prefix_wins_and_ties_are_stable() {
        let c = cands(&[("Alpha", Kind::Word), ("alpha", Kind::Word), ("alpha", Kind::Keyword)]);
        assert_eq!(first(c.clone(), &ctx("a", Class::Code), &Signals::default()), "alpha");
        assert_eq!(first(c, &ctx("A", Class::Code), &Signals::default()), "Alpha");
    }

    #[test]
    #[ignore = "timing gate; run with --release"]
    fn rank_20_000_candidates_under_10ms() {
        let c: Vec<_> = (0..20_000)
            .map(|i| {
                let kind = if i % 2 == 0 { Kind::Symbol("function".into()) } else { Kind::Word };
                let mut candidate = Candidate::text(format!("candidate_{:05}", (i * 7919) % 20_000), kind);
                candidate.detail = "current".into();
                candidate
            })
            .collect();
        let mut context = ctx("can", Class::Code);
        context.before = Some('.');
        context.receiver = Some("obj".into());
        context.after_paren = true;
        let signals = Signals {
            local: c.iter().map(|c| (c.label.clone(), (2, 5))).collect(),
            project: c.iter().map(|c| (Arc::from(c.label.as_str()), 7)).collect(),
            members: c.iter().map(|c| (c.label.clone(), 2)).collect(),
            locality: [("current".into(), 0)].into(),
            recent: c.iter().take(50).map(|c| c.label.clone()).collect(),
        };
        let mut best = std::time::Duration::MAX;
        for _ in 0..10 {
            let input = c.clone();
            let start = std::time::Instant::now();
            assert_eq!(rank(input, &context, &signals).len(), 20_000);
            best = best.min(start.elapsed());
        }
        eprintln!("20,000 candidates: best {best:?}");
        assert!(best < std::time::Duration::from_millis(10), "{best:?}");
    }

    #[test]
    fn rank_orders_by_match_then_the_shorter_then_alphabetical() {
        let c = cands(&[
            ("format", Kind::Word),
            ("for", Kind::Keyword),
            ("foo", Kind::Word),
            ("buffer", Kind::Word),
            ("fo", Kind::Word),
            ("forest", Kind::Word),
            ("ofo", Kind::Word),
        ]);
        let ranked = rank(c, &ctx("fo", Class::Code), &Signals::default());
        // what typing "fo" would leave as it is is not offered; "ofo" matches worse, "buffer" not at all
        assert_eq!(labels(&ranked), ["foo", "for", "forest", "format", "ofo"]);
        // an empty prefix offers everything
        let all = rank(cands(&[("b", Kind::Word), ("a", Kind::Word)]), &ctx("", Class::Code), &Signals::default());
        assert_eq!(labels(&all), ["a", "b"]);
    }

    #[test]
    fn rank_leaves_keywords_out_of_comments_and_strings_and_lists_a_label_once() {
        let c =
            cands(&[("match", Kind::Keyword), ("match", Kind::Word), ("matches", Kind::Word), ("mat", Kind::Keyword)]);
        let code = rank(c.clone(), &ctx("ma", Class::Code), &Signals::default());
        assert_eq!(labels(&code), ["mat", "match", "matches"]);
        assert_eq!(code[1].kind, Kind::Keyword, "the first offer of a label");
        for class in [Class::Comment, Class::String] {
            let prose = rank(c.clone(), &ctx("ma", class), &Signals::default());
            assert_eq!(labels(&prose), ["match", "matches"], "{class:?}");
            assert!(prose.iter().all(|c| c.kind == Kind::Word));
        }
    }
}
