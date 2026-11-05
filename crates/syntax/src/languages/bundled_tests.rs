//! The languages Orlo ships in `assets/languages/`: they load, they are detected, and the ones
//! with a regex grammar colour their samples.

use crate::manifest::{self, Source};
use crate::registry::{LanguageDef, Origin, Registry};
use std::path::Path;
use std::sync::Arc;
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet, SyntaxSetBuilder};

fn bundled() -> Registry {
    let (registry, errors) = Registry::bundled();
    assert!(errors.is_empty(), "{errors:?}");
    registry
}

fn language<'a>(registry: &'a Registry, id: &str) -> &'a Arc<LanguageDef> {
    registry.get(id).unwrap_or_else(|| panic!("{id} is not bundled"))
}

fn detected<'a>(registry: &'a Registry, path: &str, head: &str) -> Option<&'a str> {
    registry.detect(Some(Path::new(path)), head).map(|l| l.id())
}

#[test]
fn the_new_languages_load_with_every_file_they_name() {
    let registry = bundled();
    for id in ["pony", "nim", "rux"] {
        let language = language(&registry, id);
        assert_eq!(language.origin, Origin::Bundled);
        assert_eq!(language.manifest.scope, format!("source.{id}"));
        for (file, limit) in language.manifest.files() {
            let bytes = language.read(file, limit).unwrap();
            if file.ends_with(".wasm") {
                assert_eq!(&bytes[..4], b"\0asm", "{id}/{file}");
            }
        }
        let read = |name: &str| language.read(name, 1 << 20).map(|bytes| bytes.len());
        assert!(read("LICENSE").unwrap() > 1000, "{id} needs its licence record");
        let sample = format!("samples/sample.{id}");
        assert!(read(&sample).unwrap() > 0, "{id} needs {sample}");
    }
}

#[test]
fn pony_and_nim_name_their_servers_grammars_and_formatter() {
    let registry = bundled();
    let pony = &language(&registry, "pony").manifest;
    let lsp = pony.lsp.as_ref().unwrap();
    // since ponyc 0.67.0, pony-lsp exits on an argument it doesn't know, such as `--stdio`
    assert_eq!(lsp.command.as_deref(), Some(&["pony-lsp".to_string()][..]));
    assert_eq!((pony.server(), lsp.language_id.as_str()), (Some("pony"), "pony"));
    assert_eq!(lsp.root_markers, ["corral.json", "bundle.json"]);
    assert_eq!(pony.files.extensions, ["pony"]);
    assert!(pony.formatter.is_none());
    let ts = pony.tree_sitter.as_ref().unwrap();
    assert!([&ts.folds, &ts.indents, &ts.tags].iter().all(|q| q.is_some()));

    let nim = &language(&registry, "nim").manifest;
    let lsp = nim.lsp.as_ref().unwrap();
    assert_eq!(lsp.command.as_deref(), Some(&["nimlangserver".to_string()][..]));
    assert_eq!((nim.server(), lsp.language_id.as_str()), (Some("nim"), "nim"));
    assert_eq!(lsp.root_markers, ["*.nimble", "nim.cfg", "config.nims"]);
    assert_eq!(nim.files.extensions, ["nim", "nims", "nimble"]);
    assert_eq!(nim.formatter.as_ref().unwrap().command, ["nph", "-"]);
    let ts = nim.tree_sitter.as_ref().unwrap();
    assert!(ts.folds.is_some() && ts.indents.is_none() && ts.tags.is_none());

    for manifest in [pony, nim] {
        let ts = manifest.tree_sitter.as_ref().unwrap();
        assert_eq!(Source::of(&ts.grammar), Source::File("grammar.wasm"));
        // scripts/build-grammar.sh fetches exactly this commit
        let source = ts.source.as_ref().unwrap();
        assert!(source.repository.starts_with("https://github.com/"), "{}", source.repository);
        assert!(source.revision.len() == 40 && source.revision.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}

#[test]
fn queries_use_only_predicates_and_indent_captures_orlo_applies() {
    let registry = bundled();
    for id in ["pony", "nim"] {
        let language = language(&registry, id);
        let ts = language.manifest.tree_sitter.as_ref().unwrap();
        let queries = ts.highlights.iter().chain([&ts.folds, &ts.indents, &ts.tags].into_iter().flatten());
        for query in queries {
            let text = String::from_utf8(language.read(query, manifest::QUERY_BYTES).unwrap()).unwrap();
            for (at, _) in text.match_indices("(#") {
                let predicate: String = text[at + 1..].chars().take_while(|c| !c.is_whitespace()).collect();
                assert!(
                    ["#eq?", "#not-eq?", "#match?", "#not-match?", "#any-of?", "#not-any-of?"]
                        .contains(&predicate.as_str()),
                    "{id}/{query}: {predicate}"
                );
            }
            if query.ends_with("indents.scm") {
                for capture in text.split('@').skip(1) {
                    let capture: String = capture.chars().take_while(|c| !c.is_whitespace() && *c != ')').collect();
                    assert!(
                        ["indent.begin", "indent.end", "indent.branch", "indent.dedent"].contains(&capture.as_str()),
                        "{id}/{query}: @{capture}"
                    );
                }
            }
        }
    }
}

#[test]
fn pony_and_nim_files_are_detected_by_extension() {
    let registry = bundled();
    for (path, id) in [("a.pony", "pony"), ("A.PONY", "pony"), ("a.nim", "nim"), ("a.nims", "nim"), ("x.nimble", "nim")]
    {
        assert_eq!(detected(&registry, path, ""), Some(id), "{path}");
    }
    assert_eq!(detected(&registry, "a.ponyx", ""), None);
}

#[test]
fn rux_claims_rux_files_except_the_ui_languages() {
    let registry = bundled();
    let sample = String::from_utf8(language(&registry, "rux").read("samples/sample.rux", 1 << 20).unwrap()).unwrap();
    for head in [sample.as_str(), "", "\n", "  \n\t", "module App;\n", "\n  // a comment\n", "/* <b> */"] {
        assert_eq!(detected(&registry, "main.rux", head), Some("rux"), "{head:?}");
    }
    // the Rux UI language's files start with `<template>` or a comment before one
    for head in ["<template>\n  <div/>\n</template>\n", "<!-- App -->\n<template/>", "\n\n  <script>"] {
        assert_eq!(detected(&registry, "App.rux", head), None, "{head:?}");
    }
}

fn rux_syntaxes() -> SyntaxSet {
    let registry = bundled();
    let rux = language(&registry, "rux");
    assert_eq!(rux.manifest.syntax.as_deref(), Some("Rux.sublime-syntax"));
    let source = rux.read("Rux.sublime-syntax", manifest::SYNTAX_BYTES).unwrap();
    let source = String::from_utf8(source).unwrap();
    let definition = syntect::parsing::SyntaxDefinition::load_from_str(&source, true, Some("Rux")).unwrap();
    let mut builder = SyntaxSetBuilder::new();
    builder.add(definition);
    builder.build()
}

/// The scopes in force where `needle` (its first occurrence) starts in `text`.
fn scopes_at(set: &SyntaxSet, text: &str, needle: &str) -> String {
    let target = text.find(needle).unwrap_or_else(|| panic!("{needle:?} not in the text"));
    let mut state = ParseState::new(&set.syntaxes()[0]);
    let mut stack = ScopeStack::new();
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        for (at, op) in state.parse_line(line, set).unwrap() {
            if line_start + at > target {
                break;
            }
            stack.apply(&op).unwrap();
        }
        if target < line_start + line.len() {
            break;
        }
        line_start += line.len();
    }
    stack.as_slice().iter().map(|s| s.build_string()).collect::<Vec<_>>().join(" ")
}

#[test]
fn the_rux_syntax_compiles_alone_and_claims_no_extension() {
    let set = rux_syntaxes();
    assert!(set.find_unlinked_contexts().is_empty(), "{:?}", set.find_unlinked_contexts());
    let syntax = &set.syntaxes()[0];
    assert_eq!((syntax.name.as_str(), syntax.scope.build_string()), ("Rux", "source.rux".to_string()));
    // detection belongs to language.json, whose content rule leaves UI-language files alone
    assert!(syntax.file_extensions.is_empty(), "{:?}", syntax.file_extensions);
}

#[test]
fn the_rux_syntax_colours_every_keyword_of_the_lexer() {
    let set = rux_syntaxes();
    // Compiler/Lexer/Token.cpp at 579a62f3, which has six more than the docs' 38
    let keywords = "as break catch const continue defer do else enum extend extern fail false finally for func if \
                    import in interface intrinsic is let loop match module move none null pub return self struct \
                    throw throws true try type union unsafe var variant when while";
    assert_eq!(keywords.split(' ').count(), 44);
    for keyword in keywords.split(' ') {
        let line = format!("x {keyword};\n");
        let scopes = scopes_at(&set, &line, keyword);
        assert!(
            ["keyword.", "storage.modifier.", "constant.language.", "variable.language."]
                .iter()
                .any(|s| scopes.contains(s)),
            "{keyword}: {scopes}"
        );
    }
    for (text, word) in [
        ("asm func Syscall();", "asm"),
        ("thread var Seen: int = 0;", "thread"),
        ("sizeof(T)", "sizeof"),
        ("alignof(T)", "alignof"),
        ("-> Self", "Self"),
        ("x is .Some(v)", "Some"),
        ("return .Failure(e);", "Failure"),
    ] {
        assert!(!scopes_at(&set, text, word).ends_with("source.rux"), "{word} in {text}");
    }
    // a keyword inside a name, or a contextual word out of its context, is a name
    for (text, word) in [("format(x)", "format"), ("let thread = 1;", "thread"), ("let iffy = 1;", "iffy")] {
        let scopes = scopes_at(&set, text, word);
        assert!(!scopes.contains("keyword") && !scopes.contains("storage"), "{word} in {text}: {scopes}");
    }
}

#[test]
fn the_rux_syntax_colours_comments_strings_numbers_and_types() {
    let set = rux_syntaxes();
    let registry = bundled();
    let sample = String::from_utf8(language(&registry, "rux").read("samples/sample.rux", 1 << 20).unwrap()).unwrap();
    for (needle, scope) in [
        ("still a comment", "comment.block.rux"),
        ("nested", "comment.block.rux comment.block.rux"),
        ("Documentation block", "comment.block.documentation.rux"),
        ("A shape with", "comment.line.documentation.rux"),
        ("Shapes, their", "comment.line.double-slash.rux"),
        ("pub interface", "storage.modifier.rux"),
        ("Shape {", "entity.name.type.rux"),
        ("Area(&self) -> float64 {", "entity.name.function.rux"),
        ("float64 {", "support.type.rux"),
        ("float80 =", "support.type.rux"),
        ("char8[..]", "support.type.rux"),
        ("3.141_592e0", "constant.numeric.decimal.rux"),
        ("0x10u64", "constant.numeric.hex.rux"),
        ("u64;", "constant.numeric.suffix.rux"),
        ("0b1010_1010i32", "constant.numeric.binary.rux"),
        ("0o17", "constant.numeric.octal.rux"),
        ("2.5f32", "constant.numeric.decimal.rux"),
        ("0 as float80", "constant.numeric.integer.rux"),
        ("\\t'", "string.quoted.single.rux constant.character.escape.rux"),
        ("c32\"", "storage.type.string.rux"),
        ("wide \\u", "string.quoted.double.rux"),
        ("\\u{1F600}", "constant.character.escape.rux"),
        ("\\n\")", "constant.character.escape.rux"),
        ("NoReturn", "entity.other.attribute-name.rux"),
        ("target.arch", "variable.language.compile-time.rux"),
        ("none;", "constant.language.rux"),
        ("Parse(text", "entity.name.function.rux"),
        ("PrintLine(\"zero", "variable.function.rux"),
    ] {
        let scopes = scopes_at(&set, &sample, needle);
        assert!(scopes.ends_with(scope), "{needle:?}: {scopes}");
    }
    // after the outer comment closes, and after a string with an escaped quote, code is code again
    let code = "/* a /* b */ c */ let d = \"q\\\"q\"; let e = 1;\n";
    assert!(scopes_at(&set, code, " c ").ends_with("comment.block.rux"));
    assert_eq!(scopes_at(&set, code, "let d"), "source.rux keyword.declaration.rux");
    assert_eq!(scopes_at(&set, code, "let e"), "source.rux keyword.declaration.rux");
    // an unterminated string ends with its line
    let open = "let s = \"open\nlet t = 1;\n";
    assert_eq!(scopes_at(&set, open, "let t"), "source.rux keyword.declaration.rux");
}
