//! Scope selectors, Sublime's (TextMate's, with `&`): where a snippet, a spelling check or a
//! colour applies, as in `source.rust string - comment`, `a, b`, `(a | b) & c` or
//! `-(constant, entity)`. Pure, so any thread can parse and match one.
//!
//! ```text
//! selector    := conjunction (("," | "|")+ conjunction)*     union, the lowest precedence
//! conjunction := operand ("&"? operand)*                     every operand must match
//! operand     := "-" operand | "(" selector ")" | path
//! path        := identifier+                                 a descendant path
//! ```
//!
//! A path matches a stack that has, in order but not necessarily next to each other, a scope with
//! each of its identifiers as a dotted prefix (`source.python` is a prefix of
//! `source.python.embedded`, not of `source.pythonx`). An identifier starts with a letter, a digit,
//! `_`, `.`, `:` or `+` and goes on with those and `-` (`meta.tag-name`, `source.c++`), so a `-`
//! that starts a token is the minus. Any white space separates. An empty selector matches nothing.
//! A mistake is an error that says where, never a selector that quietly matches nothing.

use syntect::parsing::Scope;

/// Parentheses and minus signs nested deeper are an error, not a stack overflow.
const MAX_NESTING: usize = 64;

/// A scope has at most this many dotted parts (syntect's limit).
const MAX_PARTS: usize = 8;

/// A parsed selector.
#[derive(Clone, Debug, PartialEq)]
pub struct Selector(Expr);

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    /// The empty selector.
    Nothing,
    /// A descendant path, never empty.
    Path(Vec<Scope>),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

impl Selector {
    /// The selector `src` says, or what is wrong with it and where (columns count characters
    /// from 1).
    pub fn parse(src: &str) -> Result<Selector, String> {
        let tokens = lex(src)?;
        if tokens.is_empty() {
            return Ok(Selector(Expr::Nothing));
        }
        let mut parser = Parser { tokens: &tokens, at: 0 };
        let expr = parser.selector(0)?;
        match parser.tokens.get(parser.at) {
            None => Ok(Selector(expr)),
            // a selector only stops short at a `)`
            Some(close) => Err(unbalanced_close(close)),
        }
    }

    /// Whether the scope stack, outermost scope first, matches.
    pub fn matches(&self, stack: &[Scope]) -> bool {
        self.0.matches(stack)
    }
}

impl Expr {
    fn matches(&self, stack: &[Scope]) -> bool {
        match self {
            Expr::Nothing => false,
            Expr::Path(path) => {
                let mut next = 0;
                stack.iter().any(|scope| {
                    next += usize::from(path[next].is_prefix_of(*scope));
                    next == path.len()
                })
            }
            Expr::Not(inner) => !inner.matches(stack),
            Expr::And(all) => all.iter().all(|e| e.matches(stack)),
            Expr::Or(any) => any.iter().any(|e| e.matches(stack)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind<'a> {
    Ident(&'a str),
    Minus,
    And,
    /// `,` or `|`.
    Or(char),
    Open,
    Close,
}

impl Kind<'_> {
    fn text(self) -> String {
        match self {
            Kind::Ident(name) => name.into(),
            Kind::Minus => "-".into(),
            Kind::And => "&".into(),
            Kind::Or(c) => c.into(),
            Kind::Open => "(".into(),
            Kind::Close => ")".into(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Token<'a> {
    kind: Kind<'a>,
    /// Characters before it, from 1.
    column: usize,
}

fn ident_start(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '+')
}

fn lex(src: &str) -> Result<Vec<Token<'_>>, String> {
    let mut tokens = Vec::new();
    let mut chars = src.char_indices().peekable();
    let mut column = 0;
    while let Some((start, c)) = chars.next() {
        column += 1;
        let kind = match c {
            c if c.is_whitespace() => continue,
            '(' => Kind::Open,
            ')' => Kind::Close,
            '&' => Kind::And,
            ',' | '|' => Kind::Or(c),
            '-' => Kind::Minus,
            c if ident_start(c) => {
                // identifiers are ASCII, so a character is a byte
                let mut end = start + 1;
                while let Some(&(at, next)) = chars.peek() {
                    if !(ident_start(next) || next == '-') {
                        break;
                    }
                    end = at + 1;
                    chars.next();
                }
                let kind = Kind::Ident(&src[start..end]);
                tokens.push(Token { kind, column });
                column += end - start - 1;
                continue;
            }
            c => return Err(format!("unexpected character `{c}` at column {column}")),
        };
        tokens.push(Token { kind, column });
    }
    Ok(tokens)
}

fn unbalanced_close(token: &Token) -> String {
    format!("unbalanced `)` at column {}: no opening `(`", token.column)
}

struct Parser<'a, 'b> {
    tokens: &'b [Token<'a>],
    at: usize,
}

impl<'a> Parser<'a, '_> {
    fn kind(&self) -> Option<Kind<'a>> {
        self.tokens.get(self.at).map(|t| t.kind)
    }

    fn selector(&mut self, depth: usize) -> Result<Expr, String> {
        let mut any = vec![self.conjunction(depth)?];
        while let Some(Kind::Or(_)) = self.kind() {
            while let Some(Kind::Or(_)) = self.kind() {
                self.at += 1;
            }
            any.push(self.conjunction(depth)?);
        }
        Ok(match any.len() {
            1 => any.remove(0),
            _ => Expr::Or(any),
        })
    }

    fn conjunction(&mut self, depth: usize) -> Result<Expr, String> {
        let mut all = vec![self.operand(depth)?];
        loop {
            match self.kind() {
                Some(Kind::And) => {
                    self.at += 1;
                    all.push(self.operand(depth)?);
                }
                Some(Kind::Minus | Kind::Open | Kind::Ident(_)) => all.push(self.operand(depth)?),
                _ => break,
            }
        }
        Ok(match all.len() {
            1 => all.remove(0),
            _ => Expr::And(all),
        })
    }

    fn operand(&mut self, depth: usize) -> Result<Expr, String> {
        let Some(&token) = self.tokens.get(self.at) else {
            return Err(self.no_operand());
        };
        match token.kind {
            Kind::Minus | Kind::Open if depth >= MAX_NESTING => Err(format!(
                "`{}` at column {} is nested more than {MAX_NESTING} deep",
                token.kind.text(),
                token.column
            )),
            Kind::Minus => {
                self.at += 1;
                Ok(Expr::Not(Box::new(self.operand(depth + 1)?)))
            }
            Kind::Open => {
                self.at += 1;
                let inner = self.selector(depth + 1)?;
                match self.kind() {
                    Some(Kind::Close) => {
                        self.at += 1;
                        Ok(inner)
                    }
                    _ => Err(format!("unbalanced `(` at column {}: no closing `)`", token.column)),
                }
            }
            Kind::Ident(_) => {
                let mut path = Vec::new();
                while let Some(&Token { kind: Kind::Ident(name), column }) = self.tokens.get(self.at) {
                    path.push(scope(name, column)?);
                    self.at += 1;
                }
                Ok(Expr::Path(path))
            }
            _ => Err(self.no_operand()),
        }
    }

    /// The error for an operand wanted at the cursor and not there: only an operator, a `(` or
    /// the start of the selector comes before one.
    fn no_operand(&self) -> String {
        let after = self.at.checked_sub(1).map(|i| self.tokens[i]);
        match (after, self.tokens.get(self.at)) {
            (Some(open), _) if open.kind == Kind::Open => {
                format!("`(` at column {} has nothing inside", open.column)
            }
            (Some(op), _) => format!("`{}` at column {} has no scope after it", op.kind.text(), op.column),
            (None, Some(close)) if close.kind == Kind::Close => unbalanced_close(close),
            (None, Some(op)) => format!("`{}` at column {} has no scope before it", op.kind.text(), op.column),
            (None, None) => "the selector has no scope".into(),
        }
    }
}

fn scope(name: &str, column: usize) -> Result<Scope, String> {
    // `source..python`, `.x` and `x.` name no scope a stack has: a mistake, not a match of nothing
    if name.split('.').any(str::is_empty) {
        return Err(format!("`{name}` at column {column} has an empty part between dots"));
    }
    if name.split('.').count() > MAX_PARTS {
        return Err(format!("`{name}` at column {column} has more than {MAX_PARTS} parts"));
    }
    Scope::new(name).map_err(|e| format!("`{name}` at column {column}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(names: &[&str]) -> Vec<Scope> {
        names.iter().map(|n| Scope::new(n).unwrap()).collect()
    }

    fn sel(src: &str) -> Selector {
        Selector::parse(src).unwrap_or_else(|e| panic!("{src:?} failed: {e}"))
    }

    fn hit(src: &str, names: &[&str]) -> bool {
        sel(src).matches(&stack(names))
    }

    fn error(src: &str) -> String {
        match Selector::parse(src) {
            Err(e) => e,
            Ok(s) => panic!("{src:?} parsed as {s:?}"),
        }
    }

    #[test]
    fn a_path_is_a_descendant_chain_of_dotted_prefixes() {
        assert!(hit("source.python", &["source.python"]));
        assert!(hit("source.python", &["source.python.embedded"]));
        assert!(!hit("source.python", &["source.pythonx"]));
        assert!(!hit("source.python.embedded", &["source.python"]));
        // anywhere in the stack
        assert!(hit("source", &["text.html", "source.js"]));
        // in order, not necessarily next to each other
        assert!(hit("a c", &["a", "b", "c"]));
        assert!(!hit("a c", &["c", "b", "a"]));
        assert!(hit("a b c", &["a.x", "b.y", "z", "c.w"]));
        // one scope serves one identifier
        assert!(!hit("a a", &["a.b"]));
        assert!(hit("a a", &["a.b", "a.c"]));
        assert!(hit("a.b a", &["a.b", "a"]));
        // case matters, and the empty stack has nothing to match
        assert!(!hit("Source", &["source"]));
        assert!(!hit("a", &[]));
    }

    #[test]
    fn an_identifier_takes_hyphens_colons_and_pluses_and_a_leading_minus_is_the_operator() {
        assert!(hit("meta.tag-name", &["meta.tag-name.html"]));
        assert!(!hit("meta.tag-name", &["meta.tag"]));
        assert!(hit("source.c++", &["source.c++"]));
        assert!(hit("a:b", &["a:b"]));
        assert!(hit("a -b", &["a"]));
        assert!(!hit("a -b", &["a", "b"]));
        // `a-b` is one identifier, not `a` minus `b`
        assert!(!hit("a-b", &["a"]));
        assert!(hit("a-b", &["a-b"]));
        // a digit or an underscore can start one
        assert!(hit("3d_view", &["3d_view"]));
    }

    #[test]
    fn a_minus_b_or_c_is_a_and_not_b_or_c() {
        let s = sel("a - b | c");
        let m = |names: &[&str]| s.matches(&stack(names));
        assert!(m(&["a"]));
        assert!(!m(&["a", "b"]));
        assert!(m(&["a", "b", "c"]));
        assert!(m(&["c"]));
        assert!(!m(&["x"]));
        assert!(!m(&[]));
        // `,` and `|` are the same, and several in a row are one
        assert!(hit("a, b", &["b"]));
        assert!(hit("a | b", &["b"]));
        assert!(hit("a ,, | b", &["b"]));
        assert!(!hit("a, b", &["c"]));
        // a union is the lowest precedence: `a b, c` is `(a b) or c`
        assert!(hit("a b, c", &["c"]));
        assert!(!hit("a b, c", &["b", "a"]));
        assert!(hit("a & b, c", &["c"]));
    }

    #[test]
    fn operands_side_by_side_and_ampersands_are_all_required() {
        // `a (b | c)` is a and (b or c) in any order; `a b` is a descendant path
        assert!(hit("a (b | c)", &["b", "a"]));
        assert!(!hit("a b", &["b", "a"]));
        assert!(hit("a (b | c)", &["a", "c"]));
        assert!(!hit("a (b | c)", &["a", "d"]));
        assert!(hit("a & b", &["b", "a"]));
        assert!(!hit("a & b", &["a"]));
        assert!(hit("a & -b", &["a"]));
        assert!(!hit("a & -b", &["a", "b"]));
        assert!(hit("(a | b) & c", &["b", "c"]));
        assert!(!hit("(a | b) & c", &["b"]));
        assert!(hit("(a | b) - c - d", &["a"]));
        assert!(!hit("(a | b) - c - d", &["a", "d"]));
        assert!(hit("a & (b | c) - d", &["a", "c"]));
        assert!(!hit("a & (b | c) - d", &["a", "c", "d"]));
        // no white space needed around the operators
        assert!(hit("(a|b)&c", &["a", "c"]));
        assert!(hit("a,b", &["b"]));
    }

    #[test]
    fn a_minus_negates_the_operand_after_it() {
        assert!(hit("-a", &["x"]));
        assert!(hit("-a", &[]));
        assert!(!hit("-a", &["a"]));
        assert!(hit("--a", &["a"]));
        // a whole group, and a whole path
        assert!(!hit("-(constant, entity)", &["source.rust", "entity.name.function"]));
        assert!(!hit("-(constant, entity)", &["constant.numeric"]));
        assert!(hit("-(constant, entity)", &["source.rust", "comment.line"]));
        assert!(!hit("-b c", &["b", "c"]));
        assert!(hit("-b c", &["c", "b"]));
        // unlike `-constant, entity`, which is `(not constant) or entity`
        assert!(hit("-constant, entity", &["constant.numeric", "entity.name"]));
        assert!(!hit("-(constant, entity)", &["constant.numeric", "entity.name"]));
    }

    const SPELLING: &str = "markup.raw, source string.quoted - punctuation - meta.preprocessor.include, source comment - source comment.block.preprocessor, -(constant, entity, keyword, punctuation, storage, support, variable, markup.underline.link, meta.tag)";

    #[test]
    fn the_default_spelling_selector_takes_comments_and_strings_not_code() {
        let s = sel(SPELLING);
        let m = |names: &[&str]| s.matches(&stack(names));
        assert!(m(&["source.rust", "comment.line.double-slash.rust"]));
        assert!(m(&["source.python", "comment.block.documentation.python"]));
        assert!(m(&["source.rust", "string.quoted.double.rust"]));
        assert!(m(&["text.html.markdown", "markup.raw.inline.markdown"]));
        assert!(m(&["text.plain"]));
        assert!(!m(&["source.rust", "keyword.control.rust"]));
        assert!(!m(&["source.rust", "entity.name.function.rust"]));
        assert!(!m(&["source.rust", "variable.other.rust"]));
        assert!(!m(&["source.rust", "constant.numeric.integer.rust"]));
        assert!(!m(&["source.rust", "storage.type.rust"]));
        assert!(!m(&["source.rust", "support.function.rust"]));
        assert!(!m(&["text.html", "meta.tag.html"]));
        // the quote marks of a string are punctuation
        let quote = ["source.rust", "string.quoted.double.rust"];
        assert!(!m(&[quote[0], quote[1], "punctuation.definition.string.begin.rust"]));
        // the same selector over several lines, as a settings file would have it
        let lines = SPELLING.replace(", ", ",\n    ");
        assert!(hit(&lines, &["source.rust", "comment.line.double-slash.rust"]));
        assert!(!hit(&lines, &["source.rust", "keyword.control.rust"]));
    }

    #[test]
    fn any_white_space_separates() {
        assert!(hit("a\n\t- b", &["a"]));
        assert!(!hit("a\n\t- b", &["a", "b"]));
        assert!(hit("  a   b  ", &["a", "b"]));
        assert!(hit("\ta\r\n b", &["a", "b"]));
    }

    #[test]
    fn an_empty_selector_matches_nothing() {
        for src in ["", "   ", "\n\t "] {
            let s = sel(src);
            assert!(!s.matches(&stack(&["source.rust"])), "{src:?}");
            assert!(!s.matches(&[]), "{src:?}");
        }
    }

    #[test]
    fn every_mistake_is_reported_with_its_place() {
        // unbalanced parentheses
        for src in ["(a", "a)", "((a)", "a (b", ")", "a | (b - c", "(a))"] {
            let e = error(src);
            assert!(e.contains("unbalanced"), "{src:?}: {e}");
        }
        assert!(error("a (b").contains("unbalanced `(` at column 3"));
        assert!(error("a)").contains("unbalanced `)` at column 2"));
        assert!(error("(a))").contains("unbalanced `)` at column 4"));
        // an operator with nothing to work on
        for src in
            ["-", "a -", "a - )", "& a", "a &", "a & | b", "a,", ", a", "a |", "(a, )", "- , a", "a - &", "a && b"]
        {
            let e = error(src);
            assert!(e.contains("has no scope"), "{src:?}: {e}");
        }
        assert!(error("a - ").contains("`-` at column 3 has no scope after it"));
        assert!(error("& a").contains("`&` at column 1 has no scope before it"));
        assert!(error("()").contains("`(` at column 1 has nothing inside"));
        assert!(error("a - ()").contains("`(` at column 5 has nothing inside"));
        // characters that are in no selector
        for (src, bad) in [
            ("a @ b", '@'),
            ("a$", '$'),
            ("a/b", '/'),
            ("\"a\"", '"'),
            ("a!b", '!'),
            ("a*", '*'),
            ("é", 'é'),
            ("a #b", '#'),
        ] {
            let e = error(src);
            assert!(e.contains(&format!("unexpected character `{bad}`")), "{src:?}: {e}");
        }
        assert!(error("a @ b").contains("at column 3"));
        assert!(error("é @").contains("at column 1"));
        // an empty part between, before or after the dots
        for src in ["source..python", ".x", "x.", ".", "a b..c", "a.b. c", "-.a"] {
            let e = error(src);
            assert!(e.contains("empty part"), "{src:?}: {e}");
        }
        assert!(error("a source..python").contains("`source..python` at column 3"));
        // a scope name has at most 8 parts
        assert!(hit("a.b.c.d.e.f.g.h", &["a.b.c.d.e.f.g.h"]));
        assert!(error("a.b.c.d.e.f.g.h.i").contains("more than 8 parts"));
    }

    #[test]
    fn selectors_from_sublimes_own_snippets_select_what_they_say() {
        // C and C++: a group, then exclusions; `+` is part of a scope name
        let cpp =
            "(source.c | source.objc | source.c++ | source.objc++) - meta.preprocessor.include - comment - string";
        assert!(hit(cpp, &["source.c++", "meta.function.c++"]));
        assert!(hit(cpp, &["source.objc++", "meta.block.objc++"]));
        assert!(!hit(cpp, &["source.c++", "comment.line.double-slash.c++"]));
        assert!(!hit(cpp, &["source.c++", "meta.preprocessor.include.c++"]));
        assert!(!hit(cpp, &["source.python", "meta.function.python"]));
        // a group and a scope together
        let main = "(source.c | source.objc | source.c++ | source.objc++) & entity.name.function - meta.preprocessor.include - comment - string";
        assert!(hit(main, &["source.c", "meta.function.c", "entity.name.function.c"]));
        assert!(!hit(main, &["source.c", "meta.function.c"]));
        // Go templates: a path inside a group
        let go = "(text.html.go | source.css.go | source.go string) - source.go.template";
        assert!(hit(go, &["text.html.go"]));
        assert!(hit(go, &["source.go", "string.quoted.double.go"]));
        assert!(!hit(go, &["source.go", "string.quoted.double.go", "source.go.template"]));
        assert!(!hit(go, &["source.go", "keyword.go"]));
        // PHP
        let php = "(embedding.php | text.html.php) - source.php";
        assert!(hit(php, &["text.html.php"]));
        assert!(!hit(php, &["text.html.php", "source.php"]));
        // HTML: over lines, with a group negated
        let html = "text.html\n\t\t- text.html meta.tag\n\t\t- text.html string\n\t\t- (text.html.markdown - markup.raw.code-fence text.html)";
        assert!(hit(html, &["text.html.basic"]));
        assert!(!hit(html, &["text.html.basic", "meta.tag.block.any.html"]));
        assert!(!hit(html, &["text.html.markdown", "text.html.basic"]));
        let fenced = ["text.html.markdown", "markup.raw.code-fence.markdown", "text.html.basic"];
        assert!(hit(html, &fenced));
    }

    #[test]
    fn hostile_selectors_are_errors_or_cheap() {
        let deep = format!("{}a{}", "(".repeat(10_000), ")".repeat(10_000));
        assert!(error(&deep).contains("deep"));
        assert!(error(&format!("{}a", "-".repeat(10_000))).contains("deep"));
        let fine = format!("{}a{}", "(".repeat(40), ")".repeat(40));
        assert!(hit(&fine, &["a"]));
        // very wide ones are not deep
        let wide = vec!["a.b"; 20_000].join(", ");
        assert!(!hit(&wide, &["c"]));
        assert!(hit(&wide, &["a.b.c"]));
        let long_path = vec!["a"; 20_000].join(" ");
        assert!(!hit(&long_path, &["a"]));
        let many = vec!["a"; 20_000];
        assert!(hit(&long_path, &many));
    }
}
