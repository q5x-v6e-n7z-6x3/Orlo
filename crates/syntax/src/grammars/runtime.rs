//! Language folders' grammars: WebAssembly parsers run by one wasmtime engine, and the queries a
//! manifest lists. Compiled on the highlight worker, once per registry generation; a failure is
//! reported once and the language is left alone until the next generation.

use super::{builtin_language, builtin_query, compiled, grammar_name, query_text, Compiled, Grammar};
use crate::language::Language;
use crate::manifest::{self, Source};
use crate::registry::{LanguageDef, Registry};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tree_sitter::{wasmtime, Parser, Query, QueryErrorKind, WasmStore};

/// Parsers kept with their store between parses: a store costs about 3.5 ms to make.
const POOL: usize = 4;
/// A store instantiates each language it parses; past this many it is dropped, not kept.
const STORE_LANGUAGES: usize = 16;

fn engine() -> &'static wasmtime::Engine {
    static ENGINE: OnceLock<wasmtime::Engine> = OnceLock::new();
    ENGINE.get_or_init(wasmtime::Engine::default)
}

fn pool() -> &'static Mutex<Vec<Parser>> {
    static PARSERS: Mutex<Vec<Parser>> = Mutex::new(Vec::new());
    &PARSERS
}

/// A parser set to `language`; a wasm one comes with a store, from the pool when it has one.
pub(super) fn parser(language: &tree_sitter::Language) -> Result<Parser, String> {
    let pooled = language.is_wasm().then(|| pool().lock().unwrap_or_else(|e| e.into_inner()).pop()).flatten();
    let mut parser = match pooled {
        Some(parser) => parser,
        None => {
            let mut parser = Parser::new();
            if language.is_wasm() {
                let store = WasmStore::new(engine()).map_err(|e| e.to_string())?;
                parser.set_wasm_store(store).map_err(|e| e.to_string())?;
            }
            parser
        }
    };
    parser.set_language(language).map_err(|e| e.to_string())?;
    Ok(parser)
}

/// A parser a parse is done with: one with a store goes back to the pool.
pub(super) fn give_back(mut parser: Parser) {
    let Some(store) = parser.take_wasm_store() else {
        return;
    };
    if store.language_count() > STORE_LANGUAGES || parser.set_wasm_store(store).is_err() {
        return;
    }
    parser.reset();
    let mut pool = pool().lock().unwrap_or_else(|e| e.into_inner());
    if pool.len() < POOL {
        pool.push(parser);
    }
}

/// `def`'s grammar in `registry`'s generation, made by `make` the first time it is asked for.
pub(super) fn generation(
    registry: &Arc<Registry>,
    def: &Arc<LanguageDef>,
    make: impl FnOnce() -> Option<Grammar>,
) -> Option<Arc<Grammar>> {
    type Generations = Vec<(Weak<Registry>, Vec<Arc<Grammar>>)>;
    static GENERATIONS: Mutex<Generations> = Mutex::new(Vec::new());
    let mut generations = GENERATIONS.lock().unwrap_or_else(|e| e.into_inner());
    generations.retain(|(registry, _)| registry.strong_count() > 0);
    let at = match generations.iter().position(|(r, _)| std::ptr::eq(r.as_ptr(), Arc::as_ptr(registry))) {
        Some(at) => at,
        None => {
            generations.push((Arc::downgrade(registry), Vec::new()));
            generations.len() - 1
        }
    };
    let grammars = &mut generations[at].1;
    if let Some(grammar) = grammars.iter().find(|g| g.language().is_some_and(|l| Arc::ptr_eq(l, def))) {
        return Some(grammar.clone());
    }
    let grammar = Arc::new(make()?);
    grammars.push(grammar.clone());
    Some(grammar)
}

fn failures() -> &'static Mutex<Vec<String>> {
    static FAILURES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    &FAILURES
}

pub(super) fn report(errors: Vec<String>) {
    failures().lock().unwrap_or_else(|e| e.into_inner()).extend(errors);
}

pub(super) fn take_failures() -> Vec<String> {
    std::mem::take(&mut *failures().lock().unwrap_or_else(|e| e.into_inner()))
}

/// The reported failures that mention `text`, left to be taken.
#[cfg(test)]
pub(crate) fn reported(text: &str) -> Vec<String> {
    failures().lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|e| e.contains(text)).cloned().collect()
}

/// The grammar and every query `def` names, compiled; every error, each naming its file. A
/// built-in grammar with exactly its built-in highlights shares the process's copy.
pub(super) fn compile(def: &LanguageDef, builtin: Option<Language>) -> Result<Arc<Compiled>, Vec<String>> {
    let manifest_path = || def.folder.join(manifest::FILE).display().to_string();
    let ts = def.manifest.tree_sitter.as_ref().ok_or_else(|| vec![format!("{}: no tree_sitter", manifest_path())])?;
    let language = match Source::of(&ts.grammar) {
        Source::Builtin(name) => {
            builtin_language(name).ok_or_else(|| format!("{}: Orlo has no built-in grammar {name:?}", manifest_path()))
        }
        Source::File(path) => def.read(path, manifest::GRAMMAR_BYTES).and_then(|bytes| {
            // the language keeps what it needs: parsers instantiate it in their own stores
            let symbol = ts.symbol.clone().unwrap_or_else(|| def.id().replace('-', "_"));
            (WasmStore::new(engine()).and_then(|mut store| store.load_language(&symbol, &bytes)))
                .map_err(|e| format!("{}: {e}", def.folder.join(path).display()))
        }),
    }
    .map_err(|e| vec![e])?;
    let highlights = compose(def, &ts.highlights);
    let shared = builtin.filter(|&l| {
        let same = |h: &Composed| h.text.split_ascii_whitespace().eq(query_text(l).split_ascii_whitespace());
        Source::of(&ts.grammar) == Source::Builtin(grammar_name(l))
            && [&ts.folds, &ts.indents, &ts.tags].iter().all(|q| q.is_none())
            && highlights.as_ref().is_ok_and(same)
    });
    if let Some(language) = shared {
        return Ok(compiled(language).clone());
    }
    let optional = |entry: &Option<String>| {
        entry.as_ref().map(|entry| compose(def, std::slice::from_ref(entry)).and_then(|c| c.query(&language)))
    };
    let highlights = highlights.and_then(|c| c.query(&language));
    let (folds, indents, tags) = (optional(&ts.folds), optional(&ts.indents), optional(&ts.tags));
    let errors: Vec<String> = [highlights.as_ref().err()]
        .into_iter()
        .chain([&folds, &indents, &tags].map(|q| q.as_ref().and_then(|q| q.as_ref().err())))
        .flatten()
        .cloned()
        .collect();
    if !errors.is_empty() {
        return Err(errors);
    }
    // nvim-treesitter's longer capture names: on a built-in parser only as its own queries allow
    let prefixes = !matches!(Source::of(&ts.grammar), Source::Builtin(_)) || super::BUILTIN_PREFIXES;
    let mut compiled = Compiled::new(language, highlights.unwrap(), prefixes);
    let ok = |q: Option<Result<Query, String>>| q.and_then(Result::ok);
    (compiled.folds, compiled.indents, compiled.tags) = (ok(folds), ok(indents), ok(tags));
    Ok(Arc::new(compiled))
}

/// A manifest's list of queries as one text, with where each entry starts in it.
struct Composed {
    text: String,
    /// `(start, file for messages, built in)`.
    pieces: Vec<(usize, String, bool)>,
}

/// The entries joined by line breaks, as Orlo composes its built-in queries.
fn compose(def: &LanguageDef, entries: &[String]) -> Result<Composed, String> {
    let mut composed = Composed { text: String::new(), pieces: Vec::new() };
    for entry in entries {
        if !composed.text.is_empty() {
            composed.text.push('\n');
        }
        let start = composed.text.len();
        match Source::of(entry) {
            Source::Builtin(name) => {
                let manifest = def.folder.join(manifest::FILE);
                let text = builtin_query(name)
                    .ok_or_else(|| format!("{}: Orlo has no built-in query {name:?}", manifest.display()))?;
                composed.text.push_str(text);
                composed.pieces.push((start, entry.clone(), true));
            }
            Source::File(path) => {
                let label = def.folder.join(path).display().to_string();
                let text = String::from_utf8(def.read(path, manifest::QUERY_BYTES)?)
                    .map_err(|_| format!("{label}: is not UTF-8"))?;
                composed.text.push_str(&text);
                composed.pieces.push((start, label, false));
            }
        }
    }
    Ok(composed)
}

impl Composed {
    /// The entry `offset` is in, and the line (from 1) it is on there.
    fn at(&self, offset: usize) -> (&str, usize, bool) {
        let offset = offset.min(self.text.len());
        let i = self.pieces.partition_point(|p| p.0 <= offset).saturating_sub(1);
        let (start, label, builtin) = &self.pieces[i];
        (label, self.text.as_bytes()[*start..offset].iter().filter(|&&b| b == b'\n').count() + 1, *builtin)
    }

    /// Compiled for `language`, refusing predicates the Rust binding would ignore (Neovim's
    /// `#lua-match?`) in the folder's own files.
    fn query(&self, language: &tree_sitter::Language) -> Result<Query, String> {
        let query = Query::new(language, &self.text).map_err(|e| {
            let (label, line, _) = self.at(e.offset);
            let message = e.message.lines().next().unwrap_or_default();
            let what = match e.kind {
                // tree-sitter quotes these names itself
                QueryErrorKind::NodeType => format!("unknown node type {message}"),
                QueryErrorKind::Field => format!("unknown field {message}"),
                QueryErrorKind::Capture => format!("unknown capture {message}"),
                QueryErrorKind::Predicate => format!("invalid predicate: {message}"),
                QueryErrorKind::Structure => "impossible pattern".into(),
                QueryErrorKind::Syntax => "invalid syntax".into(),
                QueryErrorKind::Language => message.into(),
            };
            format!("{label}: line {line}, column {}: {what}", e.column + 1)
        })?;
        for pattern in 0..query.pattern_count() {
            let (label, line, builtin) = self.at(query.start_byte_for_pattern(pattern));
            if let Some(predicate) = query.general_predicates(pattern).first().filter(|_| !builtin) {
                return Err(format!(
                    "{label}: line {line}: Orlo does not support the predicate #{}",
                    predicate.operator
                ));
            }
            // the binding keeps `#is?` and `#is-not?` for its caller, which Orlo never asks
            if let Some((_, is)) = query.property_predicates(pattern).first().filter(|_| !builtin) {
                let operator = if *is { "is?" } else { "is-not?" };
                return Err(format!("{label}: line {line}: Orlo does not support the predicate #{operator}"));
            }
        }
        Ok(query)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};

    /// A support folder whose `Languages/` holds copies of `tests/fixtures/languages`.
    pub struct Support(pub PathBuf);

    impl Support {
        pub fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("orlo-grammar-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(std::fs::canonicalize(path).unwrap())
        }

        pub fn fixture(name: &str) -> PathBuf {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/languages").join(name)
        }

        /// The fixture folder `from` as `Languages/<id>`, its manifest's id changed to match.
        pub fn copy(&self, from: &str, id: &str) -> PathBuf {
            let (source, folder) = (Self::fixture(from), self.0.join("Languages").join(id));
            let mut pending = vec![PathBuf::new()];
            while let Some(relative) = pending.pop() {
                std::fs::create_dir_all(folder.join(&relative)).unwrap();
                for entry in std::fs::read_dir(source.join(&relative)).unwrap().flatten() {
                    let relative = relative.join(entry.file_name());
                    match entry.file_type().unwrap().is_dir() {
                        true => pending.push(relative),
                        false => drop(std::fs::copy(entry.path(), folder.join(&relative)).unwrap()),
                    }
                }
            }
            let manifest = folder.join(crate::manifest::FILE);
            let text = std::fs::read_to_string(&manifest).unwrap();
            std::fs::write(&manifest, text.replace(&format!("\"id\": \"{from}\""), &format!("\"id\": \"{id}\"")))
                .unwrap();
            folder
        }

        /// Write `text` at `relative` in the support folder.
        pub fn write(&self, relative: &str, text: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            path
        }

        /// Edit `Languages/<id>/language.json` as JSON.
        pub fn manifest(&self, id: &str, edit: impl FnOnce(&mut serde_json::Value)) {
            let path = self.0.join("Languages").join(id).join(crate::manifest::FILE);
            let mut manifest: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            edit(&mut manifest);
            std::fs::write(path, manifest.to_string()).unwrap();
        }
    }

    impl Drop for Support {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Support;
    use super::*;
    use crate::registry::Origin;
    use crate::treesitter::State;
    use ropey::Rope;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn prepare(state: &mut State, rope: &Rope) {
        state.prepare(rope, 0..rope.len_lines(), None, || false, crate::highlight::scope_style);
    }

    /// The registry of the support folder's languages, and `id`'s grammar in it.
    fn grammar(support: &Support, id: &str) -> (Arc<Registry>, Arc<Grammar>) {
        let folders = orlo_config::themes::language_folders(&support.0).0;
        let (registry, errors) = Registry::build(folders.into_iter().map(|f| (Origin::User, f)));
        assert!(errors.is_empty(), "{errors:?}");
        let registry = Arc::new(registry);
        let grammar = Grammar::of(&registry, registry.get(id).unwrap()).unwrap();
        (registry, grammar)
    }

    #[track_caller]
    fn foreground_at(state: &State, rope: &Rope, text: &str, token: &str) -> gpui::Hsla {
        let byte = text.find(token).unwrap();
        let line = rope.byte_to_line(byte);
        let (mut end, column) = (0, byte - rope.line_to_byte(line));
        let runs = state.runs(rope, line).expect("tree colours");
        runs.into_iter()
            .find(|r| {
                end += r.0;
                column < end
            })
            .unwrap()
            .1
    }

    #[test]
    fn a_wasm_grammar_highlights_a_file_end_to_end() {
        let support = Support::new("json");
        support.copy("json-wasm", "json-wasm");
        let (registry, grammar) = grammar(&support, "json-wasm");
        let text = "{\"key\": [12, true, null, \"雪\\n\"]}\n";
        let mut rope = Rope::from_str(text);
        let mut state = State::with_grammar(grammar.clone());
        prepare(&mut state, &rope);
        let style = |scope| crate::highlight::scope_style(scope).0;
        assert_eq!(foreground_at(&state, &rope, text, "12"), style("constant.numeric"));
        assert_eq!(foreground_at(&state, &rope, text, "true"), style("constant.language"));
        assert_eq!(foreground_at(&state, &rope, text, "\"雪"), style("string"));
        assert_eq!(foreground_at(&state, &rope, text, "\\n"), style("constant.character.escape"));
        let stack = crate::highlight::scope_stack(state.scopes(&rope, 0).unwrap()[1].1);
        assert_eq!(stack[0].build_string(), "source.json-wasm");
        let compiled = grammar.compiled().unwrap();
        assert!(compiled.language.is_wasm());
        let key = compiled.query.capture_index_for_name("string.special.key").unwrap();
        assert_eq!(compiled.scopes[key as usize], "string.special", "longest listed prefix");
        assert!(state.fold_regions(&rope).is_none(), "no folds.scm: indentation folding");
        assert!(!state.requested && !grammar.failed());
        // an edit reparses with a pooled parser, and the same generation's grammar is shared
        rope.insert(1, "\"more\": 34,\n");
        state.invalidate();
        state.text_changed(&[orlo_editor::text::Change { start: 1, removed: 0, inserted: 12, lines: (0, 1) }]);
        prepare(&mut state, &rope);
        assert_eq!(foreground_at(&state, &rope, &rope.to_string(), "34"), style("constant.numeric"));
        assert!(Arc::ptr_eq(&grammar, &Grammar::of(&registry, registry.get("json-wasm").unwrap()).unwrap()));
    }

    #[test]
    fn a_stop_pauses_a_wasm_parse_and_is_not_a_failure() {
        let support = Support::new("json-pause");
        support.copy("json-wasm", "json-wasm");
        let (_registry, grammar) = grammar(&support, "json-wasm");
        let text = format!("[{}0]\n", "[1, 2, {\"a\": true}],\n".repeat(20_000));
        let rope = Rope::from_str(&text);
        let mut state = State::with_grammar(grammar.clone());
        let calls = std::cell::Cell::new(0);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > 3
        };
        state.prepare(&rope, 0..1, None, stop, crate::highlight::scope_style);
        assert!(state.runs(&rope, 0).is_none() && state.requested, "paused, not given up");
        assert!(!grammar.failed());
        prepare(&mut state, &rope);
        assert!(state.runs(&rope, 20_000).is_some(), "resumed to the end");
    }

    #[test]
    fn broken_grammars_report_one_error_and_leave_plain_text() {
        let truncated = std::fs::read(Support::fixture("json-wasm/grammar.wasm")).unwrap();
        let abi = std::fs::read(Support::fixture("json-abi12.wasm")).unwrap();
        type Break = Box<dyn Fn(&Support, &str)>;
        let cases: [(&str, &str, &str, Break); 6] = [
            (
                "truncated",
                "json-wasm",
                "grammar.wasm: Failed to",
                Box::new(move |s, id| {
                    s.write(&format!("Languages/{id}/grammar.wasm"), &truncated[..truncated.len() / 2]);
                }),
            ),
            (
                "abi",
                "json-wasm",
                "incompatible language ABI version 12",
                Box::new(move |s, id| {
                    s.write(&format!("Languages/{id}/grammar.wasm"), &abi);
                }),
            ),
            (
                "symbol",
                "json-wasm",
                "grammar.wasm: Failed to instantiate",
                Box::new(|s, id| {
                    s.manifest(id, |m| m["tree_sitter"]["symbol"] = json!("yaml"));
                }),
            ),
            (
                "node",
                "json-wasm",
                "highlights.scm: line 2, column 3: unknown node type \"no_such_node\"",
                Box::new(|s, id| {
                    s.write(
                        &format!("Languages/{id}/queries/highlights.scm"),
                        "(string) @string\n (no_such_node) @keyword\n",
                    );
                }),
            ),
            (
                "predicate",
                "json-wasm",
                "highlights.scm: line 2: Orlo does not support the predicate #lua-match?",
                Box::new(|s, id| {
                    let query = "(number) @number\n((string) @string (#lua-match? @string \"^x\"))\n";
                    s.write(&format!("Languages/{id}/queries/highlights.scm"), query);
                }),
            ),
            ("trap", "trap", "grammar.wasm: the grammar failed while parsing", Box::new(|_, _| {})),
        ];
        for (case, fixture, expected, breaks) in cases {
            let support = Support::new(&format!("broken-{case}"));
            let id = format!("{fixture}-{case}");
            let folder = support.copy(fixture, &id);
            breaks(&support, &id);
            let (_registry, grammar) = grammar(&support, &id);
            let rope = Rope::from_str("{\"a\": [1]} abc ! def\n");
            for _ in 0..3 {
                let mut state = State::with_grammar(grammar.clone());
                prepare(&mut state, &rope);
                assert!(state.runs(&rope, 0).is_none(), "{case}: plain text");
                assert!(!state.requested, "{case}: not asked again");
                state.request(&rope, 0);
                assert!(!state.requested, "{case}: not asked again");
            }
            assert!(grammar.failed(), "{case}");
            let reported = reported(&folder.display().to_string());
            assert_eq!(reported.len(), 1, "{case}: {reported:?}");
            assert!(reported[0].contains(expected), "{case}: {reported:?}");
        }
        // a trapped parser is not pooled: the next wasm parse works
        let support = Support::new("after-trap");
        support.copy("json-wasm", "json-wasm");
        let mut state = State::with_grammar(grammar(&support, "json-wasm").1);
        let rope = Rope::from_str("[1]\n");
        prepare(&mut state, &rope);
        assert!(state.runs(&rope, 0).is_some());
    }

    #[test]
    fn folds_scm_folds_and_indents_and_tags_are_compiled() {
        let support = Support::new("json-folds");
        support.copy("json-wasm", "json-wasm");
        support.write("Languages/json-wasm/queries/folds.scm", "[(object) (array)] @fold\n");
        support.write("Languages/json-wasm/queries/indents.scm", "(object) @indent.begin\n");
        support.write("Languages/json-wasm/queries/tags.scm", "(pair key: (string) @name) @definition.field\n");
        support.manifest("json-wasm", |m| {
            m["tree_sitter"]["folds"] = json!("queries/folds.scm");
            m["tree_sitter"]["indents"] = json!("queries/indents.scm");
            m["tree_sitter"]["tags"] = json!("queries/tags.scm");
        });
        let (_registry, grammar) = grammar(&support, "json-wasm");
        let rope = Rope::from_str("{\n  \"a\": [\n    1\n  ], \"b\": [2]\n}\n");
        let mut state = State::with_grammar(grammar.clone());
        prepare(&mut state, &rope);
        let regions = state.fold_regions(&rope).unwrap();
        let outer = regions.at_header(0).unwrap();
        assert_eq!(rope.slice(outer.end..).to_string(), "\n");
        let inner = regions.at_header(1).unwrap();
        assert_eq!(rope.slice(inner.end..).to_string(), ", \"b\": [2]\n}\n");
        assert_eq!(regions.at_level(2), vec![inner]);
        let compiled = grammar.compiled().unwrap();
        assert!(compiled.indents().is_some() && compiled.tags().is_some());
        // a broken optional query fails the language, naming its file
        support.write("Languages/json-wasm/queries/tags.scm", "(pair key: (strin) @name)\n");
        let def = grammar.language().unwrap();
        let errors = compile(def, None).err().unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("tags.scm: line 1"), "{errors:?}");
    }

    #[test]
    fn indents_scm_dedents_and_honours_the_begin_properties() {
        let levels = |indents: &str, text: &str| {
            let support = Support::new("json-indents");
            support.copy("json-wasm", "json-wasm");
            support.write("Languages/json-wasm/queries/indents.scm", indents);
            support.manifest("json-wasm", |m| m["tree_sitter"]["indents"] = json!("queries/indents.scm"));
            let (_registry, grammar) = grammar(&support, "json-wasm");
            let (at, rope) = (text.find('|').unwrap(), Rope::from_str(&text.replace('|', "")));
            let mut state = State::with_grammar(grammar);
            prepare(&mut state, &rope);
            state.indents(&rope).unwrap().levels(at, at, false).unwrap()
        };
        let begin = "[(array) (object)] @indent.begin\n";
        assert_eq!(levels(begin, "{\"a\": 1}|\n"), 0, "a one-line object ends before the new line");
        let immediate = "(array) @indent.begin\n((object) @indent.begin (#set! indent.immediate))\n";
        assert_eq!(levels(immediate, "{\"a\": 1}|\n"), 1);
        assert_eq!(levels(begin, "[1, |{\"a\": 2}]\n"), 1, "an object starting the new line");
        let same_line = "(array) @indent.begin\n((object) @indent.begin (#set! indent.start_at_same_line))\n";
        assert_eq!(levels(same_line, "[1, |{\"a\": 2}]\n"), 1, "on one line, it opens nothing");
        let both = "(array) @indent.begin\n((object) @indent.begin (#set! indent.start_at_same_line) (#set! indent.immediate))\n";
        assert_eq!(levels(both, "[1, |{\"a\": 2}]\n"), 2);
        assert_eq!(levels(begin, "{\n  \"a\": 1|\n}\n"), 1);
        assert_eq!(levels(&format!("{begin}(pair) @indent.dedent\n"), "{\n  \"a\": 1|\n}\n"), 0);
        // captures Orlo does not apply are ignored
        assert_eq!(levels(&format!("{begin}(pair) @indent.align\n"), "{\n  \"a\": 1|\n}\n"), 1);
    }

    #[test]
    fn predicates_orlo_would_ignore_are_refused_in_every_query_file() {
        for (file, query, operator) in [
            ("highlights.scm", "(number) @number\n((string) @string (#is? local))\n", "#is?"),
            ("tags.scm", "((pair key: (string) @name) @definition.field (#is-not? local))\n", "#is-not?"),
            ("folds.scm", "((object) @fold (#strip! @fold \"x\"))\n", "#strip!"),
        ] {
            let support = Support::new("json-is");
            support.copy("json-wasm", "json-wasm");
            support.write(&format!("Languages/json-wasm/queries/{file}"), query);
            let entry = format!("queries/{file}");
            support.manifest("json-wasm", |m| match file {
                "highlights.scm" => {}
                "tags.scm" => m["tree_sitter"]["tags"] = json!(entry),
                _ => m["tree_sitter"]["folds"] = json!(entry),
            });
            let (_registry, grammar) = grammar(&support, "json-wasm");
            let errors = compile(grammar.language().unwrap(), None).err().unwrap();
            let line = if file == "highlights.scm" { 2 } else { 1 };
            let want = format!("{file}: line {line}: Orlo does not support the predicate {operator}");
            assert!(errors.len() == 1 && errors[0].contains(&want), "{errors:?}");
        }
        // `#set!` is a property: indents.scm reads its own
        let support = Support::new("json-set");
        support.copy("json-wasm", "json-wasm");
        support.write("Languages/json-wasm/queries/highlights.scm", "((string) @string (#set! priority 105))\n");
        let (_registry, grammar) = grammar(&support, "json-wasm");
        assert!(compile(grammar.language().unwrap(), None).is_ok());
    }

    #[test]
    fn activation_reports_every_broken_file() {
        let support = Support::new("activate");
        let folder = support.copy("json-wasm", "json-wasm");
        let registry = |support: &Support| grammar(support, "json-wasm").1.language().unwrap().clone();
        assert_eq!(crate::treesitter::activate(&registry(&support)), Ok(()));
        support.write("Languages/json-wasm/queries/highlights.scm", "(nope) @string\n");
        support.write("Languages/json-wasm/queries/folds.scm", "(also_nope) @fold\n");
        support.write(
            "Languages/json-wasm/Broken.sublime-syntax",
            "name: Broken\nscope: source.b\ncontexts:\n  main:\n    - include: scope:source.missing\n",
        );
        support.manifest("json-wasm", |m| {
            m["tree_sitter"]["folds"] = json!("queries/folds.scm");
            m["syntax"] = json!("Broken.sublime-syntax");
        });
        let errors = crate::treesitter::activate(&registry(&support)).unwrap_err();
        assert_eq!(errors.len(), 3, "{errors:?}");
        for file in ["Broken.sublime-syntax", "highlights.scm", "folds.scm"] {
            let path = folder.join("x").with_file_name(file);
            assert!(
                errors.iter().any(|e| e.contains(&*path.display().to_string()) || e.contains(file)),
                "{file}: {errors:?}"
            );
        }
        assert!(reported(&folder.display().to_string()).is_empty(), "validating reports nothing itself");
    }

    #[test]
    fn manifests_composing_the_builtin_queries_share_the_processes_grammar() {
        let rust = ["queries/prelude.scm", "builtin:rust/highlights"];
        let compositions: [(Language, &[&str]); 16] = [
            (Language::Rust, &rust),
            (Language::Go, &["builtin:go/highlights"]),
            (Language::Python, &["builtin:python/highlights"]),
            (Language::JavaScript, &["builtin:javascript/highlights", "builtin:javascript/jsx"]),
            (Language::TypeScript, &["builtin:javascript/highlights", "builtin:typescript/highlights"]),
            (
                Language::Tsx,
                &["builtin:javascript/highlights", "builtin:javascript/jsx", "builtin:typescript/highlights"],
            ),
            (Language::C, &["builtin:c/highlights"]),
            (Language::Cpp, &["builtin:c/highlights", "builtin:cpp/highlights"]),
            (Language::CSharp, &["builtin:csharp/highlights"]),
            (Language::Java, &["builtin:java/highlights"]),
            (Language::Php, &["builtin:php/highlights", "queries/embedded.scm"]),
            (Language::Ruby, &["builtin:ruby/highlights"]),
            (Language::Swift, &["builtin:swift/highlights"]),
            (Language::Kotlin, &["builtin:kotlin/highlights"]),
            (Language::Bash, &["builtin:bash/highlights"]),
            (Language::Hcl, &["builtin:hcl/highlights"]),
        ];
        let support = Support::new("compositions");
        for (language, highlights) in compositions {
            let id = language.registry_id();
            support.write(&format!("Languages/{id}/queries/prelude.scm"), "(identifier) @variable\n");
            support.write(&format!("Languages/{id}/queries/embedded.scm"), "(text) @orlo.embedded\n");
            let manifest = json!({"schema": 1, "id": id, "name": id, "scope": format!("source.{id}"),
                "tree_sitter": {"grammar": format!("builtin:{}", grammar_name(language)), "highlights": highlights}});
            support.write(&format!("Languages/{id}/language.json"), manifest.to_string());
        }
        for (language, _) in compositions {
            let (_registry, grammar) = grammar(&support, language.registry_id());
            assert_eq!(grammar.builtin, Some(language));
            let shared = compile(grammar.language().unwrap(), grammar.builtin).unwrap();
            assert!(Arc::ptr_eq(&shared, compiled(language)), "{language:?}");
        }
        // anything else is compiled for the folder, with the built-in capture scopes
        support.write("Languages/rust/queries/prelude.scm", "(identifier) @variable\n(lifetime) @label\n");
        let (_registry, grammar) = grammar(&support, "rust");
        let own = compile(grammar.language().unwrap(), grammar.builtin).unwrap();
        assert!(!Arc::ptr_eq(&own, compiled(Language::Rust)));
        assert_eq!(own.query.pattern_count(), compiled(Language::Rust).query.pattern_count() + 1);
    }

    /// Release only: `cargo test -p orlo-syntax --release bench_ -- --ignored --nocapture`. The
    /// worker's keystroke-to-colours on 100k lines (reparse and query), wasm against a native
    /// parser, within the UI benchmark's budget for a background job's 60 lines.
    #[test]
    #[ignore]
    fn bench_a_keystroke_recolours_a_wasm_language_as_a_native_one_does() {
        let support = Support::new("bench");
        support.copy("json-wasm", "json-wasm");
        let (_registry, grammar) = grammar(&support, "json-wasm");
        let json = "{\"name\": \"value\", \"list\": [1, 2.5, true, null], \"nested\": {\"k\": \"雪\"}},\n";
        let rust = "fn f(value: &str) -> usize { let list = [1, 2, 3]; value.len() + list.len() }\n";
        let mut medians = Vec::new();
        for (name, mut state, text) in [
            ("wasm json", State::with_grammar(grammar), format!("[\n{}{{}}]\n", json.repeat(100_000))),
            ("native rust", State::new(Language::Rust), rust.repeat(100_000)),
        ] {
            let mut rope = Rope::from_str(&text);
            prepare(&mut state, &rope);
            let mut times = Vec::new();
            for i in 0..20 {
                let at = rope.line_to_char(50_000 + i) + 10;
                rope.insert_char(at, 'x');
                let started = Instant::now();
                state.invalidate();
                state.text_changed(&[orlo_editor::text::Change { start: at, removed: 0, inserted: 1, lines: (0, 0) }]);
                state.prepare(&rope, 49_990..50_050, None, || false, crate::highlight::scope_style);
                times.push(started.elapsed());
                assert!(state.runs(&rope, 50_000).is_some());
            }
            times.sort();
            let median = times[times.len() / 2];
            println!(
                "bench {name}: keystroke to colours, 100k lines: median {median:?}, max {:?}",
                times.last().unwrap()
            );
            assert!(median < Duration::from_millis(120), "{name}: the UI bench's background job budget");
            medians.push(median);
        }
        assert!(medians[0] < medians[1] * 2, "wasm within twice native: {medians:?}");
    }
}
