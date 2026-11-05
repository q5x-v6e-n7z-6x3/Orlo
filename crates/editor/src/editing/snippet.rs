//! Snippets: the `.sublime-snippet` reader, the snippet body parser, the loader of the
//! Snippets folder, a body's expansion for one caret and the field session a view keeps after it
//! (`EditorState` makes the edits). Pure, so it runs on any thread.
//!
//! A body is Sublime's: text; `$1` and `${1:placeholder}` fields (a number used again mirrors the
//! first); `${1|first,second|}` choice fields; `${1/regex/format/options}` transformed mirrors;
//! `$NAME`, `${NAME:default}` and `${NAME/regex/format/options}` variables. Outside choices,
//! only `\$`, `\}` and `\\` are escapes. A `$` that
//! starts none of these is text, and so is a `}` outside a field. A `${` that starts one but
//! never closes, or whose transform cannot be built, is an error that says where.

use crate::buffer::{FindOpts, Selection};
use orlo_config::selector::Selector;
use orlo_project::watch::{stamp, Stamp};
use regex_automata::util::captures::Captures;
use regex_automata::{meta, util::syntax};
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A snippet file over this size is not read: a snippet is a few lines, and a stray big file in
/// the folder should not be read whole.
const MAX_FILE: u64 = 1 << 20;

/// Fields (and conditionals in a format) nested deeper are an error, not a stack overflow.
const MAX_NESTING: usize = 32;

/// The folder is looked at again when a snippet is looked up, at most this often.
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// A snippet file's extension.
pub const EXT: &str = "sublime-snippet";

/// `Application Support/Orlo/Snippets`
pub fn dir() -> PathBuf {
    orlo_utils::paths::support_dir().join("Snippets")
}

/// One piece of a snippet body.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Text(String),
    /// `$n`, `${n}` or `${n:default}`; `$0` is where the caret ends. Several with one number are
    /// one field: the first with a default, else the first, is edited and the others mirror it.
    Tab {
        n: u32,
        default: Vec<Node>,
    },
    /// `${n|first,second|}`: an editable field with owned literal alternatives.
    Choice {
        n: u32,
        values: Vec<String>,
    },
    /// `${n/regex/format/options}`: field `n` run through the transform, as a mirror.
    Transform {
        n: u32,
        re: Regex,
        fmt: Format,
        global: bool,
    },
    /// `$NAME`, `${NAME}`, `${NAME:default}` or `${NAME/regex/format/options}` (which has no
    /// default).
    Var {
        name: String,
        default: Vec<Node>,
        transform: Option<(Regex, Format, bool)>,
    },
}

/// A transform's pattern on the engine Find uses (Rust's regex syntax, so no lookaround or
/// backreferences), with the text it was made from.
#[derive(Clone)]
pub struct Regex {
    source: String,
    ignore_case: bool,
    multi_line: bool,
    engine: meta::Regex,
}

impl PartialEq for Regex {
    fn eq(&self, other: &Self) -> bool {
        (&self.source, self.ignore_case, self.multi_line) == (&other.source, other.ignore_case, other.multi_line)
    }
}

impl std::fmt::Debug for Regex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (i, m) = (self.ignore_case, self.multi_line);
        let flags = [(i, "i"), (m, "m")].map(|(on, flag)| if on { flag } else { "" });
        write!(f, "/{}/{}", self.source, flags.concat())
    }
}

impl Regex {
    /// `^` and `$` match at the ends of every line only with `multi_line` (the `m` option).
    fn new(source: &str, ignore_case: bool, multi_line: bool) -> Result<Self, String> {
        let config = syntax::Config::new().case_insensitive(ignore_case).multi_line(multi_line);
        let hir = syntax::parse_with(source, &config).map_err(|e| syntax_error(&e))?;
        let engine = FindOpts::engine().build_from_hir(&hir).map_err(|e| e.to_string())?;
        Ok(Self { source: source.into(), ignore_case, multi_line, engine })
    }

    /// `text` with its first match, or every match when `global`, replaced by `fmt`.
    pub fn replace(&self, fmt: &Format, global: bool, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        let limit = if global { usize::MAX } else { 1 };
        for caps in self.engine.captures_iter(text).take(limit) {
            let Some(whole) = caps.get_match() else {
                continue;
            };
            out.push_str(&text[last..whole.start()]);
            let mut replacement = Out::default();
            fmt.write(&caps, text, &mut replacement);
            out.push_str(&replacement.text);
            last = whole.end();
        }
        out.push_str(&text[last..]);
        out
    }
}

/// What is wrong with a regex in a line, without regex-syntax's picture of the pattern: the
/// problem, and the part of the pattern it is at.
fn syntax_error(e: &regex_syntax::Error) -> String {
    use regex_syntax::{
        ast::{ErrorKind, Span},
        Error,
    };
    let at = |pattern: &str, span: &Span| match pattern.get(span.start.offset..span.end.offset) {
        Some(part) if !part.is_empty() => format!(" at `{part}`"),
        _ => String::new(),
    };
    match e {
        Error::Parse(p)
            if matches!(p.kind(), ErrorKind::UnsupportedLookAround | ErrorKind::UnsupportedBackreference) =>
        {
            "lookaround and backreferences are not supported in a transform".into()
        }
        Error::Parse(p) => format!("{}{}", p.kind(), at(p.pattern(), p.span())),
        Error::Translate(t) => format!("{}{}", t.kind(), at(t.pattern(), t.span())),
        _ => e.to_string(),
    }
}

/// A transform's replacement text: `$n`, `${n}`, `\n`, `\t`, `\u`, `\l`, `\U`, `\L`, `\E` and
/// `(?n:if:else)`. A backslash before any other punctuation (`\$`, `\\`, `\/`, `\(`, `\)`, `\:` ...)
/// makes it text; before a letter the backslash stays, as in `\item`.
#[derive(Clone, Debug, PartialEq)]
pub struct Format(Vec<Part>);

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Text(String),
    Group(usize),
    /// `\u` or `\l`: the next character.
    Next(Fold),
    /// `\U` or `\L` until `\E` (`None`) or the end of the replacement.
    Run(Option<Fold>),
    /// `(?n:then:otherwise)`: `then` when group `n` took part in the match, even matching nothing.
    If(usize, Format, Format),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fold {
    Upper,
    Lower,
}

/// A replacement being built, with the case change `\u`, `\l`, `\U`, `\L` and `\E` have set.
#[derive(Default)]
struct Out {
    text: String,
    next: Option<Fold>,
    run: Option<Fold>,
}

impl Out {
    fn push(&mut self, s: &str) {
        for c in s.chars() {
            match self.next.take().or(self.run) {
                Some(Fold::Upper) => self.text.extend(c.to_uppercase()),
                Some(Fold::Lower) => self.text.extend(c.to_lowercase()),
                None => self.text.push(c),
            }
        }
    }
}

impl Format {
    fn parse(src: &str) -> Result<Format, String> {
        FormatParser { src, pos: 0 }.parts(&[], 0).map(Format)
    }

    fn write(&self, caps: &Captures, text: &str, out: &mut Out) {
        let group = |n: usize| caps.get_group(n).map_or("", |s| &text[s.range()]);
        for part in &self.0 {
            match part {
                Part::Text(t) => out.push(t),
                Part::Group(n) => out.push(group(*n)),
                Part::Next(fold) => out.next = Some(*fold),
                Part::Run(fold) => out.run = *fold,
                Part::If(n, then, otherwise) => {
                    let taken = if caps.get_group(*n).is_some() { then } else { otherwise };
                    taken.write(caps, text, out);
                }
            }
        }
    }
}

struct FormatParser<'a> {
    src: &'a str,
    pos: usize,
}

impl FormatParser<'_> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    /// The digits at the cursor as a group number (a huge one names no group).
    fn number(&mut self) -> usize {
        let digits = (self.src[self.pos..].bytes()).take_while(u8::is_ascii_digit).count();
        let n = (self.src[self.pos..self.pos + digits]).parse().unwrap_or(usize::MAX);
        self.pos += digits;
        n
    }

    /// Parts up to a `stop` character that is not escaped (left unread), or the end.
    fn parts(&mut self, stop: &[char], depth: usize) -> Result<Vec<Part>, String> {
        let mut out = Vec::new();
        let mut text = String::new();
        while let Some(c) = self.peek() {
            if stop.contains(&c) {
                break;
            }
            self.pos += c.len_utf8();
            // a part ends the text before it; the others add to the text
            let part = match c {
                '\\' => self.escape(&mut text),
                '$' => self.group(&mut text)?,
                '(' => self.conditional(&mut text, depth)?,
                c => {
                    text.push(c);
                    None
                }
            };
            if let Some(part) = part {
                if !text.is_empty() {
                    out.push(Part::Text(std::mem::take(&mut text)));
                }
                out.push(part);
            }
        }
        if !text.is_empty() {
            out.push(Part::Text(text));
        }
        Ok(out)
    }

    /// After a `\`: the case change it names, or the text it stands for.
    fn escape(&mut self, text: &mut String) -> Option<Part> {
        let Some(e) = self.peek() else {
            text.push('\\');
            return None;
        };
        self.pos += e.len_utf8();
        match e {
            'n' => text.push('\n'),
            't' => text.push('\t'),
            'u' => return Some(Part::Next(Fold::Upper)),
            'l' => return Some(Part::Next(Fold::Lower)),
            'U' => return Some(Part::Run(Some(Fold::Upper))),
            'L' => return Some(Part::Run(Some(Fold::Lower))),
            'E' => return Some(Part::Run(None)),
            // `\(` is a paren; before a letter the backslash stays: `\item`
            other => {
                if !other.is_ascii_punctuation() {
                    text.push('\\');
                }
                text.push(other);
            }
        }
        None
    }

    /// After a `$`: a group, `$n` or `${n}`; any other `$` is text.
    fn group(&mut self, text: &mut String) -> Result<Option<Part>, String> {
        match self.peek() {
            Some(d) if d.is_ascii_digit() => Ok(Some(Part::Group(self.number()))),
            Some('{') if self.src[self.pos + 1..].starts_with(|d: char| d.is_ascii_digit()) => {
                self.pos += 1;
                let n = self.number();
                if self.peek() != Some('}') {
                    return Err(format!("expected `}}` after `${{{n}` in the format"));
                }
                self.pos += 1;
                Ok(Some(Part::Group(n)))
            }
            _ => {
                text.push('$');
                Ok(None)
            }
        }
    }

    /// After a `(`: a conditional if `?n:` follows; any other `(` is text.
    fn conditional(&mut self, text: &mut String, depth: usize) -> Result<Option<Part>, String> {
        let rest = &self.src[self.pos..];
        let digits = (rest.strip_prefix('?')).map(|r| r.bytes().take_while(u8::is_ascii_digit).count());
        if !digits.is_some_and(|len| len > 0 && rest[1 + len..].starts_with(':')) {
            text.push('(');
            return Ok(None);
        }
        if depth >= MAX_NESTING {
            return Err(format!("conditionals are nested more than {MAX_NESTING} deep"));
        }
        self.pos += 1;
        let n = self.number();
        self.pos += 1;
        let then = self.parts(&[':', ')'], depth + 1)?;
        let otherwise = match self.peek() {
            Some(':') => {
                self.pos += 1;
                self.parts(&[')'], depth + 1)?
            }
            _ => vec![],
        };
        if self.peek() != Some(')') {
            return Err(format!("unterminated conditional `(?{n}:` in the format"));
        }
        self.pos += 1;
        Ok(Some(Part::If(n, Format(then), Format(otherwise))))
    }
}

/// What follows `$` or `${` in a field: a number or a variable's name.
enum Head {
    Tab(u32),
    Var(String),
}

impl Head {
    fn node(self, default: Vec<Node>) -> Node {
        match self {
            Head::Tab(n) => Node::Tab { n, default },
            Head::Var(name) => Node::Var { name, default, transform: None },
        }
    }

    fn transform(self, re: Regex, fmt: Format, global: bool) -> Node {
        match self {
            Head::Tab(n) => Node::Transform { n, re, fmt, global },
            Head::Var(name) => Node::Var { name, default: vec![], transform: Some((re, fmt, global)) },
        }
    }
}

/// The nodes of a snippet body (the text between the `<content>` tags). A `$` that starts no
/// field or variable is text, as is a `}` outside a field.
pub fn parse(content: &str) -> Result<Vec<Node>, String> {
    Parser { src: content, pos: 0 }.nodes(false, 0)
}

/// Server snippets use the same fields, choices and transforms as local snippets.
pub fn parse_lsp(content: &str) -> Result<Vec<Node>, String> {
    parse(content)
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    /// `msg`, with the line and column (characters, from 1) of byte `at`.
    fn error(&self, at: usize, msg: impl Display) -> String {
        let before = &self.src[..at];
        let line = 1 + before.matches('\n').count();
        let column = 1 + before.rsplit('\n').next().unwrap_or("").chars().count();
        format!("line {line}, column {column}: {msg}")
    }

    fn unbalanced(&self, at: usize) -> String {
        self.error(at, "unbalanced `${`: add the closing `}`, or write `\\$` for a literal `$`")
    }

    /// The digits at the cursor as a tab stop number.
    fn number(&mut self) -> Result<u32, String> {
        let at = self.pos;
        let digits = (self.src[at..].bytes()).take_while(u8::is_ascii_digit).count();
        self.pos += digits;
        self.src[at..self.pos].parse().map_err(|_| self.error(at, "the number is too large for a tab stop"))
    }

    /// The variable name at the cursor.
    fn name(&mut self) -> String {
        let len = (self.src[self.pos..].bytes()).take_while(|b| b.is_ascii_alphanumeric() || *b == b'_').count();
        self.pos += len;
        self.src[self.pos - len..self.pos].to_string()
    }

    /// Nodes up to the `}` that closes the field being read (`nested`; left unread), or the end.
    fn nodes(&mut self, nested: bool, depth: usize) -> Result<Vec<Node>, String> {
        let mut out = Vec::new();
        let mut text = String::new();
        while let Some(c) = self.peek() {
            match c {
                '\\' => {
                    let escaped = (self.src[self.pos + 1..].chars().next()).filter(|e| matches!(e, '$' | '}' | '\\'));
                    match escaped {
                        Some(e) => {
                            text.push(e);
                            self.pos += 2;
                        }
                        // any other backslash is text
                        None => {
                            text.push('\\');
                            self.pos += 1;
                        }
                    }
                }
                '$' => match self.field(depth)? {
                    Some(node) => {
                        if !text.is_empty() {
                            out.push(Node::Text(std::mem::take(&mut text)));
                        }
                        out.push(node);
                    }
                    None => {
                        text.push('$');
                        self.pos += 1;
                    }
                },
                '}' if nested => break,
                c => {
                    text.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
        if !text.is_empty() {
            out.push(Node::Text(text));
        }
        Ok(out)
    }

    /// The field or variable at the `$` under the cursor; `None`, the cursor not moved, when the
    /// `$` starts neither.
    fn field(&mut self, depth: usize) -> Result<Option<Node>, String> {
        let start = self.pos;
        let after = &self.src[start + 1..];
        let (braced, first) = match after.strip_prefix('{') {
            Some(inside) => (true, inside.chars().next()),
            None => (false, after.chars().next()),
        };
        let skip = 1 + usize::from(braced);
        let head = match first {
            Some(c) if c.is_ascii_digit() => {
                self.pos += skip;
                Head::Tab(self.number()?)
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                self.pos += skip;
                Head::Var(self.name())
            }
            _ => return Ok(None),
        };
        if !braced {
            return Ok(Some(head.node(vec![])));
        }
        match self.peek() {
            Some('}') => {
                self.pos += 1;
                Ok(Some(head.node(vec![])))
            }
            Some(':') => {
                if depth >= MAX_NESTING {
                    let msg = format!("fields are nested more than {MAX_NESTING} deep");
                    return Err(self.error(start, msg));
                }
                self.pos += 1;
                let default = self.nodes(true, depth + 1)?;
                if self.peek() != Some('}') {
                    return Err(self.unbalanced(start));
                }
                self.pos += 1;
                Ok(Some(head.node(default)))
            }
            Some('/') => {
                self.pos += 1;
                let (re, fmt, global) = self.transform(start)?;
                Ok(Some(head.transform(re, fmt, global)))
            }
            Some('|') if matches!(head, Head::Tab(_)) => {
                self.pos += 1;
                let mut values = Vec::new();
                let mut value = String::new();
                loop {
                    match self.peek() {
                        Some('|') if self.src[self.pos..].starts_with("|}") => {
                            self.pos += 2;
                            break;
                        }
                        Some(',') => {
                            values.push(std::mem::take(&mut value));
                            self.pos += 1;
                        }
                        Some('\\') => {
                            self.pos += 1;
                            let Some(c) = self.peek().filter(|c| matches!(c, ',' | '|' | '\\')) else {
                                return Err(self.error(self.pos, "invalid choice escape"));
                            };
                            value.push(c);
                            self.pos += c.len_utf8();
                        }
                        Some(c) if c != '|' => {
                            value.push(c);
                            self.pos += c.len_utf8();
                        }
                        _ => return Err(self.unbalanced(start)),
                    }
                }
                values.push(value);
                let Head::Tab(n) = head else { unreachable!() };
                Ok(Some(Node::Choice { n, values }))
            }
            Some('|') => Err(self.error(self.pos, "choices require a numeric tab stop")),
            Some(c) => {
                let msg = format!(
                    "expected `}}`, `:` or `/` after `{}`, found {c:?}; write `\\$` for a literal `$`",
                    &self.src[start..self.pos]
                );
                Err(self.error(self.pos, msg))
            }
            None => Err(self.unbalanced(start)),
        }
    }

    /// `regex/format/options}` after the first `/` of a transform opened at `start`: the options
    /// are `g`, `i` and `m`, then perhaps a `:` and a default that is ignored.
    fn transform(&mut self, start: usize) -> Result<(Regex, Format, bool), String> {
        let regex_at = self.pos;
        let pattern = self.until_slash(start)?;
        let format_at = self.pos;
        let format = self.until_slash(start)?;
        let (mut global, mut ignore_case, mut multi_line) = (false, false, false);
        loop {
            let at = self.pos;
            let Some(c) = self.peek() else {
                return Err(self.unbalanced(start));
            };
            self.pos += c.len_utf8();
            match c {
                'g' => global = true,
                'i' => ignore_case = true,
                'm' => multi_line = true,
                '}' => break,
                // a default after `:` (Sublime's own snippets end some transforms with
                // `:name` or `:mine`) is skipped unread to the first `}`, so one holding a `}`
                // ends early; read it when a snippet needs one
                ':' => {
                    let close = (self.src[self.pos..].find('}')).ok_or_else(|| self.unbalanced(start))?;
                    self.pos += close + 1;
                    break;
                }
                c => {
                    let msg = format!(
                        "unexpected {c:?} in the options of a transform (g, i or m, then `}}`); \
                         is the closing `/` of the format missing?"
                    );
                    return Err(self.error(at, msg));
                }
            }
        }
        let re = Regex::new(&pattern, ignore_case, multi_line)
            .map_err(|e| self.error(regex_at, format!("invalid regex `{pattern}`: {e}")))?;
        let fmt = Format::parse(&format).map_err(|e| self.error(format_at, e))?;
        Ok((re, fmt, global))
    }

    /// The text up to the next `/` that is not escaped, which is read too. `\/` is a `/`; other
    /// escapes stay for the regex or the format to read.
    fn until_slash(&mut self, start: usize) -> Result<String, String> {
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.unbalanced(start));
            };
            self.pos += c.len_utf8();
            match c {
                '/' => return Ok(out),
                '\\' => match self.peek() {
                    Some('/') => {
                        out.push('/');
                        self.pos += 1;
                    }
                    Some(next) => {
                        out.push('\\');
                        out.push(next);
                        self.pos += next.len_utf8();
                    }
                    None => out.push('\\'),
                },
                c => out.push(c),
            }
        }
    }
}

/// `TM_FULLNAME`: the user's full name, the first part of the passwd entry's comment field;
/// empty when there is none.
pub fn full_name() -> String {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let read = || {
        // SAFETY: getpwuid_r writes the entry and its strings into `pwd` and `buf`, which outlive
        // the read of `pw_gecos` below; `found` is null when there is no entry.
        unsafe {
            let mut pwd: libc::passwd = std::mem::zeroed();
            let mut buf = vec![0 as libc::c_char; 16 * 1024];
            let mut found = std::ptr::null_mut();
            let uid = libc::getuid();
            let rc = libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut found);
            if rc != 0 || found.is_null() || pwd.pw_gecos.is_null() {
                return String::new();
            }
            let gecos = std::ffi::CStr::from_ptr(pwd.pw_gecos).to_string_lossy();
            gecos.split(',').next().unwrap_or_default().trim().to_string()
        }
    };
    NAME.get_or_init(read).clone()
}

/// What a place of a field is in an expansion.
#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    /// Where the field is typed.
    Primary,
    /// The primary's text again.
    Mirror,
    /// The primary's text through a transform (`Regex::replace`).
    Transformed(Regex, Format, bool),
}

/// A place of a field in an expansion's text.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub n: u32,
    /// Chars of the expansion's text.
    pub range: Range<usize>,
    pub role: Role,
    /// The primary (an index into `Expansion::fields`) this place is inside.
    pub parent: Option<usize>,
    /// Present only on a choice field's primary; retained independently of its snippet source.
    pub choices: Option<Arc<[String]>>,
}

/// A body expanded for one caret: its text and the places of its fields, in text order (a primary
/// before the places inside it).
#[derive(Clone, Debug, PartialEq)]
pub struct Expansion {
    pub text: String,
    pub fields: Vec<Place>,
}

/// `body` expanded for one caret. `var` gives a variable's value (none or empty takes its
/// default); a line break in the body's own text is followed by `indent`, and a tab in it becomes
/// `tab`.
pub fn expand(body: &[Node], var: &dyn Fn(&str) -> Option<String>, indent: &str, tab: &str) -> Expansion {
    let mut x = Expander {
        var,
        indent,
        tab,
        values: HashMap::new(),
        primary: HashMap::new(),
        texts: HashMap::new(),
        written: HashSet::new(),
        out: Expansion { text: String::new(), fields: Vec::new() },
        chars: 0,
        open: Vec::new(),
    };
    let mut first_empty = HashMap::new();
    x.find_primaries(body, &mut first_empty);
    for (n, node) in first_empty {
        x.primary.entry(n).or_insert(node);
    }
    // the first run learns each field's text, for a mirror written before its primary
    x.write(body);
    x.written.clear();
    (x.out.text, x.out.fields, x.chars) = (String::new(), Vec::new(), 0);
    x.write(body);
    x.out
}

/// One caret's expansion being written.
struct Expander<'a> {
    var: &'a dyn Fn(&str) -> Option<String>,
    indent: &'a str,
    tab: &'a str,
    /// Each variable's value, asked for once.
    values: HashMap<String, String>,
    /// Each field's primary occurrence in the body.
    primary: HashMap<u32, *const Node>,
    /// Each field's text, as its primary was last written.
    texts: HashMap<u32, String>,
    /// The fields whose primary has been written in this run.
    written: HashSet<u32>,
    out: Expansion,
    chars: usize,
    /// The primaries being written (indexes into the places), innermost last.
    open: Vec<usize>,
}

impl Expander<'_> {
    fn value(&mut self, name: &str) -> String {
        let var = self.var;
        (self.values.entry(name.to_string())).or_insert_with(|| var(name).unwrap_or_default()).clone()
    }

    /// Each field's primary: the first of its number with a default, in body order, among the
    /// parts that are written (not a mirror's default, nor that of a variable with a value); else
    /// the first of its number, in `first_empty`.
    fn find_primaries(&mut self, nodes: &[Node], first_empty: &mut HashMap<u32, *const Node>) {
        for node in nodes {
            match node {
                Node::Choice { n, .. } => {
                    self.primary.entry(*n).or_insert(node);
                }
                Node::Tab { n, default } if !default.is_empty() => {
                    if !self.primary.contains_key(n) {
                        self.primary.insert(*n, node);
                        self.find_primaries(default, first_empty);
                    }
                }
                Node::Tab { n, .. } => {
                    first_empty.entry(*n).or_insert(node);
                }
                Node::Var { name, default, transform: None } if self.value(name).is_empty() => {
                    self.find_primaries(default, first_empty)
                }
                _ => {}
            }
        }
    }

    fn push(&mut self, s: &str) {
        self.out.text.push_str(s);
        self.chars += s.chars().count();
    }

    /// The body's own text: a line break takes the indent, a tab the view's indentation.
    fn text(&mut self, t: &str) {
        let (indent, tab) = (self.indent, self.tab);
        for (i, line) in t.split('\n').enumerate() {
            if i > 0 {
                self.push("\n");
                self.push(indent);
            }
            self.push(&line.replace('\t', tab));
        }
    }

    fn place(&mut self, n: u32, range: Range<usize>, role: Role) -> usize {
        let parent = self.open.last().copied();
        (self.out.fields).push(Place { n, range, role, parent, choices: None });
        self.out.fields.len() - 1
    }

    fn write(&mut self, nodes: &[Node]) {
        for node in nodes {
            match node {
                Node::Text(t) => self.text(t),
                Node::Choice { n, values } if self.primary.get(n) == Some(&(node as *const _)) => {
                    let at = self.chars;
                    let i = self.place(*n, at..at, Role::Primary);
                    // Resolve indentation once so choosing the first value reproduces the default.
                    let newline = format!("\n{}", self.indent);
                    let values: Arc<[String]> =
                        values.iter().map(|v| v.replace('\t', self.tab).replace('\n', &newline)).collect();
                    self.push(values.first().map_or("", String::as_str));
                    self.out.fields[i].range.end = self.chars;
                    self.texts.insert(*n, values.first().cloned().unwrap_or_default());
                    self.out.fields[i].choices = Some(values);
                    self.written.insert(*n);
                }
                Node::Choice { n, .. } => self.mirror(*n, Role::Mirror),
                Node::Tab { n, default } if self.primary.get(n) == Some(&(node as *const _)) => {
                    let (at, byte) = (self.chars, self.out.text.len());
                    let i = self.place(*n, at..at, Role::Primary);
                    self.open.push(i);
                    self.write(default);
                    self.open.pop();
                    self.out.fields[i].range.end = self.chars;
                    self.texts.insert(*n, self.out.text[byte..].to_string());
                    self.written.insert(*n);
                }
                Node::Tab { n, .. } => self.mirror(*n, Role::Mirror),
                Node::Transform { n, re, fmt, global } => {
                    self.mirror(*n, Role::Transformed(re.clone(), fmt.clone(), *global))
                }
                Node::Var { name, default, transform } => {
                    let value = self.value(name);
                    match transform {
                        Some((re, fmt, global)) => self.push(&re.replace(fmt, *global, &value)),
                        None if value.is_empty() => self.write(default),
                        None => self.push(&value),
                    }
                }
            }
        }
    }

    /// A mirror of field `n`. Inside a field it follows only a field written before it, not one
    /// it is in: fields then never follow each other round in a circle, and the others show
    /// nothing. One outside every field follows its field wherever that is.
    fn mirror(&mut self, n: u32, role: Role) {
        let live = self.primary.contains_key(&n) && (self.open.is_empty() || self.written.contains(&n));
        let source = match live {
            true => self.texts.get(&n).cloned().unwrap_or_default(),
            false => String::new(),
        };
        let text = match &role {
            Role::Transformed(re, fmt, global) => re.replace(fmt, *global, &source),
            _ => source,
        };
        let at = self.chars;
        self.push(&text);
        if live {
            self.place(n, at..self.chars, role);
        }
    }
}

/// A snippet's fields after an expansion, held by the view that expanded it until the session ends.
// each edit maps every place (carets × places a change); index them if many-caret
// sessions lag.
#[derive(Clone)]
pub struct Session {
    /// Every expanded caret's places, carets in text order, each caret's in its expansion's
    /// order, then that caret's end (field `None`).
    places: Vec<Slot>,
    /// The fields in tab order: 1, 2, …, then 0, then the ends.
    order: Vec<Option<u32>>,
    /// The field being typed in, in `order`.
    current: usize,
    choices_open: bool,
    /// How many of the view's logged changes the places have been brought through.
    pub at: usize,
}

/// An offset in a place of a session (`Session::holder`): the place, how far into it, and whether
/// at its end.
pub type Spot = (usize, usize, bool);

/// A place in the text, as the session keeps it.
#[derive(Clone)]
struct Slot {
    /// The expansion's caret origin, shared by its nested fields, mirrors and final stop.
    rectangular: bool,
    field: Option<u32>,
    range: Range<usize>,
    role: Role,
    parent: Option<usize>,
    /// Past the last place inside this one.
    end: usize,
    /// A mirror's primary.
    source: Option<usize>,
    /// Typed over, or inside a primary that was.
    gone: bool,
    choices: Option<Arc<[String]>>,
}

impl Session {
    /// The session of `expansions`, each with where its text starts, in text order; `at` is how
    /// many changes the view has logged.
    pub fn new(expansions: &[(usize, Expansion)], at: usize) -> Self {
        Self::with_origins(expansions, &[], at)
    }

    /// One origin per expansion; absent origins are ordinary (the range-only API).
    pub fn with_origins(expansions: &[(usize, Expansion)], origins: &[bool], at: usize) -> Self {
        let mut places = Vec::new();
        for (i, (start, e)) in expansions.iter().enumerate() {
            let rectangular = origins.get(i).copied().unwrap_or(false);
            let base = places.len();
            let primary: HashMap<u32, usize> = (e.fields.iter().enumerate())
                .filter(|(_, p)| p.role == Role::Primary)
                .map(|(i, p)| (p.n, base + i))
                .collect();
            places.extend(e.fields.iter().map(|p| Slot {
                rectangular,
                field: Some(p.n),
                range: start + p.range.start..start + p.range.end,
                role: p.role.clone(),
                parent: p.parent.map(|q| base + q),
                end: 0,
                source: (p.role != Role::Primary).then(|| primary.get(&p.n).copied()).flatten(),
                gone: false,
                choices: p.choices.clone(),
            }));
            let end = start + e.text.chars().count();
            places.push(Slot {
                rectangular,
                field: None,
                range: end..end,
                role: Role::Primary,
                parent: None,
                end: 0,
                source: None,
                gone: false,
                choices: None,
            });
        }
        for i in 0..places.len() {
            places[i].end = i + 1;
            let mut up = places[i].parent;
            while let Some(q) = up {
                places[q].end = i + 1;
                up = places[q].parent;
            }
        }
        let mut numbers: Vec<u32> =
            (places.iter()).filter(|p| p.role == Role::Primary).filter_map(|p| p.field).collect();
        numbers.sort_unstable();
        numbers.dedup();
        let zero = numbers.first() == Some(&0);
        let order =
            (numbers.into_iter().filter(|&n| n > 0).map(Some)).chain(zero.then_some(Some(0))).chain([None]).collect();
        Session { places, order, current: 0, choices_open: true, at }
    }

    /// The active primary's alternatives while its list has not been dismissed.
    pub fn choices(&self) -> Option<&Arc<[String]>> {
        if !self.choices_open {
            return None;
        }
        self.places
            .iter()
            .find(|p| !p.gone && p.role == Role::Primary && p.field == self.order[self.current])?
            .choices
            .as_ref()
            .filter(|values| !values.is_empty())
    }

    pub fn close_choices(&mut self) {
        self.choices_open = false;
    }

    /// One choice per expanded primary; plain mirrors are synchronized by the existing session.
    pub fn choice_edits(&self, index: usize) -> Option<Vec<(Range<usize>, String)>> {
        self.choices()?;
        self.places
            .iter()
            .filter(|p| !p.gone && p.role == Role::Primary && p.field == self.order[self.current])
            .map(|p| Some((p.range.clone(), p.choices.as_ref()?.get(index)?.clone())))
            .collect()
    }

    /// The current field is `$0` or the end: the caret ends there, and so does the session.
    pub fn at_end(&self) -> bool {
        self.order[self.current].is_none_or(|n| n == 0)
    }

    /// Whether place `i` is one the current field is typed in: a primary or a plain mirror.
    fn typed_in(&self, i: usize) -> bool {
        let p = &self.places[i];
        !p.gone && p.field == self.order[self.current] && matches!(p.role, Role::Primary | Role::Mirror)
    }

    /// What selects the current field: its primaries and plain mirrors, the first caret's primary
    /// last (the primary selection).
    pub fn selection(&self) -> Vec<Range<usize>> {
        self.selected(|p| p.range.clone())
    }

    /// Current field selections retain their originating expansion even when field counts differ.
    pub fn selections(&self) -> Vec<Selection> {
        self.selected(|p| Selection { rectangular: p.rectangular, ..Selection::new(p.range.start, p.range.end) })
    }

    fn selected<T>(&self, place: impl Fn(&Slot) -> T) -> Vec<T> {
        let mut first = None;
        let mut out = Vec::new();
        for i in (0..self.places.len()).filter(|&i| self.typed_in(i)) {
            match (&self.places[i].role, &first) {
                (Role::Primary, None) => first = Some(place(&self.places[i])),
                _ => out.push(place(&self.places[i])),
            }
        }
        out.extend(first);
        out
    }

    /// Whether `r` lies in a place the current field is typed in.
    pub fn holds(&self, r: Range<usize>) -> bool {
        (0..self.places.len()).any(|i| {
            let p = &self.places[i].range;
            self.typed_in(i) && p.start <= r.start && r.end <= p.end
        })
    }

    /// The place the current field is typed in that holds offset `x` (the first, at a boundary),
    /// how far into it `x` is, and whether at its end.
    pub fn holder(&self, x: usize) -> Option<Spot> {
        let i = (0..self.places.len()).find(|&i| {
            let r = &self.places[i].range;
            self.typed_in(i) && r.start <= x && x <= r.end
        })?;
        let r = &self.places[i].range;
        Some((i, x - r.start, x == r.end))
    }

    /// Where an offset `holder` placed is now: as far into its place, or at its end.
    pub fn held(&self, (i, into, at_end): Spot) -> usize {
        let r = &self.places[i].range;
        match at_end {
            true => r.end,
            false => (r.start + into).min(r.end),
        }
    }

    /// The current field still has a primary.
    pub fn alive(&self) -> bool {
        self.there(self.current)
    }

    /// Whether field `i` of `order` still has a primary.
    fn there(&self, i: usize) -> bool {
        let f = self.order[i];
        (self.places.iter()).any(|p| !p.gone && p.role == Role::Primary && p.field == f)
    }

    /// On to the next field still there, or back to the previous one (at the first, it stays).
    pub fn step(&mut self, forward: bool) {
        let next = match forward {
            true => (self.current + 1..self.order.len()).find(|&i| self.there(i)),
            false => (0..self.current).rev().find(|&i| self.there(i)),
        };
        self.current = next.unwrap_or(self.current);
        self.choices_open = true;
    }

    /// Bring the places through change `c`: the place that took it, `None` when it was not made
    /// in a place the current field is typed in. That place takes what is typed at either end of
    /// it, and so do the primaries it is in; a place inside it keeps what is left of its own
    /// text, and goes when none is, or when the primary it is in went; the places after it move.
    pub fn map(&mut self, c: &crate::buffer::Change) -> Option<usize> {
        let (s, r, typed) = (c.start, c.removed, c.inserted);
        let within = |p: &Range<usize>| p.start <= s && s + r <= p.end;
        let a = (0..self.places.len()).find(|&i| self.typed_in(i) && within(&self.places[i].range))?;
        // only offsets at or past the removed text move
        let shift = |x: usize| x + typed - r;
        self.move_ends(Some(a), shift);
        let inside = a + 1..self.places[a].end;
        for i in a + 1..self.places.len() {
            let parent_gone = (self.places[i].parent).is_some_and(|q| self.places[q].gone);
            let p = &mut self.places[i];
            let (x, y) = (p.range.start, p.range.end);
            if p.gone {
                continue;
            }
            if !inside.contains(&i) {
                p.range = shift(x)..shift(y);
            } else if parent_gone {
                p.gone = true;
            } else if x == y {
                // an empty place typed over goes; one where text was typed stays before it
                match r > 0 && (s..=s + r).contains(&x) {
                    true => p.gone = true,
                    false if x > s => p.range = shift(x)..shift(x),
                    false => {}
                }
            } else {
                let start = match x {
                    x if x < s => x,
                    x if x < s + r || x == s => s + typed,
                    x => shift(x),
                };
                let end = match y {
                    y if y <= s => y,
                    y if y < s + r => s,
                    y => shift(y),
                };
                match start < end {
                    true => p.range = start..end,
                    false => p.gone = true,
                }
            }
        }
        Some(a)
    }

    /// Whether a plain mirror among `took` (the places that took a command's changes) took one
    /// while its primary took none: following the primary again would undo that typing.
    pub fn typed_alone(&self, took: &[usize]) -> bool {
        let mut hit = vec![false; self.places.len()];
        for &i in took {
            hit[i] = true;
        }
        (took.iter().map(|&i| &self.places[i])).any(|p| p.role == Role::Mirror && p.source.is_some_and(|q| !hit[q]))
    }

    /// The mirrors, those inside a primary first and in text order: a mirror's primary is then
    /// up to date when it is reached (`expand` lets one inside a field follow only an earlier
    /// field), and nothing follows a mirror outside every field.
    pub fn mirrors(&self) -> Vec<usize> {
        let mirror = |i: &usize| self.places[*i].source.is_some();
        let (inner, outer): (Vec<usize>, Vec<usize>) =
            (0..self.places.len()).filter(mirror).partition(|&i| self.places[i].parent.is_some());
        inner.into_iter().chain(outer).collect()
    }

    /// What mirror `i` should hold, from its primary's text in `rope`, when it holds something
    /// else.
    pub fn stale(&self, i: usize, rope: &ropey::Rope) -> Option<String> {
        let p = &self.places[i];
        let source = &self.places[p.source?];
        let fits = |r: &Range<usize>| r.end <= rope.len_chars();
        if p.gone || source.gone || !fits(&p.range) || !fits(&source.range) {
            return None;
        }
        let (source, here) = (rope.slice(source.range.clone()), rope.slice(p.range.clone()));
        match &p.role {
            Role::Transformed(re, fmt, global) => {
                let want = re.replace(fmt, *global, &source.to_string());
                (here != want.as_str()).then_some(want)
            }
            // compared in place: a field of a huge selection is not copied at every keystroke
            _ => (here != source).then(|| source.to_string()),
        }
    }

    /// The ends of place `from` and of the primaries it is in moved by `shift`.
    fn move_ends(&mut self, from: Option<usize>, shift: impl Fn(usize) -> usize) {
        let mut up = from;
        while let Some(q) = up {
            self.places[q].range.end = shift(self.places[q].range.end);
            up = self.places[q].parent;
        }
    }

    pub fn range(&self, i: usize) -> Range<usize> {
        self.places[i].range.clone()
    }

    /// Mirror `i`'s text was replaced by `len` chars: the primaries it is in and the places
    /// after it move.
    pub fn replaced(&mut self, i: usize, len: usize) {
        let old = self.places[i].range.clone();
        let shift = |x: usize| x + len - old.len();
        self.places[i].range.end = old.start + len;
        self.move_ends(self.places[i].parent, shift);
        // a gone place's range is stale (never read again): shifting it could underflow
        for p in self.places[i + 1..].iter_mut().filter(|p| !p.gone) {
            p.range = shift(p.range.start)..shift(p.range.end);
        }
    }
}

/// A snippet: what a `.sublime-snippet` file says.
#[derive(Clone, Debug, PartialEq)]
pub struct Snippet {
    /// The word to type before Tab; empty when there is none.
    pub trigger: String,
    /// A scope selector as written, trimmed (`selector`); empty applies everywhere.
    pub scope: String,
    pub description: String,
    pub body: Vec<Node>,
    pub path: PathBuf,
}

/// What a snippet file says, before its body is parsed.
#[derive(Default)]
struct Fields {
    content: Option<String>,
    trigger: Option<String>,
    scope: Option<String>,
    description: Option<String>,
}

/// The start of `s` for a message, so a long run of garbage stays short.
fn glimpse(s: &str) -> String {
    let mut out: String = s.chars().take(24).collect();
    if s.chars().nth(24).is_some() {
        out.push('…');
    }
    out
}

fn is_name_byte(b: &u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':')
}

/// The elements of a snippet file that are read.
const ELEMENTS: [&str; 4] = ["content", "tabTrigger", "scope", "description"];

/// The element of ours that `name` is a typo of, one edit from it ignoring case. Skipped as some
/// other element, a misspelled `<scope>` would leave the snippet firing in every file.
fn near_miss(name: &str) -> Option<&'static str> {
    let name: Vec<char> = name.to_lowercase().chars().collect();
    ELEMENTS.into_iter().find(|ours| {
        let ours: Vec<char> = ours.to_lowercase().chars().collect();
        one_edit(&name, &ours)
    })
}

/// Whether `a` is `b`, or `b` with a character added, dropped or changed, or two next to each
/// other swapped.
fn one_edit(a: &[char], b: &[char]) -> bool {
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let same = short.iter().zip(long).take_while(|(x, y)| x == y).count();
    if same == short.len() {
        return true;
    }
    if long.len() > short.len() {
        return short[same..] == long[same + 1..];
    }
    short[same + 1..] == long[same + 1..]
        || (short[same] == long[same + 1] && short[same + 1] == long[same] && short[same + 2..] == long[same + 2..])
}

/// A reader for the little XML a snippet file is: `<snippet>` holding `<content>`,
/// `<tabTrigger>`, `<scope>` and `<description>`, with CDATA sections, comments, the five
/// entities and numeric references. Attributes on those, and anything but text in them, are
/// errors; other children of `<snippet>` are skipped, as Sublime ignores them, but must be well
/// formed. (No XML crate is in the build; this is all a snippet file uses.)
struct Reader<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Reader<'a> {
    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn error(&self, at: usize, msg: impl Display) -> String {
        let line = 1 + self.src[..at].matches('\n').count();
        format!("XML error at line {line}: {msg}")
    }

    /// Past whitespace, comments and `<? … ?>` declarations.
    fn skip_misc(&mut self) -> Result<(), String> {
        loop {
            self.pos += self.rest().len() - self.rest().trim_start().len();
            if self.rest().starts_with("<!--") {
                self.comment()?;
            } else if self.rest().starts_with("<?") {
                self.instruction()?;
            } else {
                return Ok(());
            }
        }
    }

    /// Past the `<? … ?>` at the cursor.
    fn instruction(&mut self) -> Result<(), String> {
        let end = (self.rest().find("?>")).ok_or_else(|| self.error(self.pos, "unterminated `<?`"))?;
        self.pos += end + 2;
        Ok(())
    }

    /// Past the comment at the cursor.
    fn comment(&mut self) -> Result<(), String> {
        let end = (self.rest()[4..].find("-->")).ok_or_else(|| self.error(self.pos, "unterminated comment"))?;
        self.pos += 4 + end + 3;
        Ok(())
    }

    /// Past the CDATA section at the cursor: its text. `None` when the cursor is not at one.
    fn cdata(&mut self) -> Result<Option<&'a str>, String> {
        let Some(cdata) = self.rest().strip_prefix("<![CDATA[") else {
            return Ok(None);
        };
        let end = (cdata.find("]]>")).ok_or_else(|| self.error(self.pos, "unterminated CDATA section"))?;
        self.pos += "<![CDATA[".len() + end + "]]>".len();
        Ok(Some(&cdata[..end]))
    }

    /// The length of the name of the start tag at the cursor, whose `<` is before `after`; an
    /// error when it has none.
    fn name_len(&self, after: &str) -> Result<usize, String> {
        match after.bytes().take_while(is_name_byte).count() {
            0 => Err(self.error(self.pos, format!("unexpected `{}`", glimpse(self.rest())))),
            len => Ok(len),
        }
    }

    /// Past the start tag at the cursor, `<name>` or `<name/>`: its name and whether it is empty.
    /// `None` when the cursor is not at a `<`.
    fn open_tag(&mut self) -> Result<Option<(&'a str, bool)>, String> {
        let at = self.pos;
        let Some(after) = self.rest().strip_prefix('<') else {
            return Ok(None);
        };
        let len = self.name_len(after)?;
        let name = &after[..len];
        let tail = after[len..].trim_start();
        let blanks = after[len..].len() - tail.len();
        let (empty, end) = if tail.starts_with("/>") {
            (true, 2)
        } else if tail.starts_with('>') {
            (false, 1)
        } else if blanks > 0 && tail.bytes().next().is_some_and(|b| is_name_byte(&b)) {
            return Err(self.error(at, format!("attributes are not supported (in <{name}>)")));
        } else {
            return Err(self.error(at, format!("malformed <{name}> tag")));
        };
        self.pos += 1 + len + blanks + end;
        Ok(Some((name, empty)))
    }

    /// The name of the element whose start tag is at the cursor; empty when there is none.
    fn tag_name(&self) -> &'a str {
        let name = self.rest().strip_prefix('<').unwrap_or("");
        &name[..name.bytes().take_while(is_name_byte).count()]
    }

    /// Past the start tag at the cursor, attributes and all: its name and whether it is empty.
    fn loose_open_tag(&mut self) -> Result<(&'a str, bool), String> {
        let at = self.pos;
        let after = &self.rest()[1..];
        let len = self.name_len(after)?;
        // the tag ends at the first `>` that is not in the quotes of an attribute's value
        let mut quote = None;
        for (i, c) in after[len..].char_indices() {
            match (quote, c) {
                (None, '"' | '\'') => quote = Some(c),
                (Some(q), c) if c == q => quote = None,
                (None, '>') => {
                    let empty = after[..len + i].ends_with('/');
                    self.pos += 1 + len + i + 1;
                    return Ok((&after[..len], empty));
                }
                _ => {}
            }
        }
        let name = glimpse(&after[..len]);
        Err(self.error(at, format!("<{name}> start tag is never closed")))
    }

    /// Past the element at the cursor and all it holds, an element that is none of ours. It is
    /// not read, but it must be well formed: tags paired, entities known, CDATA sections and
    /// comments closed.
    fn skip_element(&mut self) -> Result<(), String> {
        let mut open: Vec<&'a str> = Vec::new();
        loop {
            let at = self.pos;
            let rest = self.rest();
            let Some(c) = rest.chars().next() else {
                let name = glimpse(open.last().copied().unwrap_or(""));
                return Err(self.error(at, format!("<{name}> is never closed")));
            };
            if rest.starts_with("<!--") {
                self.comment()?;
            } else if rest.starts_with("<![CDATA[") {
                self.cdata()?;
            } else if rest.starts_with("<?") {
                self.instruction()?;
            } else if rest.starts_with("</") {
                self.pos += 2;
                let name = open.pop().unwrap_or("");
                self.close_tag(name)?;
                if open.is_empty() {
                    return Ok(());
                }
            } else if c == '<' {
                let (name, empty) = self.loose_open_tag()?;
                match (empty, open.is_empty()) {
                    (false, _) => open.push(name),
                    (true, true) => return Ok(()),
                    (true, false) => {}
                }
            } else if c == '&' {
                self.entity()?;
            } else {
                self.pos += c.len_utf8();
            }
        }
    }

    /// Past the end tag, its `</` already read, which must close `name`.
    fn close_tag(&mut self, name: &str) -> Result<(), String> {
        let at = self.pos;
        let rest = self.rest();
        let len = rest.bytes().take_while(is_name_byte).count();
        let tail = rest[len..].trim_start();
        if &rest[..len] != name {
            let found = glimpse(&rest[..len]);
            return Err(self.error(at, format!("expected </{name}>, found </{found}>")));
        }
        if !tail.starts_with('>') {
            return Err(self.error(at, format!("malformed </{name}> tag")));
        }
        self.pos += rest.len() - tail.len() + 1;
        Ok(())
    }

    /// The text of the element `name` whose start tag was read, through its end tag: character
    /// data, CDATA sections and entities, comments left out.
    fn text(&mut self, name: &str) -> Result<String, String> {
        let mut out = String::new();
        loop {
            let at = self.pos;
            let rest = self.rest();
            let Some(c) = rest.chars().next() else {
                return Err(self.error(at, format!("<{name}> is never closed")));
            };
            if let Some(cdata) = self.cdata()? {
                out.push_str(cdata);
            } else if rest.starts_with("<!--") {
                self.comment()?;
            } else if rest.starts_with("</") {
                self.pos += 2;
                self.close_tag(name)?;
                return Ok(out);
            } else if c == '<' {
                let msg = format!("unexpected `{}` in <{name}>; put text with a < in a CDATA section", glimpse(rest));
                return Err(self.error(at, msg));
            } else if c == '&' {
                out.push(self.entity()?);
            } else {
                out.push(c);
                self.pos += c.len_utf8();
            }
        }
    }

    /// The character the entity or numeric reference at the cursor stands for.
    fn entity(&mut self) -> Result<char, String> {
        let at = self.pos;
        let Some(end) = self.rest().find(';').filter(|&e| e <= 12) else {
            let msg = "a `&` starts an entity: write &amp; for a literal &, or use &lt; &gt; &quot; &apos; &#N;";
            return Err(self.error(at, msg));
        };
        let name = &self.rest()[1..end];
        let c = match name {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => match name.strip_prefix('#') {
                Some(number) => reference(number)
                    .ok_or_else(|| self.error(at, format!("bad character reference &{};", glimpse(name))))?,
                None => {
                    return Err(self.error(at, format!("unknown entity &{};", glimpse(name))));
                }
            },
        };
        self.pos += end + 1;
        Ok(c)
    }
}

/// The character `#65` or `#x41` (a reference without its `&` and `;`) stands for; none for NUL,
/// a surrogate, a number past U+10FFFF or text that is not a number.
fn reference(number: &str) -> Option<char> {
    let code = match number.strip_prefix(['x', 'X']) {
        Some(hex) if !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
            u32::from_str_radix(hex, 16).ok()?
        }
        None if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) => number.parse().ok()?,
        _ => return None,
    };
    char::from_u32(code).filter(|&c| c != '\0')
}

fn read_fields(xml: &str) -> Result<Fields, String> {
    // line ends are \n to a reader, whatever the file had
    let src = (xml.strip_prefix('\u{feff}').unwrap_or(xml)).replace("\r\n", "\n").replace('\r', "\n");
    let mut r = Reader { src: &src, pos: 0 };
    r.skip_misc()?;
    let at = r.pos;
    let Some((root, empty)) = r.open_tag()? else {
        let found = match r.rest().is_empty() {
            true => "the end of the file",
            false => "text",
        };
        return Err(r.error(at, format!("expected <snippet>, found {found}")));
    };
    if root != "snippet" {
        return Err(r.error(at, format!("expected <snippet>, found <{}>", glimpse(root))));
    }
    let mut fields = Fields::default();
    let mut open = !empty;
    while open {
        r.skip_misc()?;
        let at = r.pos;
        if r.rest().is_empty() {
            return Err(r.error(at, "<snippet> is never closed"));
        }
        if r.rest().starts_with("</") {
            r.pos += 2;
            r.close_tag("snippet")?;
            open = false;
            continue;
        }
        let name = r.tag_name();
        let slot = match name {
            "content" => &mut fields.content,
            "tabTrigger" => &mut fields.trigger,
            "scope" => &mut fields.scope,
            "description" => &mut fields.description,
            // text, or a `<` that opens no element, which open_tag says
            "" => {
                r.open_tag()?;
                return Err(r.error(at, "unexpected text in <snippet>"));
            }
            // not ours: Sublime ignores it, unless it is one of ours misspelled
            _ => {
                if let Some(ours) = near_miss(name) {
                    let msg = format!("unexpected <{}> in <snippet>: did you mean `<{ours}>`?", glimpse(name));
                    return Err(r.error(at, msg));
                }
                r.skip_element()?;
                continue;
            }
        };
        if slot.is_some() {
            return Err(r.error(at, format!("<{name}> appears more than once")));
        }
        let (_, blank) = r.open_tag()?.expect("tag_name found a start tag");
        *slot = Some(if blank { String::new() } else { r.text(name)? });
    }
    r.skip_misc()?;
    if !r.rest().is_empty() {
        return Err(r.error(r.pos, "unexpected content after </snippet>"));
    }
    Ok(fields)
}

/// A newline right after the opening of the content and one right before its end are the file's
/// layout (`<![CDATA[`, a line break, the text, a line break, `]]>`), not part of the snippet.
fn trim_layout(content: &str) -> &str {
    let content = content.strip_prefix('\n').unwrap_or(content);
    content.strip_suffix('\n').unwrap_or(content)
}

/// The snippet a file's text describes.
fn from_xml(xml: &str, path: &Path) -> Result<Snippet, String> {
    let fields = read_fields(xml)?;
    let content = fields.content.ok_or("no <content> element")?;
    let scope = fields.scope.unwrap_or_default().trim().to_string();
    // highlight::matches takes an invalid selector for one that matches nothing, silently
    Selector::parse(&scope).map_err(|e| format!("<scope> {scope:?} is not a valid scope selector: {e}"))?;
    let body = parse(trim_layout(&content)).map_err(|e| format!("<content> {e}"))?;
    Ok(Snippet {
        trigger: fields.trigger.unwrap_or_default().trim().to_string(),
        scope,
        description: fields.description.unwrap_or_default().trim().to_string(),
        body,
        path: path.to_path_buf(),
    })
}

fn read_snippet(path: &Path) -> Result<Snippet, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("cannot read the file: {e}"))?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(format!("the file is larger than {} KiB", MAX_FILE >> 10));
    }
    let text = String::from_utf8(bytes).map_err(|_| "the file is not valid UTF-8".to_string())?;
    from_xml(&text, path)
}

fn is_snippet(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXT))
}

/// The `.sublime-snippet` files under `dir`, folder by folder in name order; hidden names (a dot
/// first) are left out, and a folder that cannot be listed is a problem. A linked file is read; a
/// linked folder is not walked, as it could loop.
fn walk(dir: &Path, files: &mut Vec<PathBuf>, problems: &mut Vec<(PathBuf, String)>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            problems.push((dir.to_path_buf(), format!("cannot list the folder: {e}")));
            return;
        }
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_cached_key(|e| (e.file_name().to_string_lossy().to_lowercase(), e.file_name()));
    for entry in entries {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk(&path, files, problems),
            Ok(kind) if (kind.is_file() || (kind.is_symlink() && path.is_file())) && is_snippet(&path) => {
                files.push(path);
            }
            _ => {}
        }
    }
}

fn read_all(files: &[PathBuf], mut problems: Vec<(PathBuf, String)>) -> (Vec<Snippet>, Vec<(PathBuf, String)>) {
    let mut snippets = Vec::new();
    for path in files {
        match read_snippet(path) {
            Ok(snippet) => snippets.push(snippet),
            Err(why) => problems.push((path.clone(), why)),
        }
    }
    (snippets, problems)
}

/// Every snippet under `dir` (a missing folder has none), and each file left out with the
/// reason: malformed XML, no `<content>`, an unbalanced `${`, an invalid transform regex or
/// `<scope>` selector, a file that cannot be read. (The store reads the folder itself, keeping
/// each file's stamp.)
#[cfg(any(test, feature = "test-support"))]
pub fn load(dir: &Path) -> (Vec<Snippet>, Vec<(PathBuf, String)>) {
    let (mut files, mut problems) = (Vec::new(), Vec::new());
    walk(dir, &mut files, &mut problems);
    read_all(&files, problems)
}

/// The snippets of a folder, read again when its listing or a file's stamp changes.
pub struct Snippets {
    dir: PathBuf,
    list: Vec<Snippet>,
    problems: Vec<(PathBuf, String)>,
    /// Each snippet file and its stamp as the last load found them.
    seen: Option<Vec<(PathBuf, Option<Stamp>)>>,
    checked: Option<Instant>,
}

impl Snippets {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, list: Vec::new(), problems: Vec::new(), seen: None, checked: None }
    }

    /// Looks at the folder, at most once a second, and loads it again if it changed (the first
    /// look always loads). Whether it loaded: the moment for one alert listing `errors()`.
    pub fn refresh(&mut self) -> bool {
        self.refresh_at(Instant::now())
    }

    fn refresh_at(&mut self, now: Instant) -> bool {
        if (self.checked).is_some_and(|t| now.saturating_duration_since(t) < CHECK_EVERY) {
            return false;
        }
        self.checked = Some(now);
        // the folder is walked, and read again when it changed, on the caller's thread
        // (the UI's, from Tab, every list opening and a snippet expanded by name); cheap for
        // hundreds of files, move it off the UI thread if users keep thousands
        let (mut files, mut problems) = (Vec::new(), Vec::new());
        walk(&self.dir, &mut files, &mut problems);
        let seen: Vec<_> = files.iter().map(|p| (p.clone(), stamp(p))).collect();
        if self.seen.as_ref() == Some(&seen) {
            return false;
        }
        (self.list, self.problems) = read_all(&files, problems);
        self.seen = Some(seen);
        true
    }

    /// In folder and name order.
    pub fn snippets(&self) -> &[Snippet] {
        &self.list
    }

    /// An explicit template confirmation must see current contents, even within the refresh
    /// throttle, and may only name a file still in this local catalog.
    pub fn reread(&self, path: &Path) -> Result<Snippet, String> {
        let (mut files, mut problems) = (Vec::new(), Vec::new());
        walk(&self.dir, &mut files, &mut problems);
        if !files.iter().any(|p| p == path) {
            return Err("That template is no longer in the Snippets folder".into());
        }
        read_snippet(path)
    }

    /// The files the last load left out, with the reason.
    pub fn errors(&self) -> &[(PathBuf, String)] {
        &self.problems
    }

    /// The snippets whose tab trigger is `trigger` and whose scope matches `scope`: indexes into
    /// `snippets()`, valid until the next load.
    pub fn with_trigger(&self, trigger: &str, matches: impl Fn(&str) -> bool) -> Vec<usize> {
        self.matching(matches, |s| !trigger.is_empty() && s.trigger == trigger)
    }

    /// The snippets whose scope matches `scope`, whether they have a trigger or not.
    pub fn for_scope(&self, matches: impl Fn(&str) -> bool) -> Vec<usize> {
        self.matching(matches, |_| true)
    }

    fn matching(&self, matches: impl Fn(&str) -> bool, keep: impl Fn(&Snippet) -> bool) -> Vec<usize> {
        (self.list.iter().enumerate())
            .filter(|(_, s)| keep(s) && (s.scope.is_empty() || matches(&s.scope)))
            .map(|(i, _)| i)
            .collect()
    }
}

/// What New Snippet opens: a working snippet, every part explained.
pub fn template() -> &'static str {
    r##"<snippet>
	<content><![CDATA[Hello, ${1:this} is a ${2:snippet}.$0]]></content>
	<!-- tabTrigger: type these characters and press Tab to expand the snippet. Remove the
	     line to offer the snippet in Insert Snippet and the completion list only. -->
	<tabTrigger>hello</tabTrigger>
	<!-- scope: a scope selector that limits where the snippet applies, such as source.rust
	     or text.html - source.js. Leave it out to apply everywhere. -->
	<!-- <scope>source.rust</scope> -->
	<!-- description: shown beside the snippet in the lists. -->
	<description>Hello snippet</description>
	<!--
	In the content (a newline right after the opening CDATA bracket and one right before the
	closing one are layout, not text; the rest is inserted as written):
		$1 $2 ...              tab stops; Tab moves to the next one, Shift-Tab back
		$0                     where the caret ends (the end of the text when left out)
		${1:text}              a tab stop that starts with this text selected; they nest: ${1:a ${2:b}}
		$1 again               a mirror: it repeats what is typed at the first $1
		${1/regex/format/gim}  a mirror run through a regex (g every match, i ignore case, m ^ and $
		                       at every line); in the format $1 or ${1} is a group, \u \l change the
		                       next letter, \U \L ... \E a run, \n \t, (?1:if:else) a conditional
		${NAME} ${NAME:text}   a variable, with text for when it is empty: SELECTION, TM_SELECTED_TEXT,
		                       TM_CURRENT_LINE, TM_CURRENT_WORD, TM_FILENAME, TM_FILEPATH,
		                       TM_DIRECTORY, TM_LINE_INDEX, TM_LINE_NUMBER, TM_SOFT_TABS,
		                       TM_TAB_SIZE, TM_FULLNAME, CURRENT_YEAR, CURRENT_MONTH, CURRENT_DATE,
		                       CURRENT_HOUR, CURRENT_MINUTE, CURRENT_SECOND, CURRENT_SECONDS_UNIX,
		                       CLIPBOARD, WORKSPACE_NAME, WORKSPACE_FOLDER, RELATIVE_FILEPATH
		\$ \} \\               a literal $, } or backslash
	-->
</snippet>
"##
}

#[cfg(test)]
mod tests {
    use super::*;
    use orlo_syntax::highlight;

    fn t(s: &str) -> Node {
        Node::Text(s.into())
    }
    fn tab(n: u32) -> Node {
        Node::Tab { n, default: vec![] }
    }
    fn field(n: u32, default: Vec<Node>) -> Node {
        Node::Tab { n, default }
    }
    fn var(name: &str, default: Vec<Node>) -> Node {
        Node::Var { name: name.into(), default, transform: None }
    }
    fn nodes(src: &str) -> Vec<Node> {
        parse(src).unwrap_or_else(|e| panic!("{src:?} failed: {e}"))
    }
    fn error(src: &str) -> String {
        match parse(src) {
            Err(e) => e,
            Ok(n) => panic!("{src:?} parsed as {n:?}"),
        }
    }
    /// `src` is one `${1/…/…/…}`: that transform applied to `text`.
    /// Numbers below the one asked for, the same ones for a seed.
    fn random(mut state: u64) -> impl FnMut(usize) -> usize {
        move |below| {
            state = (state.wrapping_mul(6364136223846793005)).wrapping_add(1442695040888963407);
            (state >> 33) as usize % below
        }
    }

    fn transformed(src: &str, text: &str) -> String {
        match nodes(src).as_slice() {
            [Node::Transform { n: 1, re, fmt, global }] => re.replace(fmt, *global, text),
            other => panic!("{src:?} is not one transform of 1: {other:?}"),
        }
    }

    // ---- the body parser ----

    #[test]
    fn text_without_a_dollar_is_one_node() {
        let code = "fn main() {\n\tprintln!(\"hi\");\n}";
        assert_eq!(nodes(code), [t(code)]);
        assert!(nodes("").is_empty());
    }

    #[test]
    fn tab_stops() {
        assert_eq!(nodes("a$1b${2}c$0"), [t("a"), tab(1), t("b"), tab(2), t("c"), tab(0)]);
        assert_eq!(nodes("$12 ${345}"), [tab(12), t(" "), tab(345)]);
        assert_eq!(nodes("$1abc"), [tab(1), t("abc")]);
    }

    #[test]
    fn placeholders_nest_and_mirror() {
        assert_eq!(nodes("${1:text}"), [field(1, vec![t("text")])]);
        assert_eq!(nodes("${1:}"), [field(1, vec![])]);
        assert_eq!(nodes("${1:a:b/c}"), [field(1, vec![t("a:b/c")])]);
        assert_eq!(nodes("${1:a ${2:b} c}"), [field(1, vec![t("a "), field(2, vec![t("b")]), t(" c")])]);
        assert_eq!(nodes("${1:${2:${3:x}}}"), [field(1, vec![field(2, vec![field(3, vec![t("x")])])])]);
        assert_eq!(nodes("${1:foo} $1 ${1}"), [field(1, vec![t("foo")]), t(" "), tab(1), t(" "), tab(1)]);
        assert_eq!(nodes("${1:$SELECTION}"), [field(1, vec![var("SELECTION", vec![])])]);
        assert_eq!(nodes("${1:$2}"), [field(1, vec![tab(2)])]);
        // braces inside a placeholder: { is text, \} is an escape
        assert_eq!(nodes(r"${1:{a\}}"), [field(1, vec![t("{a}")])]);
    }

    #[test]
    fn variables() {
        assert_eq!(nodes("$TM_FILENAME"), [var("TM_FILENAME", vec![])]);
        assert_eq!(nodes("${TM_FILENAME}.rs"), [var("TM_FILENAME", vec![]), t(".rs")]);
        assert_eq!(nodes("${NAME:no name}"), [var("NAME", vec![t("no name")])]);
        assert_eq!(nodes("${SELECTION:a ${1:b}}"), [var("SELECTION", vec![t("a "), field(1, vec![t("b")])])]);
        // letters, digits and _; any such name is a variable (an unknown one expands to nothing)
        assert_eq!(nodes("$_a1-b"), [var("_a1", vec![]), t("-b")]);
        assert_eq!(nodes("$this->x"), [var("this", vec![]), t("->x")]);
    }

    #[test]
    fn escapes() {
        assert_eq!(nodes(r"\$1 \$"), [t("$1 $")]);
        assert_eq!(nodes(r"a\}b\\c"), [t("a}b\\c")]);
        // an escaped backslash does not escape the $ after it
        assert_eq!(nodes(r"\\$1"), [t("\\"), tab(1)]);
        assert_eq!(nodes(r"\${1:x"), [t("${1:x")]);
        assert_eq!(nodes(r"${1:a\}b\$c}"), [field(1, vec![t("a}b$c")])]);
        // any other backslash is text
        assert_eq!(nodes(r"\n\t\{ \"), [t(r"\n\t\{ \")]);
    }

    #[test]
    fn a_dollar_that_starts_nothing_is_literal() {
        assert_eq!(nodes("$"), [t("$")]);
        assert_eq!(nodes("cost: $ 5 $- $( $$ $"), [t("cost: $ 5 $- $( $$ $")]);
        assert_eq!(nodes("${ x} ${} ${-1} ${:x}"), [t("${ x} ${} ${-1} ${:x}")]);
        assert_eq!(nodes("$$1"), [t("$"), tab(1)]);
        // braces outside a field are text; backticks too, as no shell is run
        assert_eq!(nodes("a } {b} `date`"), [t("a } {b} `date`")]);
        assert_eq!(nodes("`echo $1`"), [t("`echo "), tab(1), t("`")]);
        // Alternatives belong only to numeric fields, not variables.
        assert!(error("${NAME|a,b|}").contains("numeric"));
    }

    #[test]
    fn n31_choices_parse_literal_values_and_reject_broken_delimiters() {
        let source = r"${1|a\,b,c\|d,e\\f,$2,{x},界,|}";
        assert_eq!(
            nodes(source),
            [Node::Choice {
                n: 1,
                values: vec![
                    "a,b".into(),
                    "c|d".into(),
                    r"e\f".into(),
                    "$2".into(),
                    "{x}".into(),
                    "界".into(),
                    "".into(),
                ]
            }]
        );
        assert_eq!(parse_lsp(source).unwrap(), nodes(source));
        assert_eq!(expanded("$1 ${1||} $1", &[]).text, "  ");
        for source in [r"${1|a\q,b|}", r"${1|a\", "${1|a,b}", "${1|a|b|}"] {
            assert!(parse(source).is_err(), "{source}");
            assert!(parse_lsp(source).is_err(), "{source}");
        }
    }

    #[test]
    fn transforms_are_parsed_on_tab_stops_and_variables() {
        assert!(matches!(&nodes("${3/a/b/g}")[..], [Node::Transform { n: 3, global: true, .. }]));
        assert!(matches!(
            &nodes("x${1/a/b/}y")[..],
            [Node::Text(_), Node::Transform { n: 1, global: false, .. }, Node::Text(_)]
        ));
        let parsed = nodes(r"${TM_FILENAME/(.+)\..+/$1/}");
        let [Node::Var { name, default, transform: Some((re, fmt, global)) }] = &parsed[..] else {
            panic!("{parsed:?}");
        };
        assert_eq!(name, "TM_FILENAME");
        assert!(default.is_empty() && !global);
        assert_eq!(re.replace(fmt, *global, "main.rs"), "main");
        // a variable with a default has no transform and the other way round
        assert!(matches!(&nodes("${A:x}")[..], [Node::Var { transform: None, .. }]));
    }

    #[test]
    fn the_regex_options_are_g_i_and_m() {
        assert_eq!(transformed("${1/a/b/}", "aaa"), "baa");
        assert_eq!(transformed("${1/a/b/g}", "aaa"), "bbb");
        assert_eq!(transformed("${1/A/x/}", "a"), "a");
        assert_eq!(transformed("${1/A/x/i}", "a"), "x");
        // ^ is the start of the text, and of every line with m
        assert_eq!(transformed("${1/^/> /g}", "a\nb"), "> a\nb");
        assert_eq!(transformed("${1/^/> /gm}", "a\nb"), "> a\n> b");
        assert_eq!(transformed("${1/$/;/gm}", "a\nb"), "a;\nb;");
        assert_eq!(transformed("${1/x*/-/g}", "ab"), "-a-b-");
        // any order, repeated
        assert_eq!(transformed("${1/a/b/mig}", "aA"), "bb");
        assert_eq!(transformed("${1/a/b/gg}", "aa"), "bb");
        // a `:` starts a default, which Sublime's own snippets leave in (`:name`, `:mine`): it is
        // skipped to the `}` and sets no option
        assert_eq!(transformed("${1/a/b/:name}", "a"), "b");
        assert_eq!(transformed("${1/a/b/:big}", "aAa"), "bAa");
        assert_eq!(transformed("${1/A/x/g:mine}", "aAA"), "axx");
        assert_eq!(transformed("${1/^/> /g:mine}", "a\nb"), "> a\nb");
    }

    #[test]
    fn after_the_format_only_g_i_m_and_a_default_may_come_before_the_brace() {
        for (src, bad) in [
            ("${1/a/b/x}", 'x'),
            ("${1/a/b/gx}", 'x'),
            ("${1/a/b/ g}", ' '),
            ("${1/a/b/G}", 'G'),
            ("${1/a/b/g,i}", ','),
            ("${1/a/b/gi/}", '/'),
            ("${TM_FILENAME/a/b/x}", 'x'),
        ] {
            let e = error(src);
            assert!(e.contains(&format!("unexpected {bad:?} in the options of a transform")), "{src}: {e}");
        }
        // a closing slash left out: the format ends at the first / after it, and the text after
        // must not be swallowed whole as options
        let e = error(r"${1/(\w+)/\u$1} // c ${2:x}");
        assert!(e.starts_with("line 1, column 18: "), "{e}");
        assert!(e.contains("unexpected '/'") && e.contains("closing `/`"), "{e}");
        assert!(error("<b>${1/x/y}</b>").contains("unexpected 'b'"));
        assert!(error("${1/x/y} b/ ${2:z}").contains("unexpected ' '"));
        // a default that never ends is an unclosed field
        assert!(error("${1/a/b/:name").contains("unbalanced `${`"));
    }

    #[test]
    fn slashes_in_a_transform_are_escaped() {
        assert_eq!(transformed(r"${1/a\/b/x/}", "a/b"), "x");
        assert_eq!(transformed(r"${1/a/x\/y/}", "a"), "x/y");
        // an escaped backslash, then the slash that ends the regex
        assert_eq!(transformed(r"${1/\\/\//}", r"a\b"), "a/b");
        assert_eq!(transformed(r"${1/[^\/]+$//}", "a/b"), "a/");
        // a } in the format is text; only the options end at one
        assert_eq!(transformed("${1/a/}/}", "a"), "}");
        assert_eq!(transformed("${1/a//g}", "banana"), "bnn");
    }

    #[test]
    fn formats_take_groups() {
        assert_eq!(transformed(r"${1/\w+/<$0>/}", "ab cd"), "<ab> cd");
        assert_eq!(transformed(r"${1/\w+/<$0>/g}", "ab cd"), "<ab> <cd>");
        assert_eq!(transformed("${1/(a)(b)/$2$1/}", "ab"), "ba");
        // ${1}2 is group 1 then 2; $12 is group 12, which is empty
        assert_eq!(transformed("${1/(a)/${1}2/}", "a"), "a2");
        assert_eq!(transformed("${1/(a)/$12/}", "a"), "");
        // a group that is not there or did not take part is empty
        assert_eq!(transformed("${1/a/[$3]/}", "a"), "[]");
        assert_eq!(transformed("${1/(a)|(b)/[$1$2]/g}", "ab"), "[a][b]");
    }

    #[test]
    fn formats_take_escapes() {
        assert_eq!(transformed(r"${1/,/\n/g}", "a,b"), "a\nb");
        assert_eq!(transformed(r"${1/,/\t/}", "a,b"), "a\tb");
        assert_eq!(transformed(r"${1/a/\\/}", "a"), "\\");
        assert_eq!(transformed(r"${1/a/\$1/}", "a"), "$1");
        // a backslash before punctuation is that character, before a letter it stays
        assert_eq!(transformed(r"${1/a/\(\)\.\_\-\*/}", "a"), "()._-*");
        // so a paren can be written in a condition, as Sublime's own snippets do
        assert_eq!(transformed(r"${1/(^.*\S.*)|.*/(?1:\()/}", "args"), "(");
        assert_eq!(transformed(r"${1/(^.*\S.*)|.*/(?1:\()/}", ""), "");
        assert_eq!(transformed(r"${1/a/\d\item/}", "a"), r"\d\item");
        assert_eq!(transformed("${1/a/$x US$/}", "a"), "$x US$");
        assert_eq!(transformed("${1/a/(x) (?x)/}", "a"), "(x) (?x)");
    }

    #[test]
    fn formats_change_case() {
        assert_eq!(transformed(r"${1/(\w+)/\u$1/}", "hello world"), "Hello world");
        assert_eq!(transformed(r"${1/(\w+)/\l$1/}", "HELLO"), "hELLO");
        assert_eq!(transformed(r"${1/(\w+) (\w+)/\U$1\E $2/}", "ab cd"), "AB cd");
        assert_eq!(transformed(r"${1/.*/\L$0/}", "ABC"), "abc");
        // \u picks the first letter whichever order the two come in
        assert_eq!(transformed(r"${1/\w+/\u\L$0/}", "hELLO"), "Hello");
        assert_eq!(transformed(r"${1/\w+/\L\u$0/}", "hELLO"), "Hello");
        // literal text in the format changes too; a run ends with the replacement
        assert_eq!(transformed(r"${1/b/\Ux$0/}", "abc"), "aXBc");
        assert_eq!(transformed(r"${1/\w+/\Ux$0\Ey/g}", "ab cd"), "XABy XCDy");
        assert_eq!(transformed(r"${1/(\w+)/\u$1/}", "élan"), "Élan");
        assert_eq!(transformed(r"${1/\w+/\u$0/}", "ßx"), "SSx");
    }

    #[test]
    fn formats_take_conditionals() {
        assert_eq!(transformed("${1/(a)|(b)/(?1:A:B)/g}", "abab"), "ABAB");
        assert_eq!(transformed("${1/(a)?b/(?1:yes)/}", "b"), "");
        assert_eq!(transformed("${1/(a)?b/(?1:yes)/}", "ab"), "yes");
        assert_eq!(transformed("${1/(a)?b/(?1:yes:no)/}", "b"), "no");
        // a group that took part counts as matched, though it matched nothing (Sublime 4215)
        assert_eq!(transformed("${1/(a*)b/(?1:full:empty)/}", "b"), "full");
        assert_eq!(transformed("${1/(a*)b/(?1:full:empty)/}", "aab"), "full");
        // while one that sat out the match (the other branch of the pattern) does not
        assert_eq!(transformed("${1/(a*)b|c/(?1:T:E)/}", "c"), "E");
        // probed in Sublime 4215: this inserts `bT`
        assert_eq!(expanded("${1:b}${1/(a*)b/(?1:T:E)/}", &[]).text, "bT");
        // groups and case changes inside the parts, nested conditionals
        assert_eq!(transformed(r"${1/(\w+)(!)?/(?2:\U$1:$1)/}", "ab!"), "AB");
        let nested = "${1/(a)?(b)?c/(?1:(?2:AB:A):(?2:B:-))/}";
        assert_eq!(transformed(nested, "abc"), "AB");
        assert_eq!(transformed(nested, "ac"), "A");
        assert_eq!(transformed(nested, "bc"), "B");
        assert_eq!(transformed(nested, "c"), "-");
        // \: and \) are text in a part; a colon in the else part is text anyway
        assert_eq!(transformed(r"${1/(a)?b/(?1:x\:y\):z)/}", "ab"), "x:y)");
        assert_eq!(transformed(r"${1/(a)?b/(?1:x:y\)z)/}", "b"), "y)z");
        assert_eq!(transformed("${1/(a)?b/(?1:x:y:z)/}", "b"), "y:z");
    }

    #[test]
    fn transforms_from_sublimes_own_snippets_do_what_they_say() {
        let var = |src: &str, text: &str| match &nodes(src)[..] {
            [Node::Var { transform: Some((re, fmt, global)), .. }] => re.replace(fmt, *global, text),
            other => panic!("{src:?}: {other:?}"),
        };
        // PHP's constructor: a $ in the format that starts nothing, and \$$0
        assert_eq!(transformed(r"${1/.+/$this->$0 = \$$0;/}", "name"), "$this->name = $name;");
        // Groovy's closure: a space and an arrow only around a parameter that is there
        let before = r"${1/(.+)/(?1: )/}";
        assert_eq!(transformed(before, "obj"), " ");
        assert_eq!(transformed(before, ""), "");
        // an empty then-part: camel case from snake case, a file name for a class
        let camel = r"${1/[[:alpha:]]+|(_)/(?1::\u$0)/g}";
        assert_eq!(transformed(camel, "foo_bar_baz"), "FooBarBaz");
        let class = r"${TM_FILENAME/(?:\A|_)([A-Za-z0-9]+)(?:\.rb)?/(?2::\u$1)/g}";
        assert_eq!(var(class, "my_class.rb"), "MyClass");
        // the file name without its extension, then with another
        assert_eq!(var(r"${TM_FILENAME/(.*?)(\..+)/$1/}", "main.test.rs"), "main");
        assert_eq!(var(r"${TM_FILENAME/\..+$/.h/}", "main.cpp"), "main.h");
        // the leading white space of the selection, with the m option of the snippet
        assert_eq!(var(r"${TM_SELECTED_TEXT/([\t ]*).*/$1/m}", "  foo"), "  ");
        // the C++ class name from the file name; the `:name` after the options is ignored
        assert_eq!(var(r"${TM_FILENAME/(.+)\..+|.*/$1/:name}", "vec.h"), "vec");
    }

    #[test]
    fn realistic_snippets_parse() {
        // Sublime's Python class
        let class = "class ${1:ClassName}(${2:object}):\n\t\"\"\"${3:docstring for $1}\"\"\"\n\tdef __init__(self, ${4:arg}):\n\t\t${5:super($1, self).__init__()}\n\t\tself.$4 = $4\n\t\t$0";
        assert_eq!(
            nodes(class),
            [
                t("class "),
                field(1, vec![t("ClassName")]),
                t("("),
                field(2, vec![t("object")]),
                t("):\n\t\"\"\""),
                field(3, vec![t("docstring for "), tab(1)]),
                t("\"\"\"\n\tdef __init__(self, "),
                field(4, vec![t("arg")]),
                t("):\n\t\t"),
                field(5, vec![t("super("), tab(1), t(", self).__init__()")]),
                t("\n\t\tself."),
                tab(4),
                t(" = "),
                tab(4),
                t("\n\t\t"),
                tab(0),
            ]
        );
        // Sublime's Rails form_tag: a placeholder starting with :, { and \} inside one
        let rails = r#"${TM_RAILS_TEMPLATE_START_RUBY_INLINE}form_tag(${1::action => "${5:update}"}${6:, {:${8:class} => "${9:form}"\}})"#;
        assert_eq!(
            nodes(rails),
            [
                var("TM_RAILS_TEMPLATE_START_RUBY_INLINE", vec![]),
                t("form_tag("),
                field(1, vec![t(":action => \""), field(5, vec![t("update")]), t("\"")]),
                field(6, vec![t(", {:"), field(8, vec![t("class")]), t(" => \""), field(9, vec![t("form")]), t("\"}")]),
                t(")"),
            ]
        );
    }

    #[test]
    fn unbalanced_fields_are_errors_with_their_place() {
        for src in
            ["${1:foo", "${1", "${NAME", "${NAME:x", "${1:${2:x}", "${1/", "${1/a", "${1/a/b", "${1/a/b/g", "x ${1:\n"]
        {
            let e = error(src);
            assert!(e.contains("unbalanced `${`"), "{src:?}: {e}");
        }
        // the place is where the field opened, in lines and characters
        assert!(error("a ${1:foo").starts_with("line 1, column 3: "));
        let e = error("one\n\ttwo ${1:x ${2:y}");
        assert!(e.starts_with("line 2, column 6: "), "{e}");
        let e = error("${1:é ${2:b}");
        assert!(e.starts_with("line 1, column 1: "), "{e}");
    }

    #[test]
    fn malformed_fields_are_errors() {
        let e = error("a ${1 x}");
        assert!(e.contains("expected `}`, `:` or `/` after `${1`"), "{e}");
        assert!(e.starts_with("line 1, column 6: "), "{e}");
        assert!(error("${NAME.x}").contains("after `${NAME`"));
        assert!(error("$99999999999").contains("too large"));
        assert!(error("${99999999999}").contains("too large"));
        let deep = format!("{}x{}", "${1:".repeat(40), "}".repeat(40));
        assert!(error(&deep).contains("nested"));
        assert!(parse(&format!("{}x{}", "${1:".repeat(30), "}".repeat(30))).is_ok());
    }

    #[test]
    fn invalid_transforms_are_errors() {
        let e = error("${1/(/x/}");
        assert!(e.contains("invalid regex `(`"), "{e}");
        assert!(e.starts_with("line 1, column 5: "), "{e}");
        // a regex Sublime reads but this engine does not says which part
        let e = error(r"${1/(\A\s*)\Z/x/}");
        assert!(e.contains(r"unrecognized escape sequence at `\Z`"), "{e}");
        for lookaround in ["${1/a(?=b)/x/}", "${1/(?<!a)b/x/}", r"${1/(a)\1/x/}", r"${TM_FILENAME/(?!a)b/x/}"] {
            let e = error(lookaround);
            assert!(e.contains("lookaround and backreferences are not supported"), "{lookaround}: {e}");
        }
        assert!(error("${1/a/${1:x}/}").contains("expected `}` after `${1`"));
        assert!(error("${1/a/(?1:x/}").contains("unterminated conditional"));
        let deep = format!("${{1/a/{}x{}/}}", "(?1:".repeat(40), ")".repeat(40));
        assert!(error(&deep).contains("nested"));
    }

    // ---- the file reader and the loader ----

    const FILE: &str = "x.sublime-snippet";

    fn from(xml: &str) -> Result<Snippet, String> {
        from_xml(xml, Path::new(FILE))
    }

    fn wrapped(inner: &str) -> Result<Snippet, String> {
        from(&format!("<snippet>{inner}</snippet>"))
    }

    fn body_of(inner: &str) -> Vec<Node> {
        wrapped(inner).unwrap_or_else(|e| panic!("{inner:?}: {e}")).body
    }

    fn parts(s: &Snippet) -> (&str, &str, &str) {
        (&s.trigger, &s.scope, &s.description)
    }

    #[test]
    fn content_reads_cdata_entities_and_comments() {
        assert_eq!(body_of("<content><![CDATA[a < b && ${1:c}]]></content>"), [t("a < b && "), field(1, vec![t("c")])]);
        assert_eq!(
            body_of("<content>&lt;a&gt; &amp; &quot;b&quot; &apos;c&apos; &#65;&#x42;&#X43;</content>"),
            [t("<a> & \"b\" 'c' ABC")]
        );
        assert_eq!(body_of("<!-- one --><content>x<!-- two -->y</content><!-- three -->"), [t("xy")]);
        // ]]> in a snippet is written across two sections
        assert_eq!(body_of("<content><![CDATA[a]]]]><![CDATA[>b]]></content>"), [t("a]]>b")]);
    }

    #[test]
    fn the_layout_newlines_around_the_content_are_not_part_of_it() {
        assert_eq!(body_of("<content><![CDATA[\nline\n]]></content>"), [t("line")]);
        assert_eq!(body_of("<content><![CDATA[\n\nline\n\n]]></content>"), [t("\nline\n")]);
        assert_eq!(body_of("<content><![CDATA[a\nb]]></content>"), [t("a\nb")]);
        assert!(body_of("<content><![CDATA[\n]]></content>").is_empty());
        assert!(body_of("<content></content>").is_empty());
        // Windows line endings and a byte order mark
        let windows = "\u{feff}<snippet>\r\n<content><![CDATA[\r\na\r\nb\r\n]]></content>\r\n</snippet>\r\n";
        assert_eq!(from(windows).unwrap().body, [t("a\nb")]);
    }

    #[test]
    fn the_other_elements_are_trimmed_and_optional() {
        let s = wrapped("<content>x</content><tabTrigger> for </tabTrigger><scope>\n\tsource.rust\n</scope><description>A &amp; B</description>").unwrap();
        assert_eq!(parts(&s), ("for", "source.rust", "A & B"));
        assert_eq!(s.path, Path::new(FILE));
        let s = wrapped("<content>x</content>").unwrap();
        assert_eq!(parts(&s), ("", "", ""));
        let s = wrapped("<content>x</content><scope/><tabTrigger></tabTrigger>").unwrap();
        assert_eq!(parts(&s), ("", "", ""));
        // a declaration and comments around the root, in any order of the children
        let s = from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- hi -->\n<snippet>\n<scope>a</scope>\n<content>x</content>\n</snippet>\n<!-- bye -->\n").unwrap();
        assert_eq!(parts(&s), ("", "a", ""));
    }

    #[test]
    fn xml_problems_are_named() {
        let cases = [
            ("", "expected <snippet>"),
            ("<foo/>", "expected <snippet>"),
            ("<snippet>", "<snippet> is never closed"),
            ("<snippet><content>x</content>", "<snippet> is never closed"),
            ("<snippet><content>x", "<content> is never closed"),
            ("<snippet><content>x</snippet>", "expected </content>"),
            ("<snippet><content type=\"x\">y</content></snippet>", "attributes are not supported"),
            ("<snippet>hello<content>x</content></snippet>", "unexpected text"),
            ("<snippet><content>x</content><content>y</content></snippet>", "more than once"),
            ("<snippet><tabTrigger>x</tabTrigger></snippet>", "no <content>"),
            ("<snippet/>", "no <content>"),
            ("<snippet><content>a &nbsp; b</content></snippet>", "unknown entity &nbsp;"),
            ("<snippet><content>a & b</content></snippet>", "entity"),
            ("<snippet><content>&#xZZ;</content></snippet>", "character reference"),
            ("<snippet><content>&#0;</content></snippet>", "character reference"),
            ("<snippet><content>&#+65;</content></snippet>", "character reference"),
            ("<snippet><content>&#1114112;</content></snippet>", "character reference"),
            ("<snippet><content>a < b</content></snippet>", "CDATA"),
            ("<snippet><content><![CDATA[a</content></snippet>", "unterminated CDATA"),
            ("<snippet><!-- a <content>x</content></snippet>", "unterminated comment"),
            ("<?xml version=\"1.0\"<snippet/>", "unterminated"),
            ("<snippet><content>x</content></snippet> junk", "after </snippet>"),
            ("<snippet><content>x</content></snippet><snippet/>", "after </snippet>"),
        ];
        for (xml, wants) in cases {
            let e = from(xml).expect_err(xml);
            assert!(e.contains(wants), "{xml:?}: {e}");
        }
        // with the line it is on
        let e = from("<snippet>\n\t<content>x</content>\n\t<scope>a</content>\n</snippet>").unwrap_err();
        assert!(e.contains("line 3"), "{e}");
    }

    #[test]
    fn unknown_elements_of_a_snippet_are_skipped_with_their_content() {
        // Sublime's own C++ struct snippet has a <name>
        let s = wrapped("<name>Struct</name><content>x</content><tabTrigger>t</tabTrigger>").unwrap();
        assert_eq!(s.body, [t("x")]);
        assert_eq!(parts(&s), ("t", "", ""));
        // before or after ours, whatever they hold: attributes, children, text, CDATA, comments,
        // entities; a <content> or <scope> inside one is not ours and no duplicate
        let skipped = [
            "<author>me</author>",
            "<author/>",
            "<author></author>",
            "<author id=\"7\" note='a > b'>me</author>",
            "<meta><content>zzz</content><scope>zzz</scope><x><y/></x></meta>",
            "<info>a &amp; b &#65; <![CDATA[<not> a tag]]> <!-- <c> --> <?pi x?></info>",
            "<deep><a><b><c>text</c></b></a></deep>",
            "<a:b xmlns:a=\"u\">x</a:b>",
        ];
        for one in skipped {
            for xml in [
                format!("{one}<content>x</content><scope>a.b</scope>"),
                format!("<content>x</content>{one}<scope>a.b</scope>{one}"),
            ] {
                let s = wrapped(&xml).unwrap_or_else(|e| panic!("{xml}: {e}"));
                assert_eq!(s.body, [t("x")], "{xml}");
                assert_eq!(s.scope, "a.b", "{xml}");
            }
        }
        // only the children of <snippet> are skipped: inside ours, and as the root, they are errors
        assert!(wrapped("<content>x<b>y</b></content>").is_err());
        assert!(from("<author><content>x</content></author>").is_err());
    }

    #[test]
    fn a_skipped_element_must_still_be_well_formed() {
        let cases = [
            ("<snippet><author>me</snippet>", "expected </author>"),
            ("<snippet><author><b>me</author></snippet>", "expected </b>"),
            ("<snippet><author>me", "<author> is never closed"),
            ("<snippet><author><b>", "<b> is never closed"),
            ("<snippet><author>&nbsp;</author></snippet>", "unknown entity &nbsp;"),
            ("<snippet><author>a & b</author></snippet>", "entity"),
            ("<snippet><author>a < b</author></snippet>", "unexpected"),
            ("<snippet><author><![CDATA[x</author></snippet>", "unterminated CDATA"),
            ("<snippet><author><!-- x</author></snippet>", "unterminated comment"),
            ("<snippet><author id=\"x>me</author></snippet>", "start tag is never closed"),
            ("<snippet><author id=\"x\"", "start tag is never closed"),
            ("<snippet><!DOCTYPE x></snippet>", "unexpected `<!DOCTYPE"),
        ];
        for (xml, wants) in cases {
            let e = from(xml).expect_err(xml);
            assert!(e.contains(wants), "{xml:?}: {e}");
        }
        let e = from("<snippet>\n<author>\n  <b>\n</author>\n</snippet>").unwrap_err();
        assert!(e.contains("line 4") && e.contains("expected </b>"), "{e}");
    }

    #[test]
    fn a_misspelled_element_of_ours_is_an_error_that_names_the_right_one() {
        // one edit away, ignoring case: added, dropped, changed, or two letters swapped
        for (typo, ours) in [
            ("Scope", "scope"),
            ("scopes", "scope"),
            ("scop", "scope"),
            ("scoep", "scope"),
            ("tabtrigger", "tabTrigger"),
            ("TabTrigger", "tabTrigger"),
            ("tabTriger", "tabTrigger"),
            ("tabTirgger", "tabTrigger"),
            ("tabTrigger2", "tabTrigger"),
            ("Content", "content"),
            ("contents", "content"),
            ("contant", "content"),
            ("descripton", "description"),
            ("descriptions", "description"),
            ("Description", "description"),
        ] {
            for xml in [
                format!("<snippet><{typo}>x</{typo}><content>x</content></snippet>"),
                format!("<snippet>\n<content>x</content>\n<{typo}/>\n</snippet>"),
            ] {
                let e = from(&xml).expect_err(&xml);
                assert!(e.contains(&format!("unexpected <{typo}>")), "{xml}: {e}");
                assert!(e.contains(&format!("did you mean `<{ours}>`?")), "{xml}: {e}");
            }
        }
        let e = from("<snippet>\n<content>x</content>\n<Scope>a</Scope>\n</snippet>").unwrap_err();
        assert!(e.contains("line 3"), "{e}");
        // two edits away, or more, is some other element: skipped
        for other in [
            "name",
            "author",
            "source",
            "scope-note",
            "tab",
            "desc",
            "con",
            "scops2",
            "x:scope",
            "meta",
            "tabTriggers2",
        ] {
            let xml = format!("<snippet><{other}>x</{other}><content>x</content></snippet>");
            assert!(from(&xml).is_ok(), "{other}");
        }
        // only the children of <snippet> are looked at: inside an unknown one, a near miss is skipped
        assert!(wrapped("<meta><Scope>x</Scope></meta><content>x</content>").is_ok());
    }

    #[test]
    fn one_edit_agrees_with_the_edit_distance() {
        // optimal string alignment distance, the plain way
        fn distance(a: &[char], b: &[char]) -> usize {
            let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
            for (i, row) in d.iter_mut().enumerate() {
                row[0] = i;
            }
            for (j, cell) in d[0].iter_mut().enumerate() {
                *cell = j;
            }
            for i in 1..=a.len() {
                for j in 1..=b.len() {
                    let cost = usize::from(a[i - 1] != b[j - 1]);
                    let best = (d[i - 1][j] + 1).min(d[i][j - 1] + 1);
                    d[i][j] = best.min(d[i - 1][j - 1] + cost);
                    if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                        d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
                    }
                }
            }
            d[a.len()][b.len()]
        }
        // every word of up to four letters from `abc`, against every other
        let mut words = vec![String::new()];
        let mut longest = vec![String::new()];
        for _ in 0..4 {
            longest = (longest.iter()).flat_map(|w| "abc".chars().map(move |c| format!("{w}{c}"))).collect();
            words.extend(longest.clone());
        }
        for a in &words {
            for b in &words {
                let (x, y): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
                assert_eq!(one_edit(&x, &y), distance(&x, &y) <= 1, "{a:?} and {b:?}");
            }
        }
    }

    #[test]
    fn content_errors_name_the_content_and_the_place() {
        let e = wrapped("<content><![CDATA[a\n  ${1:b]]></content>").unwrap_err();
        assert!(e.starts_with("<content> line 2, column 3: "), "{e}");
        assert!(e.contains("unbalanced"), "{e}");
        let e = wrapped("<content><![CDATA[${1/(/x/}]]></content>").unwrap_err();
        assert!(e.contains("invalid regex"), "{e}");
    }

    #[test]
    fn an_invalid_scope_selector_is_reported() {
        let bad = [
            ("source.a.b.c.d.e.f.g.h", "more than 8 parts"),
            ("source..python", "empty part"),
            ("(source.c", "unbalanced `(`"),
            ("source.c)", "unbalanced `)`"),
            ("source.c -", "has no scope"),
            ("source.c @ string", "unexpected character `@`"),
        ];
        for (scope, wants) in bad {
            let xml = format!("<content>x</content><scope>{scope}</scope>");
            let e = wrapped(&xml).unwrap_err();
            assert!(e.contains("<scope>") && e.contains(scope) && e.contains(wants), "{scope}: {e}");
        }
        // Sublime's groups, & and -( ) are selectors; an empty one applies everywhere
        for ok in [
            "",
            "   ",
            "source.rust",
            "text.html - source.js",
            "source.rust, source.python",
            "source.rust | source.go",
            "source.rust string - comment",
            "(source.c | source.c++) - comment - string",
            "source.c &amp; string",
            "-(constant, entity)",
        ] {
            let xml = format!("<content>x</content><scope>{ok}</scope>");
            assert!(wrapped(&xml).is_ok(), "{ok}");
        }
        // written over lines, kept as written
        let s = wrapped("<content>x</content><scope>\n\ta.b\n\t\t- c   d\n</scope>").unwrap();
        assert_eq!(s.scope, "a.b\n\t\t- c   d");
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("orlo-snippet-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, text: impl AsRef<[u8]>) -> PathBuf {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn rel(dir: &Path, path: &Path) -> String {
        path.strip_prefix(dir).unwrap().display().to_string()
    }

    const GOOD: &str = "<snippet>\n\t<content><![CDATA[for ${1:i} in ${2:0..n} {\n\t$0\n}]]></content>\n\t<tabTrigger>for</tabTrigger>\n\t<scope>source.rust</scope>\n\t<description>For loop</description>\n</snippet>\n";

    #[test]
    fn load_reads_a_folder_tree_and_reports_each_bad_file() {
        let dir = temp("tree");
        for (name, text) in [
            ("for", GOOD),
            ("Rust/deep/min", "<snippet><content>hi &amp; bye</content></snippet>"),
            (".hidden", GOOD),
            (".git/inside", GOOD),
            ("unclosed", "<snippet><content>x</snippet>"),
            ("nocontent", "<snippet><tabTrigger>x</tabTrigger></snippet>"),
            ("unbalanced", "<snippet><content><![CDATA[a ${1:b]]></content></snippet>"),
            ("badregex", "<snippet><content><![CDATA[${1/(/x/}]]></content></snippet>"),
            ("badscope", "<snippet><content>x</content><scope>a.b.c.d.e.f.g.h.i</scope></snippet>"),
        ] {
            write(&dir, &format!("{name}.sublime-snippet"), text);
        }
        write(&dir, "notes.txt", "not a snippet");
        write(&dir, "binary.sublime-snippet", [0xffu8, 0xfe, 0x00]);
        write(&dir, "empty.sublime-snippet", "");

        let (good, bad) = load(&dir);
        let found: Vec<_> = good.iter().map(|s| rel(&dir, &s.path)).collect();
        assert_eq!(found, ["for.sublime-snippet", "Rust/deep/min.sublime-snippet"]);
        assert_eq!(good[0].path, dir.join("for.sublime-snippet"));
        assert_eq!(parts(&good[0]), ("for", "source.rust", "For loop"));
        assert_eq!(
            good[0].body,
            [t("for "), field(1, vec![t("i")]), t(" in "), field(2, vec![t("0..n")]), t(" {\n\t"), tab(0), t("\n}")]
        );
        assert_eq!(parts(&good[1]), ("", "", ""));
        assert_eq!(good[1].body, [t("hi & bye")]);

        let failed: Vec<_> = bad.iter().map(|(p, _)| rel(&dir, p)).collect();
        assert_eq!(
            failed,
            [
                "badregex.sublime-snippet",
                "badscope.sublime-snippet",
                "binary.sublime-snippet",
                "empty.sublime-snippet",
                "nocontent.sublime-snippet",
                "unbalanced.sublime-snippet",
                "unclosed.sublime-snippet"
            ]
        );
        let whys = [
            "invalid regex",
            "<scope>",
            "UTF-8",
            "expected <snippet>",
            "no <content>",
            "unbalanced",
            "expected </content>",
        ];
        for ((_, why), wants) in bad.iter().zip(whys) {
            assert!(why.contains(wants), "{why}");
        }
    }

    #[test]
    fn load_of_nothing_is_empty_and_quiet() {
        let dir = temp("nothing");
        assert_eq!(load(&dir.join("missing")), (vec![], vec![]));
        let (good, bad) = load(&dir);
        assert!(good.is_empty() && bad.is_empty());
        // a file where the folder should be is said, not ignored
        let file = write(&dir, "file", "x");
        let (good, bad) = load(&file);
        assert!(good.is_empty());
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0].0, file);
    }

    #[test]
    fn load_follows_linked_files_not_linked_folders() {
        let dir = temp("links");
        let outside = temp("links-outside");
        let real = write(&outside, "real.sublime-snippet", GOOD);
        std::os::unix::fs::symlink(&real, dir.join("linked.sublime-snippet")).unwrap();
        // a folder that links to its parent would never end
        std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("elsewhere")).unwrap();
        write(&dir, "plain.sublime-snippet", GOOD);
        let (good, bad) = load(&dir);
        assert!(bad.is_empty(), "{bad:?}");
        let found: Vec<_> = good.iter().map(|s| rel(&dir, &s.path)).collect();
        assert_eq!(found, ["linked.sublime-snippet", "plain.sublime-snippet"]);
        // a link to nowhere is not a snippet
        std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("dangling.sublime-snippet")).unwrap();
        assert_eq!(load(&dir).0.len(), 2);
    }

    #[test]
    fn a_file_over_the_size_limit_is_reported_unread() {
        let dir = temp("big");
        let filler = " ".repeat(MAX_FILE as usize);
        write(&dir, "big.sublime-snippet", format!("<snippet>{filler}<content>x</content></snippet>"));
        let (good, bad) = load(&dir);
        assert!(good.is_empty());
        assert!(bad[0].1.contains("larger than"), "{bad:?}");
    }

    #[test]
    fn the_folder_is_snippets_in_the_support_folder() {
        assert_eq!(dir(), orlo_utils::paths::support_dir().join("Snippets"));
    }

    fn snippet_xml(trigger: &str, scope: &str) -> String {
        format!("<snippet><content>x</content><tabTrigger>{trigger}</tabTrigger><scope>{scope}</scope></snippet>")
    }

    #[test]
    fn snippets_are_found_by_trigger_and_by_scope() {
        let dir = temp("scopes");
        for (name, trigger, scope) in [
            ("any", "fn", ""),
            // over two lines, as Sublime's own HTML snippets write theirs
            ("html", "", "text.html\n\t\t- source"),
            ("py", "fn", "source.python"),
            ("rs", "fn", "source.rust"),
            ("rsstr", "q", "source.rust string"),
            ("rsor", "fn", "source.go, source.rust"),
            // Sublime's groups and exclusions, as its own snippets write them
            ("cpp", "fn", "(source.c | source.c++) - comment"),
            ("paren", "fn", "(source.go | source.rust) - comment"),
        ] {
            write(&dir, &format!("{name}.sublime-snippet"), snippet_xml(trigger, scope));
        }
        let mut store = Snippets::new(dir);
        store.refresh();
        assert_eq!(store.snippets().len(), 8);
        let files = |found: Vec<usize>| -> Vec<String> {
            let name = |i: usize| rel(&store.dir, &store.snippets()[i].path);
            (found.into_iter().map(name)).map(|n| n.replace(".sublime-snippet", "")).collect()
        };
        let rust = highlight::scope_id(&["source.rust", "meta.function.rust"]);
        let rust_string = highlight::scope_id(&["source.rust", "string.quoted.double.rust"]);
        let html = highlight::scope_id(&["text.html.basic"]);
        let html_js = highlight::scope_id(&["text.html.basic", "source.js.embedded.html"]);
        let rust_comment = highlight::scope_id(&["source.rust", "comment.line.double-slash.rust"]);
        let cpp = highlight::scope_id(&["source.c++", "meta.function.c++"]);
        let none = highlight::scope_id(&[]);
        let trigger =
            |word: &str, scope| files(store.with_trigger(word, |selector| highlight::matches(scope, selector)));
        let in_scope = |scope| files(store.for_scope(|selector| highlight::matches(scope, selector)));
        assert_eq!(trigger("fn", rust), ["any", "paren", "rs", "rsor"]);
        assert_eq!(trigger("fn", rust_comment), ["any", "rs", "rsor"]);
        assert_eq!(trigger("fn", cpp), ["any", "cpp"]);
        assert_eq!(trigger("fn", none), ["any"]);
        assert_eq!(trigger("q", rust), Vec::<String>::new());
        assert_eq!(trigger("q", rust_string), ["rsstr"]);
        // no trigger, nothing to find it by; the empty word finds nothing
        assert_eq!(trigger("", rust), Vec::<String>::new());
        assert_eq!(in_scope(rust), ["any", "paren", "rs", "rsor"]);
        assert_eq!(in_scope(rust_string), ["any", "paren", "rs", "rsor", "rsstr"]);
        assert_eq!(in_scope(html), ["any", "html"]);
        assert_eq!(in_scope(html_js), ["any"]);
    }

    #[test]
    fn the_store_reloads_when_the_listing_or_a_stamp_changes_at_most_once_a_second() {
        let dir = temp("store");
        write(&dir, "a.sublime-snippet", snippet_xml("a", ""));
        let mut store = Snippets::new(dir.clone());
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        assert!(store.snippets().is_empty());
        assert!(store.refresh_at(t0), "the first look loads");
        assert_eq!(store.snippets().len(), 1);
        assert!(!store.refresh_at(at(1000)), "nothing changed");

        // a new file is not looked for within a second of the last look
        write(&dir, "b.sublime-snippet", snippet_xml("b", ""));
        assert!(!store.refresh_at(at(1500)));
        assert_eq!(store.snippets().len(), 1);
        assert!(store.refresh_at(at(2000)));
        assert_eq!(store.snippets().len(), 2);
        assert!(!store.refresh_at(at(3000)));

        // a changed file (its size, so even a coarse clock shows it)
        write(&dir, "a.sublime-snippet", snippet_xml("aaaa", ""));
        assert!(store.refresh_at(at(4000)));
        assert_eq!(store.snippets()[0].trigger, "aaaa");

        // a file in a new folder, then a removed one
        write(&dir, "sub/c.sublime-snippet", snippet_xml("c", ""));
        assert!(store.refresh_at(at(5000)));
        assert_eq!(store.snippets().len(), 3);
        std::fs::remove_file(dir.join("b.sublime-snippet")).unwrap();
        assert!(store.refresh_at(at(6000)));
        assert_eq!(store.snippets().len(), 2);

        // a bad file is reported by the load that reads it, and kept until it is fixed
        assert!(store.errors().is_empty());
        write(&dir, "bad.sublime-snippet", "<snippet>");
        assert!(store.refresh_at(at(7000)));
        assert_eq!(store.errors().len(), 1);
        assert!(!store.refresh_at(at(8000)));
        assert_eq!(store.errors().len(), 1);
        write(&dir, "bad.sublime-snippet", snippet_xml("fixed", ""));
        assert!(store.refresh_at(at(9000)));
        assert!(store.errors().is_empty());
        assert_eq!(store.snippets().len(), 3);

        // a folder that is not there is an empty store, not an error
        let mut none = Snippets::new(dir.join("missing"));
        assert!(none.refresh_at(t0));
        assert!(none.snippets().is_empty() && none.errors().is_empty());
        assert!(!none.refresh_at(at(5000)));
    }

    #[test]
    fn the_template_is_a_snippet_that_loads() {
        let dir = temp("template");
        write(&dir, "new.sublime-snippet", template());
        let (good, bad) = load(&dir);
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(good.len(), 1);
        assert!(!good[0].trigger.is_empty() && !good[0].body.is_empty());
        // the comments are XML comments any XML reader takes: no `--` inside one
        for comment in template().split("<!--").skip(1) {
            let inside = comment.split("-->").next().unwrap();
            assert!(!inside.contains("--"), "{inside}");
        }
        assert!(
            template().contains("tabTrigger") && template().contains("scope") && template().contains("description")
        );
        assert!(template().contains("${1/regex/format/"));
    }

    /// Garbage from the characters that mean something: an error or a parse, never a panic.
    #[test]
    fn garbage_is_an_error_or_a_parse_never_a_panic() {
        let mut next = random(0x2545_f491_4f6c_dd1d);
        let body = [
            "$", "{", "}", "\\", "/", ":", "(", ")", "?", "1", "0", "a", "é", "\n", "|", "g", "m", "${1:", "${1/",
            "/g}", "(?1:", "\\u", "\\U", "\\E", "x*", "^", "[", "😀",
        ];
        let xml = [
            "<",
            ">",
            "/",
            "!",
            "-",
            "[",
            "]",
            "&",
            ";",
            "#",
            "x",
            "a",
            " ",
            "\n",
            "?",
            "é",
            "\r",
            "<![CDATA[",
            "]]>",
            "<!--",
            "-->",
            "<snippet>",
            "</snippet>",
            "<content>",
            "</content>",
            "&amp;",
            "&#65;",
            "<scope/>",
        ];
        for _ in 0..1500 {
            let len = 1 + next(20);
            let src: String = (0..len).map(|_| body[next(body.len())]).collect();
            let _ = parse(&src);
            let file: String = (0..24).map(|_| xml[next(xml.len())]).collect();
            let _ = from(&file);
            let format: String = (0..10).map(|_| body[next(body.len())]).collect();
            let _ = parse(&format!("${{1/(a)|(é)/{}/g}}", format.replace('/', "\\/")));
        }
    }

    // ---- expansion for one caret, and the places sessions start from ----

    /// `src` expanded with these variables, no indentation, tabs kept.
    #[test]
    fn a_shrinking_mirror_skips_fields_already_gone() {
        let ch = |start, removed, inserted| crate::buffer::Change { start, removed, inserted, lines: (0, 0) };
        let mut s = Session::new(&[(0, expanded("$1 ${1:ab${2:c}}", &[]))], 0);
        s.map(&ch(0, 3, 1));
        s.map(&ch(2, 3, 1));
        for _ in 0..6 {
            s.map(&ch(s.range(0).end, 0, 1));
            s.map(&ch(s.range(1).end, 0, 1));
        }
        let p = s.range(1);
        s.map(&ch(p.start, p.len(), 0));
        s.replaced(0, 0);
        assert_eq!(s.range(0), 0..0);
    }

    fn expanded(src: &str, vars: &[(&str, &str)]) -> Expansion {
        let var = |name: &str| vars.iter().find(|v| v.0 == name).map(|v| v.1.to_string());
        expand(&nodes(src), &var, "", "\t")
    }

    /// An expansion's places: number, range, role (p, m or t) and parent.
    fn places(e: &Expansion) -> Vec<(u32, Range<usize>, char, Option<usize>)> {
        let role = |r: &Role| match r {
            Role::Primary => 'p',
            Role::Mirror => 'm',
            Role::Transformed(..) => 't',
        };
        (e.fields.iter()).map(|p| (p.n, p.range.clone(), role(&p.role), p.parent)).collect()
    }

    #[test]
    fn a_session_follows_random_typing_without_losing_its_places() {
        use crate::buffer::Change;
        let mut pick = random(0x0bad_cafe_f00d_1234);
        let bits = [
            "${1:ab}",
            "$1",
            "${2:c ${3:d}}",
            "$2",
            "${1/(.)/<$1>/g}",
            "$3",
            "x",
            "é",
            "\n",
            "${4:$1}",
            "$0",
            "${2/b/B/}",
            "${3:}",
            "$4",
        ];
        for _ in 0..500 {
            let src: String = (0..1 + pick(6)).map(|_| bits[pick(bits.len())]).collect();
            let e = expanded(&src, &[]);
            let mut rope = ropey::Rope::from_str(&e.text);
            let mut s = Session::new(&[(0, e)], 0);
            for _ in 0..6 {
                if pick(4) == 0 {
                    s.step(pick(2) == 0);
                }
                let sel = s.selection();
                let Some(r) = sel.get(pick(sel.len().max(1))).cloned() else {
                    break;
                };
                let start = r.start + pick(r.len() + 1);
                let removed = pick(r.end - start + 1);
                let inserted = pick(3);
                let change = Change { start, removed, inserted, lines: (0, 0) };
                rope.remove(start..start + removed);
                rope.insert(start, &"x".repeat(inserted));
                assert!(s.map(&change).is_some(), "{src:?}: {change:?}");
                for _ in 0..2 {
                    for i in s.mirrors() {
                        if let Some(want) = s.stale(i, &rope) {
                            let r = s.range(i);
                            rope.remove(r.clone());
                            rope.insert(r.start, &want);
                            s.replaced(i, want.chars().count());
                        }
                    }
                }
                for i in s.mirrors() {
                    assert_eq!(s.stale(i, &rope), None, "{src:?} after {change:?}");
                }
                // a place that went is no longer kept up to date
                for (i, p) in s.places.iter().enumerate().filter(|(_, p)| !p.gone) {
                    assert!(p.range.start <= p.range.end, "{src:?}: {change:?}");
                    assert!(p.range.end <= rope.len_chars(), "{src:?}: {change:?}");
                    if let (false, Some(q)) = (p.gone, p.parent) {
                        let outer = &s.places[q].range;
                        assert!(
                            outer.start <= p.range.start && p.range.end <= outer.end,
                            "{src:?}: place {i} left its primary {q} after {change:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_full_name_is_the_passwd_entrys() {
        // macOS says it too: `id -F`
        let said = std::process::Command::new("id").arg("-F").output();
        if let Some(out) = said.ok().filter(|o| o.status.success()) {
            assert_eq!(full_name(), String::from_utf8_lossy(&out.stdout).trim());
        }
        assert!(!full_name().contains(','));
    }

    #[test]
    fn a_variable_is_its_value_else_its_default() {
        let vars = [("SELECTION", "picked"), ("TM_FILENAME", "main.rs"), ("EMPTY", "")];
        let text = |src: &str| expanded(src, &vars).text;
        assert_eq!(text("<$SELECTION>"), "<picked>");
        assert_eq!(text("${TM_FILENAME}"), "main.rs");
        // unknown or empty: the default, else nothing
        assert_eq!(text("[$NOPE]"), "[]");
        assert_eq!(text("${NOPE:fallback}"), "fallback");
        assert_eq!(text("${EMPTY:fallback}"), "fallback");
        assert_eq!(text("${SELECTION:fallback}"), "picked");
        // a transform of the value, of nothing when there is none
        assert_eq!(text("${TM_FILENAME/(\\w+)\\.rs/\\u$1/}"), "Main");
        assert_eq!(text("${NOPE/^$/none/}"), "none");
        // a field in a default is there only when the default is used
        let e = expanded("${NOPE:${1:x}}", &vars);
        assert_eq!((e.text.as_str(), places(&e)), ("x", vec![(1, 0..1, 'p', None)]));
        let e = expanded("${SELECTION:${1:x}}$1", &vars);
        assert_eq!((e.text.as_str(), places(&e)), ("picked", vec![(1, 6..6, 'p', None)]));
        // backticks are text: nothing is run
        assert_eq!(text("`date`"), "`date`");
    }

    #[test]
    fn lines_after_the_first_take_the_indent_and_tabs_become_the_views() {
        let body = nodes("if ${1:x} {\n\t${2:y\n\tz}\n}");
        let none = |_: &str| None;
        let e = expand(&body, &none, "    ", "  ");
        assert_eq!(e.text, "if x {\n      y\n      z\n    }");
        assert_eq!(places(&e), [(1, 3..4, 'p', None), (2, 13..22, 'p', None)]);
        // a view that indents with tabs keeps them
        let e = expand(&body, &none, "\t", "\t");
        assert_eq!(e.text, "if x {\n\t\ty\n\t\tz\n\t}");
        // a variable's value goes in as it is
        let sel = |n: &str| (n == "SELECTION").then(|| "a\n\tb".to_string());
        let e = expand(&nodes("[\n$SELECTION]"), &sel, "  ", "    ");
        assert_eq!(e.text, "[\n  a\n\tb]");
    }

    #[test]
    fn a_number_used_again_is_a_mirror_or_a_transformed_one() {
        let e = expanded("${1:foo} $1 ${1/(.)/\\u$1/}", &[]);
        assert_eq!(e.text, "foo foo Foo");
        assert_eq!(places(&e), [(1, 0..3, 'p', None), (1, 4..7, 'm', None), (1, 8..11, 't', None)]);
        // the primary is the first with a default, else the first; one before it mirrors it too
        let e = expanded("$1 ${1:foo} ${1:bar}", &[]);
        assert_eq!(e.text, "foo foo foo");
        assert_eq!(places(&e), [(1, 0..3, 'm', None), (1, 4..7, 'p', None), (1, 8..11, 'm', None)]);
        let e = expanded("$1 and ${1}", &[]);
        assert_eq!(places(&e), [(1, 0..0, 'p', None), (1, 5..5, 'm', None)]);
        // places count chars
        let e = expanded("${1:é}$1", &[]);
        assert_eq!(places(&e), [(1, 0..1, 'p', None), (1, 1..2, 'm', None)]);
        // a field that is not there transforms nothing, and is not followed
        let e = expanded("${2/^$/none/}", &[]);
        assert_eq!((e.text.as_str(), places(&e)), ("none", vec![]));
    }

    #[test]
    fn places_inside_a_field_name_its_primary() {
        let e = expanded("${1:a ${2:b}} $0", &[]);
        assert_eq!(e.text, "a b ");
        assert_eq!(places(&e), [(1, 0..3, 'p', None), (2, 2..3, 'p', Some(0)), (0, 4..4, 'p', None)]);
        // a mirror inside a field, of a field written before it
        let e = expanded("${1:key}: ${2:$1}", &[]);
        assert_eq!(e.text, "key: key");
        assert_eq!(places(&e), [(1, 0..3, 'p', None), (2, 5..8, 'p', None), (1, 5..8, 'm', Some(1))]);
    }

    #[test]
    fn a_mirror_inside_a_field_never_follows_one_it_is_in_or_one_written_after_it() {
        // either way fields could follow each other round in a circle: those mirrors are empty
        let e = expanded("${1:a $1}", &[]);
        assert_eq!((e.text.as_str(), places(&e)), ("a ", vec![(1, 0..2, 'p', None)]));
        let e = expanded("${1:<$2>} ${2:x}", &[]);
        assert_eq!(e.text, "<> x");
        assert_eq!(places(&e), [(1, 0..2, 'p', None), (2, 3..4, 'p', None)]);
        let e = expanded("${1:a $2} ${2:b $1}", &[]);
        assert_eq!(e.text, "a  b a ");
        assert_eq!(places(&e), [(1, 0..2, 'p', None), (2, 3..7, 'p', None), (1, 5..7, 'm', Some(1))]);
        // a mirror's own default is not expanded: it shows the primary
        let e = expanded("${1:a} ${1:b ${2:c}}", &[]);
        assert_eq!(e.text, "a a");
        assert_eq!(places(&e), [(1, 0..1, 'p', None), (1, 2..3, 'm', None)]);
    }
}
