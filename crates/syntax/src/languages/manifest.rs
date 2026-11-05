//! `language.json`: one language's definition, checked field by field before the registry
//! takes it. The file is strict JSON and unknown fields are errors, so a typo is reported
//! rather than ignored.

use serde::Deserialize;
use serde_json::Value;
use std::path::{Component, Path};

pub const FILE: &str = "language.json";
/// The manifest format this Orlo reads; a newer one is reported, not half-read.
pub const SCHEMA: u64 = 1;

pub const MANIFEST_BYTES: u64 = 256 << 10;
pub const SYNTAX_BYTES: u64 = 4 << 20;
pub const GRAMMAR_BYTES: u64 = 32 << 20;
pub const QUERY_BYTES: u64 = 1 << 20;
/// Detection patterns are short; a longer one is more likely a mistake than a need.
const PATTERN_CHARS: usize = 1024;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u64,
    pub id: String,
    pub name: String,
    /// Detected, but left out of the syntax menu and name lookups (`hidden` in a Sublime syntax).
    #[serde(default)]
    pub hidden: bool,
    pub scope: String,
    #[serde(default)]
    pub files: Files,
    #[serde(default)]
    pub comments: Comments,
    pub indent: Option<Indent>,
    /// A `.sublime-syntax` in the folder, or `builtin:<name>` for one of syntect's.
    pub syntax: Option<String>,
    pub tree_sitter: Option<TreeSitter>,
    pub lsp: Option<Lsp>,
    pub formatter: Option<Formatter>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Files {
    /// Without the dot; compared ignoring ASCII case, as syntect compares them, though an
    /// exact-case claim wins (`C` is C++'s, `c` C's).
    pub extensions: Vec<String>,
    /// Whole file names (`Makefile`), compared like extensions.
    pub names: Vec<String>,
    /// Case-sensitive EditorConfig globs; without `/` matches a basename, otherwise a full path.
    pub globs: Vec<String>,
    /// Tried on the first line when no file claim matches (including shebangs).
    pub first_line: Option<String>,
    /// Tried on the start of the file to settle an extension several languages claim.
    pub content: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Comments {
    pub line: Option<String>,
    pub block: Option<[String; 2]>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Indent {
    pub tab_size: Option<usize>,
    pub translate_tabs_to_spaces: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TreeSitter {
    /// `grammar.wasm` in the folder, or `builtin:<name>` for a grammar compiled into Orlo.
    pub grammar: String,
    /// The grammar's exported name (`tree_sitter_<symbol>`); the id by default.
    pub symbol: Option<String>,
    /// Concatenated in order; later patterns win ties.
    pub highlights: Vec<String>,
    pub folds: Option<String>,
    pub indents: Option<String>,
    pub tags: Option<String>,
    /// Where a bundled `.wasm` was built from.
    pub source: Option<GrammarSource>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GrammarSource {
    pub repository: String,
    pub revision: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Lsp {
    pub language_id: String,
    /// Languages naming the same server share its session in a project; the id by default.
    pub server: Option<String>,
    pub command: Option<Vec<String>>,
    pub initialization_options: Option<Value>,
    /// File names marking a project root; one leading `*` is allowed (`*.nimble`).
    #[serde(default)]
    pub root_markers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Formatter {
    /// Arguments, not a shell line: the document goes to stdin, the result comes from stdout.
    pub command: Vec<String>,
}

/// What a manifest field names: something compiled into Orlo, or a file in the folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source<'a> {
    Builtin(&'a str),
    File(&'a str),
}

impl<'a> Source<'a> {
    pub fn of(value: &'a str) -> Self {
        value.strip_prefix("builtin:").map_or(Self::File(value), Self::Builtin)
    }
}

impl Manifest {
    /// Read and check a manifest. Files it names are checked by the caller, which knows the
    /// folder.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        match value.get("schema").map(Value::as_u64) {
            None if !value.is_object() => return Err("expected a JSON object".into()),
            None => return Err("missing field `schema`".into()),
            Some(Some(SCHEMA)) => {}
            Some(Some(n)) if n > SCHEMA => {
                return Err(format!("schema {n} needs a newer Orlo (this one reads schema {SCHEMA})"))
            }
            Some(_) => return Err(format!("`schema` must be {SCHEMA}")),
        }
        // parsed again from the text, so serde's errors keep their line and column
        let manifest: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        manifest.check()?;
        Ok(manifest)
    }

    /// The server name: the languages that name the same one share its session.
    pub fn server(&self) -> Option<&str> {
        let lsp = self.lsp.as_ref()?;
        Some(lsp.server.as_deref().unwrap_or(&self.id))
    }

    /// Every file in the folder the manifest names, with its size limit.
    pub fn files(&self) -> Vec<(&str, u64)> {
        let mut named = vec![(self.syntax.as_deref(), SYNTAX_BYTES)];
        if let Some(ts) = &self.tree_sitter {
            named.push((Some(&ts.grammar), GRAMMAR_BYTES));
            named.extend(ts.highlights.iter().map(|q| (Some(q.as_str()), QUERY_BYTES)));
            for query in [&ts.folds, &ts.indents, &ts.tags] {
                named.push((query.as_deref(), QUERY_BYTES));
            }
        }
        named
            .into_iter()
            .filter_map(|(value, limit)| match Source::of(value?) {
                Source::File(path) => Some((path, limit)),
                Source::Builtin(_) => None,
            })
            .collect()
    }

    fn check(&self) -> Result<(), String> {
        if !is_id(&self.id) {
            return Err(format!(
                "id {:?} must be 1-40 lowercase letters, digits, `-` or `_`, starting with a letter or digit",
                self.id
            ));
        }
        if self.name.trim().is_empty()
            || self.name.trim() != self.name
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err(format!(
                "name {:?} must be 1-80 characters, without control characters or surrounding spaces",
                self.name
            ));
        }
        check_scope(&self.scope)?;
        for extension in &self.files.extensions {
            // inner dots are fine (`d.ts`); detection tries the longest ending first
            if !is_file_name(extension) || extension.split('.').any(str::is_empty) {
                return Err(format!("extension {extension:?} must be a file name ending, without the dot"));
            }
        }
        for name in &self.files.names {
            if !is_file_name(name) {
                return Err(format!("file name {name:?} must not contain `/`"));
            }
        }
        for pattern in &self.files.globs {
            if pattern.is_empty() {
                return Err("files.globs: a pattern must not be empty".into());
            }
            orlo_config::settings::editorconfig::validate_pattern(pattern).map_err(|e| format!("files.globs: {e}"))?;
        }
        for (field, pattern) in [("first_line", &self.files.first_line), ("content", &self.files.content)] {
            if let Some(pattern) = pattern {
                pattern_regex(pattern).map_err(|e| format!("files.{field}: {e}"))?;
            }
        }
        let line = |s: &str| !s.is_empty() && !s.contains(['\n', '\r']);
        if self.comments.line.as_deref().is_some_and(|s| !line(s))
            || (self.comments.block.as_ref()).is_some_and(|[a, b]| !line(a) || !line(b))
        {
            return Err("comment markers must be non-empty and on one line".into());
        }
        if let Some(size) = self.indent.as_ref().and_then(|i| i.tab_size) {
            if !(1..=16).contains(&size) {
                return Err(format!("indent.tab_size {size} must be 1-16"));
            }
        }
        if let Some(syntax) = &self.syntax {
            check_source("syntax", syntax, ".sublime-syntax", |name| !name.is_empty())?;
        }
        if let Some(ts) = &self.tree_sitter {
            self.check_tree_sitter(ts)?;
        }
        if let Some(lsp) = &self.lsp {
            check_lsp(lsp)?;
        }
        if let Some(formatter) = &self.formatter {
            check_command("formatter.command", &formatter.command)?;
        }
        Ok(())
    }

    fn check_tree_sitter(&self, ts: &TreeSitter) -> Result<(), String> {
        use crate::treesitter::{BUILTIN_GRAMMARS, BUILTIN_QUERIES};
        check_source("tree_sitter.grammar", &ts.grammar, ".wasm", |name| BUILTIN_GRAMMARS.contains(&name))?;
        if let Some(symbol) = &ts.symbol {
            let mut chars = symbol.chars();
            if symbol.len() > 64
                || !chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(format!("tree_sitter.symbol {symbol:?} must be a C identifier"));
            }
        }
        if ts.highlights.is_empty() {
            return Err("tree_sitter.highlights must name at least one query".into());
        }
        for query in &ts.highlights {
            check_source("tree_sitter.highlights", query, ".scm", |name| BUILTIN_QUERIES.contains(&name))?;
        }
        for (field, query) in
            [("tree_sitter.folds", &ts.folds), ("tree_sitter.indents", &ts.indents), ("tree_sitter.tags", &ts.tags)]
        {
            if let Some(query) = query {
                check_source(field, query, ".scm", |_| false)?;
            }
        }
        if let Some(source) = &ts.source {
            if !source.repository.starts_with("https://") || source.repository.contains(char::is_whitespace) {
                return Err("tree_sitter.source.repository must be an https URL".into());
            }
            if source.revision.is_empty()
                || source.revision.len() > 100
                || source.revision.contains(char::is_whitespace)
            {
                return Err("tree_sitter.source.revision must be a tag or commit".into());
            }
        }
        Ok(())
    }
}

fn check_lsp(lsp: &Lsp) -> Result<(), String> {
    if lsp.language_id.is_empty() || lsp.language_id.len() > 64 || lsp.language_id.contains(char::is_whitespace) {
        return Err("lsp.language_id must be 1-64 characters without spaces".into());
    }
    if let Some(server) = &lsp.server {
        if !is_id(server) {
            return Err(format!("lsp.server {server:?} must be 1-40 lowercase letters, digits, `-` or `_`"));
        }
    }
    if let Some(command) = &lsp.command {
        check_command("lsp.command", command)?;
    }
    if lsp.initialization_options.as_ref().is_some_and(|o| !o.is_object()) {
        return Err("lsp.initialization_options must be a JSON object".into());
    }
    for marker in &lsp.root_markers {
        let rest = marker.strip_prefix('*').unwrap_or(marker);
        if !is_file_name(marker) || rest.is_empty() || rest.contains('*') {
            return Err(format!("lsp.root_markers entry {marker:?} must be a file name, with at most a leading `*`"));
        }
    }
    Ok(())
}

fn check_command(field: &str, command: &[String]) -> Result<(), String> {
    if command.first().is_none_or(|program| program.trim().is_empty()) {
        return Err(format!("{field} must start with a program name"));
    }
    if command.iter().any(|argument| argument.contains('\0')) {
        return Err(format!("{field} must not contain NUL"));
    }
    Ok(())
}

fn check_scope(scope: &str) -> Result<(), String> {
    let atoms: Vec<_> = scope.split('.').collect();
    let atom = |a: &&str| !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_+".contains(c));
    if atoms.len() > 8 || !atoms.iter().all(atom) || syntect::parsing::Scope::new(scope).is_err() {
        return Err(format!("scope {scope:?} must be dotted words, such as `source.pony`"));
    }
    Ok(())
}

/// A `builtin:` name that `builtin` accepts, or a relative path inside the folder ending in
/// `extension`.
fn check_source(field: &str, value: &str, extension: &str, builtin: impl Fn(&str) -> bool) -> Result<(), String> {
    match Source::of(value) {
        Source::Builtin(name) if builtin(name) => Ok(()),
        Source::Builtin(name) => Err(format!("{field}: Orlo has no built-in {name:?}")),
        Source::File(path) if !path.ends_with(extension) => {
            Err(format!("{field}: {path:?} must be a {extension} file"))
        }
        Source::File(path) => relative(path).map_err(|e| format!("{field}: {e}")),
    }
}

/// A path that stays inside the language's folder: no root, no `..`, no `.`.
pub fn relative(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if path.is_empty() || !p.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!("{path:?} must be a path inside the language's folder"));
    }
    Ok(())
}

/// A detection pattern, compiled with the linear-time engine so no pattern can hang detection.
pub fn pattern_regex(pattern: &str) -> Result<regex_automata::meta::Regex, String> {
    if pattern.chars().count() > PATTERN_CHARS {
        return Err(format!("pattern is longer than {PATTERN_CHARS} characters"));
    }
    regex_automata::meta::Regex::builder()
        .configure(regex_automata::meta::Regex::config().nfa_size_limit(Some(1 << 20)))
        .build(pattern)
        .map_err(|e| e.to_string())
}

fn is_id(s: &str) -> bool {
    (1..=40).contains(&s.len())
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn is_file_name(s: &str) -> bool {
    (1..=255).contains(&s.len()) && !s.contains(['/', '\\']) && !s.chars().any(|c| c.is_control() || c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PONY: &str = r#"{
        "schema": 1,
        "id": "pony",
        "name": "Pony",
        "scope": "source.pony",
        "files": {"extensions": ["pony"]},
        "comments": {"line": "//", "block": ["/*", "*/"]},
        "indent": {"tab_size": 2, "translate_tabs_to_spaces": true},
        "tree_sitter": {
            "grammar": "grammar.wasm",
            "highlights": ["queries/highlights.scm"],
            "folds": "queries/folds.scm",
            "source": {"repository": "https://github.com/tree-sitter-grammars/tree-sitter-pony", "revision": "73ff874a"}
        },
        "lsp": {"language_id": "pony", "command": ["pony-lsp"], "root_markers": ["corral.json", "*.nimble"]},
        "formatter": {"command": ["ponyfmt", "-"]}
    }"#;

    fn with(field: &str, value: Value) -> String {
        let mut manifest: Value = serde_json::from_str(PONY).unwrap();
        let mut slot = &mut manifest;
        let parts: Vec<_> = field.split('.').collect();
        for part in &parts[..parts.len() - 1] {
            slot = &mut slot[*part];
        }
        slot[parts[parts.len() - 1]] = value;
        manifest.to_string()
    }

    #[test]
    fn a_complete_manifest_reads_and_names_its_files() {
        let manifest = Manifest::parse(PONY).unwrap();
        assert_eq!(manifest.server(), Some("pony"));
        assert_eq!(
            manifest.files(),
            [
                ("grammar.wasm", GRAMMAR_BYTES),
                ("queries/highlights.scm", QUERY_BYTES),
                ("queries/folds.scm", QUERY_BYTES)
            ]
        );
        let minimal = Manifest::parse(
            r#"{"schema":1,"id":"rux","name":"Rux","scope":"source.rux","syntax":"Rux.sublime-syntax"}"#,
        )
        .unwrap();
        assert_eq!(minimal.server(), None);
        assert_eq!(minimal.files(), [("Rux.sublime-syntax", SYNTAX_BYTES)]);
    }

    #[test]
    fn built_in_grammars_and_queries_are_not_files() {
        let manifest = Manifest::parse(&with(
            "tree_sitter",
            serde_json::json!({"grammar":"builtin:rust",
                "highlights":["queries/prelude.scm","builtin:rust/highlights"]}),
        ))
        .unwrap();
        assert_eq!(manifest.files(), [("queries/prelude.scm", QUERY_BYTES)]);
        let unknown = with(
            "tree_sitter",
            serde_json::json!({"grammar":"builtin:cobol","highlights":["builtin:rust/highlights"]}),
        );
        assert!(Manifest::parse(&unknown).unwrap_err().contains("\"cobol\""));
    }

    #[test]
    fn added_bundled_grammars_are_available_to_language_manifests() {
        for id in ["c", "cpp", "csharp", "java", "php", "ruby", "swift", "kotlin", "bash", "hcl"] {
            let source = with(
                "tree_sitter",
                serde_json::json!({
                    "grammar": format!("builtin:{id}"),
                    "highlights": [format!("builtin:{id}/highlights")]
                }),
            );
            let manifest = Manifest::parse(&source).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert!(manifest.files().is_empty());
        }
    }

    #[test]
    fn mistakes_are_reported_with_the_field_they_are_in() {
        use serde_json::json;
        for (field, value, expected) in [
            ("files.extention", json!(["x"]), "unknown field `extention`"),
            ("id", json!("Pony"), "id \"Pony\""),
            ("id", json!(""), "id \"\""),
            ("name", json!(" Pony"), "name"),
            ("scope", json!("source pony"), "scope"),
            ("files.extensions", json!([".pony"]), "without the dot"),
            ("files.extensions", json!(["pony."]), "without the dot"),
            ("files.extensions", json!(["d..ts"]), "without the dot"),
            ("files.names", json!(["a/b"]), "file name"),
            ("files.content", json!("(unclosed"), "files.content"),
            ("files.first_line", json!("x".repeat(2000)), "longer than"),
            ("comments.line", json!(""), "comment markers"),
            ("indent.tab_size", json!(0), "1-16"),
            ("syntax", json!("../Pony.sublime-syntax"), "inside the language's folder"),
            ("syntax", json!("/abs/Pony.sublime-syntax"), "inside the language's folder"),
            ("syntax", json!("Pony.tmLanguage"), ".sublime-syntax file"),
            ("tree_sitter.grammar", json!("grammar.so"), ".wasm file"),
            ("tree_sitter.symbol", json!("1pony"), "C identifier"),
            ("tree_sitter.highlights", json!([]), "at least one query"),
            ("tree_sitter.folds", json!("builtin:rust/highlights"), "no built-in"),
            ("tree_sitter.source.repository", json!("git@github.com:x/y"), "https URL"),
            ("lsp.language_id", json!("po ny"), "language_id"),
            ("lsp.server", json!("Pony LSP"), "lsp.server"),
            ("lsp.command", json!([]), "program name"),
            ("lsp.command", json!(["  "]), "program name"),
            ("lsp.command", json!(["server", "bad\0arg"]), "NUL"),
            ("files.globs", json!([""]), "files.globs"),
            ("lsp.initialization_options", json!([1]), "JSON object"),
            ("lsp.root_markers", json!(["a*b"]), "leading `*`"),
            ("lsp.root_markers", json!(["*"]), "leading `*`"),
            ("formatter.command", json!([""]), "program name"),
        ] {
            let error = Manifest::parse(&with(field, value)).unwrap_err();
            assert!(error.contains(expected), "{field}: {error}");
        }
    }

    #[test]
    fn the_schema_is_checked_before_the_fields() {
        let newer = with("schema", serde_json::json!(2)).replace("\"id\"", "\"identity\"");
        assert!(Manifest::parse(&newer).unwrap_err().contains("needs a newer Orlo"));
        let missing = PONY.replace("\"schema\": 1,", "");
        assert_eq!(Manifest::parse(&missing).unwrap_err(), "missing field `schema`");
        assert!(Manifest::parse("[1]").unwrap_err().contains("JSON object"));
        assert!(Manifest::parse("{\"schema\":1,").is_err());
        assert!(Manifest::parse(&with("schema", serde_json::json!("1"))).unwrap_err().contains("must be 1"));
    }

    #[test]
    fn detection_patterns_use_the_linear_engine() {
        let rux = pattern_regex(r"\A\s*(?:[^<\s]|\z)").unwrap();
        assert!(rux.is_match("module main\n"));
        assert!(rux.is_match("\n  // comment"));
        assert!(rux.is_match(""));
        assert!(!rux.is_match("\n<template>\n"));
        // backreferences need backtracking, which the linear engine refuses
        assert!(pattern_regex(r"(a)\1").is_err());
    }
}
