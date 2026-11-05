//! Bounded, indentation-only lexical context. This is not a parser or formatter.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentLanguage {
    Python,
    Rust,
    JavaScript,
    TypeScript,
}

impl IndentLanguage {
    pub fn from_syntax(name: &str) -> Option<Self> {
        match name {
            "Python" => Some(Self::Python),
            "Rust" => Some(Self::Rust),
            "JavaScript" | "JavaScript (Babel)" => Some(Self::JavaScript),
            "TypeScript" | "TypeScript (TSX)" | "TSX" => Some(Self::TypeScript),
            _ => None,
        }
    }
}

pub(crate) const LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Code,
    Quote(char),
    Triple(char),
    Raw(usize),
    Regex(bool),
    BlockComment,
    LineComment,
}

pub(crate) struct Context {
    language: IndentLanguage,
    mode: Mode,
    escaped: bool,
    brackets: Vec<char>,
    comment_depth: usize,
    reliable: bool,
    has_code: bool,
}

pub(crate) struct Line {
    pub code: String,
    pub before_depth: usize,
    pub after_depth: usize,
    pub active: bool,
    pub protected: bool,
    pub starts_protected: bool,
}

impl Context {
    pub fn new(language: IndentLanguage) -> Self {
        Self {
            language,
            mode: Mode::Code,
            escaped: false,
            brackets: Vec::new(),
            comment_depth: 0,
            reliable: true,
            has_code: false,
        }
    }

    pub fn reliable(&self) -> bool {
        self.reliable
    }

    pub fn line(&mut self, text: &str) -> Line {
        let before_depth = self.brackets.len();
        let starts_protected = self.mode != Mode::Code;
        let chars: Vec<char> = text.chars().collect();
        let mut code = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            match self.mode {
                Mode::LineComment => break,
                Mode::BlockComment => {
                    if self.language == IndentLanguage::Rust && c == '/' && next == Some('*') {
                        self.comment_depth += 1;
                        i += 1;
                    } else if c == '*' && next == Some('/') {
                        self.comment_depth -= 1;
                        i += 1;
                        if self.comment_depth == 0 {
                            self.mode = Mode::Code;
                        }
                    }
                }
                Mode::Raw(hashes) => {
                    if c == '"' && (0..hashes).all(|n| chars.get(i + n + 1) == Some(&'#')) {
                        self.mode = Mode::Code;
                        i += hashes;
                    }
                }
                Mode::Regex(class) => {
                    if self.escaped {
                        self.escaped = false;
                    } else if c == '\\' {
                        self.escaped = true;
                    } else if c == '[' && !class {
                        self.mode = Mode::Regex(true);
                    } else if c == ']' && class {
                        self.mode = Mode::Regex(false);
                    } else if c == '/' && !class {
                        self.mode = Mode::Code;
                    }
                }
                Mode::Quote(q) | Mode::Triple(q) => {
                    if self.escaped {
                        self.escaped = false;
                    } else if c == '\\' {
                        self.escaped = true;
                    } else if c == q {
                        if self.mode == Mode::Quote(q) {
                            self.mode = Mode::Code;
                        } else if next == Some(q) && chars.get(i + 2) == Some(&q) {
                            self.mode = Mode::Code;
                            i += 2;
                        }
                    }
                }
                Mode::Code => {
                    let python = self.language == IndentLanguage::Python;
                    let raw_hashes = if self.language == IndentLanguage::Rust && c == 'r' {
                        let count = chars[i + 1..].iter().take_while(|&&c| c == '#').count();
                        (chars.get(i + 1 + count) == Some(&'"')).then_some(count)
                    } else {
                        None
                    };
                    let lifetime = self.language == IndentLanguage::Rust
                        && c == '\''
                        && next.is_some_and(|c| c.is_alphabetic() || c == '_')
                        && chars.get(i + 2) != Some(&'\'');
                    if let Some(hashes) = raw_hashes {
                        self.mode = Mode::Raw(hashes);
                        i += hashes + 1;
                        code.push('x');
                    } else if lifetime {
                        code.push(c);
                    } else if (python && c == '#') || (!python && c == '/' && next == Some('/')) {
                        self.mode = Mode::LineComment;
                    } else if !python && c == '/' && next == Some('*') {
                        self.mode = Mode::BlockComment;
                        self.comment_depth = 1;
                        i += 1;
                    } else if matches!(self.language, IndentLanguage::JavaScript | IndentLanguage::TypeScript)
                        && c == '/'
                    {
                        let context = if code.trim().is_empty() && self.has_code { None } else { slash_context(&code) };
                        match context {
                            Some(true) => {
                                self.mode = Mode::Regex(false);
                                code.push('x');
                            }
                            Some(false) => code.push(c),
                            None => {
                                self.reliable = false;
                                code.push(c);
                            }
                        }
                    } else if matches!(c, '\'' | '"') || (!python && c == '`') {
                        if python && next == Some(c) && chars.get(i + 2) == Some(&c) {
                            self.mode = Mode::Triple(c);
                            i += 2;
                        } else {
                            self.mode = Mode::Quote(c);
                        }
                        code.push('x');
                    } else {
                        code.push(c);
                        match c {
                            '(' | '[' | '{' => self.brackets.push(c),
                            ')' | ']' | '}' => {
                                let open = match c {
                                    ')' => '(',
                                    ']' => '[',
                                    _ => '{',
                                };
                                if self.brackets.last() == Some(&open) {
                                    self.brackets.pop();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            i += 1;
        }
        if matches!(self.mode, Mode::Regex(_)) {
            self.reliable = false;
        }
        if !code.trim().is_empty() {
            self.has_code = true;
        }
        let active = self.mode == Mode::Code;
        let protected = !matches!(self.mode, Mode::Code | Mode::LineComment);
        if self.mode == Mode::LineComment {
            self.mode = Mode::Code;
        }
        // Rust double-quoted strings can contain literal newlines.
        if matches!(self.mode, Mode::Quote('\'' | '"'))
            && !(self.language == IndentLanguage::Rust && self.mode == Mode::Quote('"'))
            && !self.escaped
        {
            self.mode = Mode::Code;
        }
        self.escaped = false;
        Line { code, before_depth, after_depth: self.brackets.len(), active, protected, starts_protected }
    }
}

// Slash can start either division or a regexp; decline ambiguous statement boundaries.
fn slash_context(code: &str) -> Option<bool> {
    let code = code.trim_end();
    let Some(last) = code.chars().last() else {
        return Some(true);
    };
    if last.is_alphanumeric() || last == '_' || last == '$' {
        let word = code.rsplit(|c: char| !c.is_alphanumeric() && c != '_' && c != '$').next().unwrap_or("");
        return Some(matches!(
            word,
            "return"
                | "throw"
                | "case"
                | "delete"
                | "void"
                | "typeof"
                | "new"
                | "yield"
                | "await"
                | "in"
                | "of"
                | "else"
                | "do"
        ));
    }
    match last {
        ']' => Some(false),
        ')' | '}' => None,
        '+' | '-' if code.ends_with("++") || code.ends_with("--") => None,
        '(' | '[' | '{' | '=' | ':' | ',' | ';' | '!' | '?' | '+' | '-' | '*' | '%' | '&' | '|' | '^' | '~' | '<'
        | '>' => Some(true),
        _ => None,
    }
}

/// Extract an ASCII keyword only at a conservative complete-token boundary.
/// Non-ASCII scalars after the candidate (including combining marks) may extend
/// an identifier and therefore never authorize a keyword rule.
pub(crate) fn first_word(code: &str) -> &str {
    let code = code.trim_start();
    let end = code.find(|c: char| !c.is_ascii_alphanumeric() && c != '_').unwrap_or(code.len());
    let suffix = &code[end..];
    if suffix.chars().next().is_some_and(|c| !c.is_ascii() && !c.is_whitespace()) {
        ""
    } else {
        &code[..end]
    }
}

pub(crate) fn dedent_keyword(code: &str) -> bool {
    matches!(first_word(code), "elif" | "else" | "except" | "finally")
}

pub(crate) fn exits_block(code: &str) -> bool {
    matches!(first_word(code), "return" | "raise" | "break" | "continue" | "pass")
}

pub(crate) fn next_columns(language: IndentLanguage, line: &Line, base: usize, width: usize) -> usize {
    if line.protected {
        return base;
    }
    next_columns_active(language, line, base, width)
}

pub(crate) fn next_columns_active(language: IndentLanguage, line: &Line, base: usize, width: usize) -> usize {
    let code = line.code.trim_end();
    let leading_closers = code.trim_start().chars().take_while(|c| matches!(c, ')' | ']' | '}')).count();
    let opened = line.after_depth.saturating_add(leading_closers).saturating_sub(line.before_depth);
    if opened > 0 {
        base + width * opened
    } else if (language == IndentLanguage::Python && code.ends_with(':')) || code.ends_with('\\') {
        base + width
    } else if language == IndentLanguage::Python && exits_block(code) {
        base.saturating_sub(width)
    } else {
        let closed_after_code = line.before_depth.saturating_sub(line.after_depth).saturating_sub(leading_closers);
        base.saturating_sub(width * closed_after_code)
    }
}
