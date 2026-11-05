# Language support plan

Status: approved with the recommended D1–D8 and implemented, stages 1–6, except where
"Deviations from this plan" below says otherwise. Languages are folders (`assets/languages/<id>/`,
embedded by `crates/syntax/build.rs`, and the user's `Languages/<id>/`) read by the registry in
`orlo-syntax`; WebAssembly grammars run through tree-sitter's `wasm` feature; LSP servers,
markers, ids, comments, indentation and formatters come from manifests; Pony, Nim and Rux are
bundled; `Orlo --check-language`, the `docs/templates/example-language` template and
[Adding a language](user-guide/adding-a-language.md) are for contributors. The rest of this
document is the plan as approved, kept for its reasoning; where it and the code disagree, the
code and the user guides are right.

## Deviations from this plan

- **24 variants, not 7.** By the time this was built Orlo had 15 bundled Tree-sitter parsers and
  24 `Language` variants (Bash, C, C++, C#, Go, HCL and seven Terraform dialects, Java,
  JavaScript, JSX, Kotlin, PHP, Python, Ruby, Rust, Swift, TypeScript, TSX). Each has a folder in
  `assets/languages/`, and the golden test in `crates/syntax/src/detection/language.rs` covers all
  of them. `builtin:` offers 16 grammars and 16 highlight queries (`treesitter::BUILTIN_GRAMMARS`
  and `BUILTIN_QUERIES`).
- **The `Language` enum stays.** It is the catalog of parsers compiled into Orlo, behind
  `builtin:<name>`, instead of being deleted for a private `enum Builtin` (8.1, stage 4).
  Registry-first code paths (`ServerLanguage`, `Highlighter::detect`) fall back to it. LSP session
  keys are `(PathBuf, String)`, not `(PathBuf, Arc<str>)`.
- **Rux has 44 keywords**, not 38: the syntax colours `try`, `throw`, `finally`, `unsafe`,
  `throws` and `move` as well (`assets/languages/rux/Rux.sublime-syntax`).
- **Nim is pinned to `main` at `ac72ba30`**, not tag 0.6.2.
- **D7 is implemented with limits.** `indents.scm` drives Enter only, from a tree that is current
  for the text; a line inside a top-level parse error, or a stale tree, falls back to the bracket
  rule. `tags.scm` feeds Goto Symbol, the outline and the project index only when the language's
  syntax is Plain Text (no `.sublime-syntax`), and a tags parse restarts at line 0 after each edit
  (no checkpoints). Nim ships no `tags.scm`, so its Goto Symbol is empty. `#is?` and `#is-not?`
  are refused like other predicates the Rust binding would ignore.
- **The capture-prefix fallback applies only to non-`builtin:` grammars** (8.3): the built-in
  queries keep exact names (`BUILTIN_PREFIXES = false` in `grammars/treesitter.rs`).
- **`files.globs`** (EditorConfig globs) is a manifest field the plan didn't have; detection tries
  names, extensions, globs, then `first_line`.
- **Eager activation after a reload** compiles each user grammar through its generation's
  `Grammar` (so the first buffer reuses it) rather than calling `treesitter::activate`, whose
  `.sublime-syntax` check the reload already did. Failures that happen later, while parsing (a
  trapping scanner), are named by the window's 500 ms poll while it is in front, or by the next
  reload.
- **The template is basic level**: `Example.sublime-syntax` and a `queries/highlights.scm` whose
  example patterns are commented out, since a template can't ship a grammar for them to compile
  against. A unit test in `orlo-app` compiles that query against a built-in grammar.
- **CI checks `assets/languages/*/`**, with the slash, so `assets/languages/README.md` isn't taken
  for a folder.
- **The wasm keystroke check** is `bench_a_keystroke_recolours_a_wasm_language_as_a_native_one_does`
  (ignored, release): on 100k lines a keystroke's background recolouring took 34.5 ms for the
  json-wasm fixture and 49.3 ms for native Rust, within the worker's 120 ms job budget. The 8.3 ms
  frame budget the plan implied does not apply, because this work runs on the highlight worker for
  native grammars too. Opening a wasm-language file in the app and the hardened-runtime result in
  9.2 were checked by hand, not by a test.

Goal: languages become data. Anyone can add one by dropping a folder in place, without editing
Orlo's Rust code or recompiling. The seven languages Orlo has today keep working exactly as they
do now. Orlo still gets no debugger and no plugin runtime with an editor API.

## 1. Summary

- A language is a folder with a `language.json` manifest. It can add three optional pieces on top:
  a `.sublime-syntax` file (basic), a Tree-sitter grammar with query files (good), and a
  language-server command (full). Tree-sitter wins over the regex grammar when a folder has both.
- Folders load from Orlo's bundled `assets/languages/<id>/`, which is compiled into the binary,
  and from `~/Library/Application Support/Orlo/Languages/<id>/`. A user folder with the same id
  replaces the bundled one whole. I recommend a separate `Languages/` folder rather than a
  package field; section 5 gives the reasons.
- A registry in `orlo-syntax` replaces the hard-coded `Language` enum in stages. Until callers
  have moved over, the enum stays as a bridge, and a golden test pins the seven languages to the
  values the enum hard-codes today.
- Runtime grammars are WebAssembly, loaded through tree-sitter 0.27's `wasm` feature. I measured
  the cost on this machine; it is real (about 7 MB of binary, 35 new crates, `cmake` at build
  time, and a hardened-runtime entitlement). The alternative is native shared libraries, which
  cost nothing in size but would let a broken grammar crash Orlo. Section 9 has the numbers.
- The LSP client is already generic. What ties it to four server families is the enum and
  `default_servers()`. Server names become manifest data, and the existing `language_servers`
  keys (`rust`, `go`, `typescript`, `python`) keep their meaning.

## 2. What exists today

These are the places where language knowledge is hard-coded, and so where the plan has to change
code.

| Seam | Where | What it hard-codes |
| --- | --- | --- |
| `enum Language` | `crates/syntax/src/detection/language.rs:10-69` | Seven variants; extension detection (path only); LSP ids; server family; root markers |
| `default_servers()` | `language.rs:73-95` | Commands for the families `rust`, `go`, `typescript` and `python`; Go's `semanticTokens` option |
| Display names, scopes, `named()` | `crates/syntax/src/grammars/treesitter.rs:20-52` | `"TypeScript (TSX)"`, `source.tsx`, and so on |
| Grammars and queries | `treesitter.rs:54-123` | One `OnceLock` per grammar; query concatenation (Rust gets an `(identifier) @variable` prefix; TS/TSX get the JS query first); `.expect()` on query compile (119) and `set_language` (277) |
| Capture-to-scope map | `treesitter.rs:126-155` | Exact capture names only; unknown names map to `""` |
| Highlighter selection | `crates/syntax/src/highlighting/highlight.rs:1393-1430, 1701-1772` | syntect picks the syntax first. Tree-sitter is attached only when that syntax is one of syntect's bundled ones (`index < bundled`), so a user `.sublime-syntax` for `.rs` turns Tree-sitter off. TypeScript has no syntect syntax and uses JavaScript's |
| Syntax menu | `highlight.rs:1735-1747` | Adds `"TypeScript"` and `"TypeScript (TSX)"` by hand |
| Comment markers | `highlight.rs:1342-1384, 1829-1885` | `LINE_COMMENTS` / `BLOCK_COMMENTS`, keyed by syntect syntax name |
| Docstrings | `highlight.rs:838` | `DOCSTRING_SYNTAXES = ["Python"]` |
| LSP sessions | `crates/ui/src/views/workspace/language.rs:48, 118-132, 334, 591-592, 727` | `ServerKey = (PathBuf, &'static str)`; root from `markers()`; `languageId` from the path (not the buffer's syntax); user config merged over `default_servers()`; the error "LSP supports Rust, Go, JavaScript, TypeScript and Python" |
| Navigation gate | `crates/ui/src/views/workspace/goto.rs:66` | `Language::detect(path).is_some()` decides whether to ask a server |
| Real-server smoke test | `crates/syntax/src/languages/lsp/real_servers.rs:293-304` | Takes `default_servers().remove(name)` for the four family names |
| Settings | `crates/config/src/settings/mod.rs:25, 64, 203-232, 663` | `language_servers` is keyed by family. `overrides[].language` matches the display name case-insensitively; `scope` uses base scopes such as `source.rust` |
| Asset loading | `crates/config/src/themes/assets.rs`, `highlight.rs:55-150` | `Syntaxes/`, `Themes/` and enabled packages. Errors are strings shown in the Reload Customizations alert |

Things that work in our favour:

- The LSP transport (`crates/syntax/src/languages/lsp/mod.rs`) already takes any command, root
  and initialization options.
- Asset errors already flow as `Vec<String>` into one alert.
- The highlight worker already slices and cancels parses with a progress callback, so a slower
  grammar never blocks typing.

## 3. Where the requested design doesn't fit Orlo as built

You asked me to say so instead of working around these. None of them is a blocker, but each one
changes the work.

1. **A broken syntax or package already takes others down with it.**
   - If any custom syntax has an unresolved `include`, every custom syntax is dropped
     (`highlight.rs:133-140`).
   - If one enabled package has an invalid `package.json`, every package's syntaxes and themes
     are dropped (`crates/config/src/persistence/automation.rs:417-423`).

   "One broken language folder must never break Orlo" needs per-item isolation. Each language's
   syntax is validated on its own before it joins the shared `SyntaxSet`, and only the offending
   folder is dropped. The package problem is another reason to keep languages out of packages
   (section 5). I would fix the existing all-or-nothing behaviour too, but only if you ask, since
   it changes how today's syntaxes behave.

2. **LSP picks a language by path alone.** `Language::detect(path)` runs again in four places
   in the UI. Content detection (Rux versus the `.rux` UI language) only works if the buffer's
   detected language is computed once and carried to LSP, the gutter and the gates. That is a
   real refactor of `ui/views/workspace/language.rs` (stage 4).

3. **Folding and auto-indent are not syntax-aware anywhere.**
   - Folds come from indentation (`crates/editor/src/rendering/fold.rs:230`).
   - Enter indents only after `{ ( [` (`crates/editor/src/commands/typing.rs:127`).

   Using `folds.scm` and `indents.scm` therefore means new editor behaviour, not just new data.
   The plan uses them only for languages that ship those files. The built-in seven ship neither,
   so their behaviour doesn't change.

4. **WebAssembly grammars need two things outside the code.**
   - `cmake` at build time: the `wasmtime-c-api-impl` build script fails without it (seen here).
   - The entitlement `com.apple.security.cs.allow-unsigned-executable-memory` when signing.
     `scripts/bundle.sh:42` signs with the hardened runtime and no entitlements. A probe signed
     that way was killed with SIGKILL the first time it parsed with a wasm grammar
     (section 9.2). Shipping wasm without changing `bundle.sh` would make opening a Pony file
     kill Orlo.

5. **"No embedded plugin runtime."** Wasmtime is a general WebAssembly runtime. Here it would run
   only a grammar's lexer and scanner, against tree-sitter's fixed libc imports (malloc, mem*,
   str*, isw*/tow*). There is no editor API, no file system and no network. I don't consider that
   a plugin runtime, but it is your rule, so it is decision D1.

## 4. Language folders

### 4.1 Layout

```text
<id>/
  language.json            required
  <Name>.sublime-syntax    optional: basic level (regex highlighting, Goto Symbol scopes)
  grammar.wasm             optional: good level (Tree-sitter)
  queries/highlights.scm   required when grammar.wasm is present
  queries/folds.scm        optional
  queries/indents.scm      optional
  queries/tags.scm         optional (Goto Symbol for languages without a regex grammar; D7)
  LICENSE                  required for bundled folders that contain third-party files
```

The folder name must equal the manifest's `id`. Every path in the manifest is relative to the
folder. A path that is absolute, contains `..`, or resolves through a symlink to somewhere
outside the folder is an error.

Limits: `language.json` at most 256 KiB; each query at most 1 MiB; `grammar.wasm` at most 32 MiB;
at most 256 files per folder. Bundled folders are checked by CI. User folders are checked when
they load.

### 4.2 The manifest

The manifest is strict JSON, like `package.json`, and parsed with serde using
`deny_unknown_fields`, so a typo is reported instead of ignored. `schema` lets a later Orlo say
"this folder needs a newer Orlo" instead of rejecting a new field without explanation.

```json
{
  "schema": 1,
  "id": "pony",
  "name": "Pony",
  "scope": "source.pony",
  "files": {
    "extensions": ["pony"],
    "names": [],
    "first_line": null,
    "content": null
  },
  "comments": { "line": "//", "block": ["/*", "*/"] },
  "indent": { "tab_size": 2, "translate_tabs_to_spaces": true },
  "syntax": null,
  "tree_sitter": {
    "grammar": "grammar.wasm",
    "symbol": "pony",
    "highlights": ["queries/highlights.scm"],
    "folds": "queries/folds.scm",
    "indents": "queries/indents.scm",
    "tags": "queries/tags.scm",
    "source": {
      "repository": "https://github.com/tree-sitter-grammars/tree-sitter-pony",
      "revision": "73ff874a…"
    }
  },
  "lsp": {
    "language_id": "pony",
    "server": "pony",
    "command": ["pony-lsp"],
    "initialization_options": {},
    "root_markers": ["corral.json", "bundle.json"]
  },
  "formatter": null
}
```

| Field | Meaning |
| --- | --- |
| `id` | Unique, `[a-z0-9_-]{1,40}`. Equals the folder name. The key for overrides and the default server name |
| `name` | Display name: the View > Syntax menu, `overrides[].language`, the status bar. It need not be unique (JSX, below) |
| `hidden` | Default `false`. Hidden languages are detected but left out of the syntax menu and name lookups, like `hidden: true` in a `.sublime-syntax` |
| `scope` | Base scope (`source.pony`), used by selectors, keymap `syntax == source-pony`, `word_wrap: "auto"` |
| `files.extensions` / `files.names` | Case-insensitive extension and exact file-name claims, matching syntect's `eq_ignore_ascii_case` |
| `files.first_line` | Regex tried on line 1 when no extension or name matches (shebangs, modelines) |
| `files.content` | Regex tried on the first 4 KiB to settle a claim that several languages make (section 7) |
| `comments` | Line marker and block pair, used by Toggle Comment instead of the name tables |
| `indent` | Language defaults for `tab_size` and `translate_tabs_to_spaces` (precedence: D4) |
| `syntax` | A `.sublime-syntax` in the folder, or `builtin:<syntect name>` (for example `builtin:JavaScript`) |
| `tree_sitter.grammar` | `grammar.wasm`, or `builtin:<name>` for a grammar compiled into Orlo (section 8.3) |
| `tree_sitter.symbol` | Exported language name passed to `WasmStore::load_language`. Defaults to `id` with `-` changed to `_` |
| `tree_sitter.highlights` | A list of queries, concatenated in order; later patterns win ties, as they do today. Entries can be `builtin:rust/highlights` |
| `tree_sitter.source` | Where the bundled `.wasm` came from, used by `scripts/build-grammar.sh` |
| `lsp.language_id` | `languageId` sent in `didOpen` |
| `lsp.server` | Server name. Languages that name the same server share a session per root (today's `typescript` family). Defaults to `id`. It is also the key in the `language_servers` setting |
| `lsp.command`, `lsp.initialization_options` | Default server definition. Two manifests defining the same server name is reported, and the higher-precedence source wins |
| `lsp.root_markers` | File names that mark a project root; one leading `*` is allowed (`*.nimble`). `.git` stays the fallback, as today |
| `formatter.command` | Argument array, document on stdin, formatted text on stdout. The output is applied only on exit status 0 (D5) |

Levels follow from which fields are present:

- **Basic**: `syntax` only.
- **Good**: `tree_sitter`, with or without `syntax`.
- **Full**: either of those plus `lsp.command` (or a user `language_servers` entry for that server).

### 4.3 The seven built-ins as manifests (stage 2)

Built-in folders point at what is compiled in, so nothing third-party gets copied:

```json
{ "schema": 1, "id": "rust", "name": "Rust", "scope": "source.rust",
  "files": { "extensions": ["rs"] },
  "comments": { "line": "//", "block": ["/*", "*/"] },
  "syntax": "builtin:Rust",
  "tree_sitter": { "grammar": "builtin:rust",
                   "highlights": ["queries/prelude.scm", "builtin:rust/highlights"] },
  "lsp": { "language_id": "rust", "server": "rust", "root_markers": ["Cargo.toml"],
           "command": ["rust-analyzer"], "initialization_options": {} } }
```

`queries/prelude.scm` holds Orlo's own `(identifier) @variable`.

JSX gets its own folder: id `jsx`, name `JavaScript`, `hidden: true`, `builtin:javascript`
grammar, `language_id` `javascriptreact`, server `typescript`. That reproduces today exactly:

- `.jsx` buffers report the syntax name "JavaScript", so `overrides[{"language":"JavaScript"}]`
  still applies to them.
- LSP still receives `javascriptreact`.

TypeScript and TSX use `syntax: builtin:JavaScript`, which is what the syntect fallback uses
today.

None of the seven declares `indent`, `first_line`, `content`, `folds`, `indents` or `formatter`.
Today none of them has those behaviours, so adding any would change them.

## 5. Where folders live: `Languages/` (recommended) or a package field

Recommendation: a separate `~/Library/Application Support/Orlo/Languages/<id>/` folder, with the
same layout as the bundled `assets/languages/<id>/`.

Reasons:

1. **Overrides are structural.** The folder name is the id, so two user languages cannot claim
   the same id, and overriding means "a folder with the same name". With packages, two packages
   can both contribute `nim`, and Orlo needs a tie-break rule that users would have to learn.
2. **Same shape for every role.** A user copies `assets/languages/rust` to `Languages/rust` to
   override it. A contributor develops in `Languages/pony` and opens a PR adding the same folder
   under `assets/languages/pony`. Nothing has to be wrapped or unwrapped.
3. **Isolation.** The package loader drops every package when one manifest is broken
   (section 3.1). A loose folder fails alone.
4. **It doesn't touch package semantics.** `package.json` uses `deny_unknown_fields`, and
   packages are about shell commands and tasks. Languages are data that the highlighter and LSP
   read.
5. **Disabling works the same way as for packages.** A `.disabled` file in the folder skips it.

What this gives up: Install/Manage Packages, and shipping a language together with tasks in one
unit. Both can come later without changing the folder format: a package could list
`"languages": ["languages/pony"]`, and the loader would treat each entry as one more root.

The choice is D2.

## 6. Loading, overrides and errors

- **Sources, lowest precedence first:** bundled (embedded), then `Languages/`. A user folder
  replaces a bundled folder with the same id entirely. Files are never merged (D3), so what you
  see in the folder is what runs. A user copy can still say `"grammar": "builtin:rust"`.
- **Bundled folders are embedded.** A `build.rs` in `orlo-syntax` generates an `include_bytes!`
  table for `assets/languages/**`, the same way `crates/ui/src/icons/mod.rs` embeds icons. So
  `cargo run`, tests and the CLI need no `.app` Resources lookup. Cost: the size of the bundled
  `.wasm` files (measured in stage 5).
- **Two phases.**
  - Load (startup and Reload Customizations) parses manifests, checks paths and limits, and
    compiles regex patterns. It is cheap and synchronous.
  - Activate (first buffer of that language, on the highlight worker) compiles the wasm and
    queries.

  User folders are also activated eagerly in the background after a reload, so their errors
  show up in the same alert. Bundled ones are already proven by CI. Activation is lazy, so 100+
  languages cost nothing until one is opened.
- **Errors.** Every failure is a `String` in the list that `highlight::reload_customizations`
  returns, formatted `path: reason`, like invalid themes and syntaxes today. Activation errors
  that turn up later go through the same alert path. A failed language is reported once and
  dropped for the rest of that generation; detection then falls back as if the folder weren't
  there. Grammar and query code paths return `Result`. The two `.expect()` calls in
  `treesitter.rs` remain only for `builtin:` grammars, which a test proves.
- **Generations.** The registry is an `Arc` snapshot swapped on reload, like `Assets`
  (`highlight.rs:31-48`). Running jobs finish on the snapshot they started with.
- **Crate placement**, keeping `docs/architecture/overview.md`'s direction:
  - `orlo-config` (`themes/assets.rs`) only lists `Languages/` folders, beside `Syntaxes/` and
    `Themes/`. Config owns asset discovery.
  - `orlo-syntax` owns the manifest types, registry, detection, grammars and queries
    (`languages/registry.rs`, `languages/manifest.rs`).
  - `orlo-ui` consumes the registry.

  No new crate edges.

## 7. Detection

Order, first hit wins:

1. An explicit View > Syntax choice for the buffer.
2. Exact file name (`files.names`).
3. Extension:
   - one claimant wins;
   - with several claimants, those whose `content` matches beat those without a `content`
     pattern, and those whose pattern fails are out;
   - among equal claimants, the user folder beats the bundled one, then `id` order.
4. `first_line`.
5. Today's syntect detection, for everything that isn't a registered language.

Compatibility rule: today a loose `Syntaxes/` grammar that claims `.rs` replaces bundled Rust
highlighting. That stays true for bundled languages, while LSP, comments and indent still come
from the manifest. A user `Languages/` folder beats a loose syntax, because it is the newer and
more specific mechanism.

Rux and the `.rux` UI language:

- Rux declares `"content": "\\A\\s*(?:[^<\\s]|\\z)"`, meaning the first non-blank character is
  not `<`, or the file is empty.
- Every one of 1,405 Rux sources in rux-lang/Rux and rux-lang/Examples starts with something
  other than `<`. 62 of the UI language's 68 example files start with `<template>` and 6 with
  `<!--`.
- With only Rux installed, a UI-language file therefore falls to Plain Text instead of being
  coloured as Rux. If someone adds a `ruxlang` folder with `"content": "\\A\\s*<"`, both work
  side by side.

Detection re-runs on Save As and on Reload Customizations, as it does now. The detected language
id is stored with the editor and sent to LSP, instead of LSP detecting again from the path
(section 3.2).

## 8. Replacing the `Language` enum without breaking callers

### 8.1 End state

- `orlo_syntax::language::Language` becomes a cheap-clone handle (`Arc<LanguageDef>`) with
  `id()`, `name()`, `scope()`, `server()`, `markers()` and `lsp_language_id()`.
- The enum's one irreducible job, naming parsers compiled into the binary, moves to a private
  `enum Builtin { Rust, Go, JavaScript, TypeScript, Tsx, Python }` in `grammars/treesitter.rs`.
  It backs the `builtin:` namespace (grammars plus `HIGHLIGHTS_QUERY` constants).

### 8.2 Bridge, by stage

- **Stage 1.**
  - The registry exists and is tested, and its errors are reported.
  - The enum and every caller are unchanged, so the editor behaves exactly as before.
- **Stage 2.**
  - The seven manifests are added.
  - Enum methods read the registry: `Language::detect` asks the registry and maps the seven ids
    back to variants, returning `None` for any other language for now.
  - `treesitter::{name, scope, named}` read manifests.
  - A golden test keeps today's tables as the oracle. For every variant it asserts that the
    extensions, id, server, markers, display name, scope, highlight query text and
    `default_servers()` are identical.
- **Stages 3–4.**
  - Callers move to the handle: `treesitter::State.language`, `Highlighter`, `ServerKey`, the
    LSP `Document`.
  - When none remain, the enum is deleted. Tests that say `Language::Rust` become
    `Language::bundled("rust")`.

`markers()` and `server()` return `&'static` data today; they become borrowed from the handle.
`ServerKey` becomes `(PathBuf, Arc<str>)`.

### 8.3 Highlighter changes (stage 3)

`Highlighter::detect` asks the registry first. For a registered language it uses the language's
syntect syntax for the 16 MiB fallback, symbol parsing and comments, and attaches Tree-sitter
when the manifest has a grammar. When no language matches, it does what it does today.

Related changes:

- `by_name` and `syntax_names` use registry names, so the hand-added TypeScript entries are
  gone.
- `comment_prefix` and `block_comment` ask the language first, then fall back to the tables for
  plain syntect syntaxes.

`capture_scope` gains a longest-dotted-prefix fallback (`keyword.function` → `keyword` →
`keyword.control`), because third-party queries use nvim-treesitter's longer names. It applies to
every grammar only if a test shows that each capture name in the six built-in queries maps to the
same scope as before. Otherwise it is limited to non-builtin grammars.

## 9. Runtime Tree-sitter grammars

### 9.1 Verified API (tree-sitter 0.27.1, read from the crate source)

- **Feature:** `wasm`. It enables `wasmtime-c-api-impl ^48.0.1` (resolves to 48.0.7) with
  `default-features = false` and features `cranelift`, `gc` and `gc-null`. The crate re-exports
  `wasmtime`.
- **Loading:**
  - `WasmStore::new(&wasmtime::Engine) -> Result<WasmStore, WasmError>`
  - `WasmStore::load_language(&mut self, name: &str, bytes: &[u8]) -> Result<Language, WasmError>`
  - `Parser::set_wasm_store(store)` and `take_wasm_store()`
  - `Language::is_wasm()`
  - `WasmError { kind: Parse | Compile | Instantiate | Other, message }`
- **Compatibility:**
  - `LANGUAGE_VERSION` is 15 and `MIN_COMPATIBLE_LANGUAGE_VERSION` is 13.
  - Pony's and Nim's ABI-14 parsers load.
  - `set_language` returns `LanguageError::Version`, `NotParseable` or `Wasm` rather than
    panicking.
- **Stores:**
  - Each parser owns one store, because `set_wasm_store` takes it by value.
  - A language loaded through any store works with any parser whose store shares the `Engine`;
    the store instantiates the module lazily (`ts_wasm_store_add_language`, `src/wasm_store.c`).
- **Queries:** only the lexer and external scanner run in wasm. Parse tables are copied to
  native memory, so `Query` and `QueryCursor` work unchanged.
- **Traps:** a trap inside the grammar sets the store's error flag, and the parse returns `None`
  (`parser.c` checks `ts_wasm_store_has_error`). It is not a crash.
- **Native alternative:** `tree_sitter_language::LanguageFn::from_raw` plus `Language::new`
  (tree-sitter-language 0.1.9). `libloading` 0.8.9 is already in `Cargo.lock`.

### 9.2 Measured cost

Setup: Apple M4, rustc 1.99.0, Orlo's release profile (thin LTO, 1 codegen unit). Probe binary =
tree-sitter 0.27.1 + tree-sitter-rust 0.24.2. The wasm grammar is tree-sitter-rust v0.24.0's
release `.wasm` (1.1 MB). Input is a 4.6 MB Rust file built by concatenating Orlo's crates. The
probe source is in the appendix.

| | Native only | + `wasm` (cranelift) | + Pulley interpreter |
| --- | --- | --- | --- |
| Probe binary, stripped | 2.83 MB | 9.64 MB (**+6.8 MB**) | 9.73 MB |
| Probe binary, unstripped | 3.12 MB | 11.54 MB (+8.4 MB) | 11.65 MB |
| New crates, compared with Orlo's `Cargo.lock` | 0 | 35 (cranelift-*, wasmtime-*, wasmparser, regalloc2, …) | same |
| Clean build of the probe | 8 s | 58 s wall, 161 s CPU | same |
| Extra build tool | none | **cmake** | cmake |
| Full parse, 4.6 MB | 288–290 ms | 433–450 ms (**1.5×**) | 2,760–2,770 ms (9.6×) |
| Reparse after a one-line insert | 4.0 ms | 4.3–4.5 ms | 8.1 ms |
| Query capture walk | 197 ms | 197 ms (queries run natively) | 196–218 ms |
| Engine + compile 1.1 MB grammar | n/a | 17–29 ms, once per grammar | 17 ms |
| `WasmStore::new` per parser | n/a | 3.5 ms | 3.7 ms |

For scale, Orlo's current unstripped release binary is 27.0 MB.

tree-sitter PR #5847 published about 65% of native speed for typical code, which matches the
1.5× above.

Hardened runtime test: the cranelift probe was ad-hoc signed with `--options runtime`, as
`bundle.sh` signs Orlo.

- No entitlements: **killed (exit 137) on the first wasm parse.**
- `allow-jit` only: killed.
- `allow-unsigned-executable-memory`: works.

This matches wasmtime issue #11989, which says wasmtime 48 uses RW-then-RX `mprotect`, not
`MAP_JIT`. Zed ships both `allow-jit` and `allow-unsigned-executable-memory`. Pulley would avoid
executable memory (I haven't verified that under the hardened runtime), but it parses 9.6× slower
than native.

### 9.3 WebAssembly or native libraries

| | WebAssembly (recommended) | Native `.dylib` |
| --- | --- | --- |
| A broken grammar | Traps; parse returns `None`; Orlo reports it and falls back | A bad scanner can segfault Orlo. That breaks "never crash" |
| One artifact for every Mac | Yes; can be committed | No: arm64 and x86_64 (or lipo), built per release |
| User-added grammars | `tree-sitter build --wasm` (CLI 0.27 downloads wasi-sdk itself; no emscripten or docker) | Needs Xcode CLT; Orlo would need `disable-library-validation`, which lets any unsigned library load into Orlo |
| Size and dependencies | +6.8 MB stripped, 35 crates, cmake | 0 (libloading already present) |
| Speed | 1.5× slower full parse; incremental reparses about equal | Native |
| Signing | `allow-unsigned-executable-memory` | `disable-library-validation` |

I recommend WebAssembly. "One broken folder must never crash Orlo" is a hard requirement, and
only the sandbox can meet it for grammar code. Both options weaken the hardened runtime by one
entitlement, and the wasm one is narrower: library validation stays on. The size is the real
price. That is D1, and it includes whether wasmtime falls under your "no plugin runtime" rule.

### 9.4 Runtime design

- **Engine.** One `wasmtime::Engine` per process, created lazily, so users who never open a wasm
  language pay no startup cost.
- **Grammars.** Each grammar is compiled once per registry generation, on the highlight worker,
  never on the UI thread (17–29 ms per grammar).
- **Parsers.** `treesitter.rs` creates a `Parser` for every new parse job today. With wasm that
  adds 3.5 ms for a store plus module instantiation, so the worker keeps a small pool of parsers
  that each own a `WasmStore`, the way Zed does (`take_wasm_store` / `set_wasm_store`). The
  stage-3 test asserts keystroke-to-colour stays within the existing UI benchmark budget for a
  wasm language.
- **Limits.**
  - `MAX_PARSE_BYTES` (16 MiB) and the progress-callback cancellation apply unchanged; the
    callback runs between lexer calls.
  - A trap and a cancellation look the same: the parse returns `None` either way.
    `treesitter.rs` treats `None` as "cancelled" and retries on the next job, so a trapping
    grammar would retry forever. The progress callback will record whether Orlo asked to stop.
    A `None` that Orlo didn't ask for marks the language failed and reports it.
  - Remaining risk: an infinite loop inside a grammar's scanner would stall the highlight
    worker. Tree-sitter creates the wasmtime store internally, so Orlo cannot set fuel or epoch
    deadlines. Native grammars have the same risk. I'm listing it and not solving it now.
- **Queries.**
  - `highlights` drive colours.
  - `folds` (capture `@fold`) feed Fold, Fold Level N and the gutter buttons for that language;
    other languages keep indentation folding.
  - `indents` support a small, documented subset of nvim-treesitter's captures (`@indent.begin`,
    `@indent.end`, `@indent.branch`, `@indent.dedent`), used only by Enter. A language without
    `indents.scm` keeps today's bracket rule.
  - `tags` (`@definition.*` with `@name`) feed Goto Symbol and the index when the language has
    no regex grammar.
- **Predicates.** Queries are compiled with the Rust binding, which applies `#eq?`, `#match?`,
  `#any-of?` and their negations. Neovim-only predicates (`#lua-match?`, `#has-ancestor?`)
  compile but would be silently ignored, so the validator rejects unknown predicates. Vendored
  queries get rewritten (for example `#lua-match?` → `#match?`).

## 10. Generic LSP, `language_servers` and formatters

- `default_servers()` is built from the manifests: every distinct `lsp.server` that has a
  `command`. The four entries come out byte-for-byte the same, including Go's
  `{"semanticTokens": true}`, which moves into `go/language.json`. The real-server smoke test
  keeps calling `default_servers().remove("go")` unchanged.
- `language_servers` stays an application setting keyed by server name, merged over the
  defaults (`crates/ui/src/views/workspace/language.rs:591-592`). An empty `command` still disables a server. New keys
  (`pony`, `nim`, or a user language's id) need no code. User languages can leave `lsp.command`
  out and rely on the setting alone.
- Session key `(root, server name)`. Root markers come from the buffer's language. `languageId`
  comes from the language, which is detected once and stored (section 3.2).
- Gates and messages become data-driven:
  - `can_navigate_with_server` becomes "the buffer's language has a configured server";
  - "LSP supports Rust, Go, …" becomes "No language server is configured for Pony".
- **Formatters** (`formatter.command`, D5):
  - Run argv directly, without a shell, matching the `language_servers` convention.
  - Reuse Filter Through Shell's runner: one undo step, stale-buffer check, timeout, process-group
    kill, 4 MiB cap.
  - Apply output only on exit 0. nph exits 3/4/5 on parse failure and can echo its input.
  - An application setting `formatters`, keyed by language id, overrides or disables (`[]`) the
    manifest command.

## 11. New languages (stage 5)

| | Pony | Nim | Rux |
| --- | --- | --- | --- |
| Level | Full | Full + formatter | Basic |
| Grammar | tree-sitter-grammars/tree-sitter-pony @ `73ff874a` (master, 2024-04-20), ABI 14. No prebuilt wasm, so we build it | alaviss/tree-sitter-nim @ `ac72ba30` (main) or tag 0.6.2, ABI 14. No prebuilt wasm | Our own `Rux.sublime-syntax` |
| Grammar licence | MIT | **MPL-2.0** (file-level copyleft) | MIT (rux-lang/Rux); references are rux-lang/VSCode (MIT) and rux-lang/SublimeText (MIT) |
| Queries | highlights, folds, indents, tags adapted: `#lua-match?` → `#match?`, nvim indent captures mapped to our subset | `highlights.scm` (MPL-2.0, no predicates). Folds from our own query; nvim-treesitter's are Apache-2.0 but use `#has-ancestor?` | n/a |
| Extensions | `pony` | `nim`, `nims`, `nimble` | `rux`, with the content rule from section 7 |
| Server | `pony-lsp`, **no arguments** (since ponyc 0.67.0 it exits 1 on an unknown argument such as `--stdio`); bundled with ponyc ≥ 0.60.5 via ponyup | `nimlangserver` (stdio by default) | none |
| Root markers | `corral.json`, `bundle.json` | `*.nimble`, `nim.cfg`, `config.nims` | n/a |
| Formatter | none | `["nph", "-"]` | none |
| `languageId` | `pony` (VS Code extension, nvim-lspconfig) | `nim` | n/a |

Rux keywords come from the Rux lexer (`Compiler/Lexer/Token.cpp`) and docs, which list 38:

> as break catch const continue defer do else enum extend extern fail false for func if import
> in interface intrinsic is let loop match module none null pub return self struct true type
> union var variant when while

Contextual words: `asm sizeof alignof Self Success Failure Some`. Built-in types are predefined
names (`int8`…`int512`, `float80`, `char8`…), coloured as `support.type`. Comments are `//`,
`/* */` (they nest; doc forms `///` and `/** */`). Both existing editor grammars are out of date:
VSCode lacks `catch defer fail none variant when`, and SublimeText lacks more. So the syntax is
written from the lexer and docs, using those grammars for structure, and their MIT notices go in
`rux/LICENSE`.

Licence records follow `assets/icons/LICENSE`: each bundled folder's `LICENSE` opens with a few
sentences on what is derived from where (repository, revision, files), followed by the full
licence text. `nim/LICENSE` carries MPL-2.0 and the grammar's source location, which MPL §3.2
requires for the executable form. The README's "Orlo vendors no third-party source" becomes a
pointer to `assets/languages/*/LICENSE`. That is D6.

Bundled `.wasm` files are built by `scripts/build-grammar.sh <id>`. It clones `source.repository`
at `source.revision` and runs `tree-sitter build --wasm` with tree-sitter CLI 0.27.1. The files
are committed (D6) so that building Orlo needs no network.

## 12. Contributors and CI (stage 6)

- **Template.** `docs/templates/example-language/` (id `example-language`): a manifest
  explained by a README next to it (strict JSON has no comments), a starter `.sublime-syntax`, an
  empty `queries/highlights.scm` and a LICENSE stub. CI validates it, so it can't go stale.
- **Guide.** `docs/user-guide/adding-a-language.md` covers:
  - the three levels and what each one buys;
  - the manifest reference;
  - capture names Orlo colours;
  - building a `.wasm`;
  - testing in `Languages/`;
  - opening a PR to `assets/languages/`;
  - licence rules.
- **Validator.**
  - `Orlo --check-language <folder>...` is a non-GUI launch mode beside `--help` and
    `--version` (`crates/workspace/src/navigation/cli.rs`).
  - It runs exactly the loader's code, with activation forced:
    - parse the manifest;
    - check paths and limits;
    - load the grammar;
    - compile every query against it and reject unknown predicates;
    - build the `.sublime-syntax` on its own in a fresh `SyntaxSet` and check that it has no
      unlinked contexts.
  - Contributors run it on their own folder without building Orlo. Exit code is non-zero on any
    error.
- **CI.** A step after the release build runs
  `target/release/Orlo --check-language assets/languages/* docs/templates/example-language`. A unit
  test in `orlo-syntax` covers the same thing inside `cargo test`.

## 13. Stages, what "done" means, tests

Every stage ends with the following, then a commit, then a stop for you to test:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked --release
```

1. **Registry and manifest.**
   - Work:
     - manifest types and validation;
     - `Languages/` discovery in config;
     - registry with override and detection;
     - activation API without consumers yet;
     - errors in the Reload Customizations alert.
   - Tests:
     - valid manifest;
     - unknown field;
     - bad id;
     - folder/id mismatch;
     - bad regex;
     - `..`, absolute and symlink-escape paths;
     - missing file;
     - over-size file;
     - `.disabled`;
     - user-over-bundled override;
     - extension/name/first-line/content detection, including the Rux-versus-UI-language
       fixtures and an empty file;
     - tie-breaks;
     - one broken folder among good ones (the others load and the error names the path);
     - arbitrary bytes as `language.json` never panic.
   - Users see no change.
2. **The seven built-ins as manifests.**
   - Work:
     - `assets/languages/{rust,go,python,javascript,jsx,typescript,tsx}`;
     - the embedding `build.rs`;
     - enum methods and `treesitter::{name,scope,named}` backed by the registry;
     - tests that replace the process-wide registry restore it afterwards, since enum methods
       now read it.
   - Done when:
     - the golden test passes;
     - every existing test passes unchanged;
     - the real-server smoke test passes against `target/lsp-tools` (I'll run it if the tools
       are present and say so if not).
3. **Runtime Tree-sitter.**
   - Work:
     - `wasm` feature;
     - cmake in CI;
     - the entitlements plist in `bundle.sh`;
     - the parser/store pool;
     - wasm and query activation with errors, reading files through a `LanguageDef` method
       that checks again that each path stays inside the folder and within its size limit (a
       link could change after load);
     - Highlighter driven by the registry;
     - capture prefix fallback;
     - folds/indents/tags as scoped in 9.4.
   - Tests:
     - a small MIT wasm fixture grammar (tree-sitter-json) highlights a file;
     - a truncated `.wasm`, a query naming an unknown node, and a wrong ABI each give one
       reported error and Plain Text, with no crash;
     - Tree-sitter wins over the folder's regex grammar;
     - the UI keystroke benchmark stays in budget with a wasm language;
     - the bundled app is launched and a wasm-language file is opened.
4. **Generic LSP.**
   - Work:
     - `ServerKey (root, Arc<str>)`;
     - the detected language carried on `Document`;
     - manifest-driven `default_servers()`, markers and ids;
     - data-driven gates and messages;
     - formatter runner and `formatters` setting;
     - enum removed.
   - Tests:
     - existing `language_tests.rs` unchanged;
     - a fake-server test for a manifest-only language (via `tests/fixtures/language_features.py`);
     - user `language_servers` entry for a new server name;
     - shared server across two languages;
     - formatter: exit 0 applies, non-zero leaves text;
     - smoke test unchanged.
5. **Pony, Nim, Rux.**
   - Work:
     - folders, built `.wasm` files, adapted queries, LICENSE files;
     - Rux syntax.
   - Tests:
     - each language's sample file highlights;
     - the Rux and UI-language samples detect correctly.
   - Manual check, if the tools are installed: `pony-lsp` and `nimlangserver` start and return
     diagnostics, and `nph -` formats.
6. **Contributors.**
   - Work:
     - template;
     - guide;
     - `--check-language`;
     - CI step;
     - README and architecture updates.

Documentation moves with each stage:

- **README "Language intelligence":** stages 2–5.
- **README "Not in this version":**
  - grammars are not downloaded or built by Orlo;
  - no debugger;
  - no plugin runtime (the wasm sandbox runs grammars only);
  - no indent-on-type beyond Enter.
- **`docs/architecture/overview.md`:** stages 1, 3 and 6 (registry ownership,
  `assets/languages/`, the `wasm` dependency).
- **`docs/user-guide/customization.md`:** stage 3, when a `Languages/` folder first changes
  highlighting. Until then a folder is only loaded and checked, so the guide doesn't advertise it.

## 14. Decisions I need from you

- **D1. Runtime grammar format.**
  - Recommended: WebAssembly.
  - It means accepting:
    - wasmtime 48 inside Orlo, running grammar code only;
    - about +7 MB of binary;
    - 35 crates;
    - cmake for builds;
    - `allow-unsigned-executable-memory` in `bundle.sh`.
  - Alternatives:
    - native `.dylib` grammars: no size cost, but a bad grammar can crash Orlo, and it needs
      `disable-library-validation`;
    - no runtime grammars: new languages would be basic or full only, never "good" without a
      recompile.
- **D2. Where user languages live.**
  - Recommended: `Languages/<id>/` beside `Syntaxes/`.
  - Alternative: a `"languages"` field in `package.json`.
- **D3. Override granularity.**
  - Recommended: the user folder replaces the bundled folder whole.
  - Alternative: field-by-field merge, like Helix's `languages.toml`.
- **D4. Manifest indent defaults.**
  - Recommended: apply only where you haven't set `tab_size` or `translate_tabs_to_spaces`
    anywhere (global, overrides, project, EditorConfig), using the existing
    `indentation_explicit` flags.
  - Alternative: Sublime's order, where syntax-specific settings beat your global preferences.
- **D5. Formatters.**
  - Recommended:
    - a configured formatter wins over LSP formatting for that language;
    - output applied only on exit 0;
    - a new application setting `formatters` keyed by language id overrides or disables it.
  - Alternative: LSP first, with the formatter only as a fallback.
- **D6. Licences and binaries.**
  - Accept MPL-2.0 files (Nim's grammar `.wasm` and `highlights.scm`) under
    `assets/languages/nim/`. That is file-level copyleft; Orlo stays MIT.
  - Commit the built `.wasm` files with pinned revisions and a rebuild script.
  - Change the README's "vendors no third-party source" sentence.
  - Alternative for Nim queries: write our own highlights and keep only the grammar binary under
    MPL.
- **D7. Stage-3 scope.**
  - Recommended: highlights, folds, Enter-only indents and `tags.scm` Goto Symbol. Without tags,
    Pony and Nim have an empty Goto Symbol and outline.
  - Alternative: highlights only now, with the other queries validated but unused until later.
- **D8. Built-in queries.**
  - Recommended: reference the crates' query constants via `builtin:`, so nothing is vendored and
    nothing can drift.
  - Alternative: copy them into `.scm` files so they can be edited in place. That means vendoring
    MIT files and keeping them in step with crate bumps.

## Appendix: measurement probe

Scratch crate, built outside the repository with `[profile.release] lto = "thin"`,
`codegen-units = 1`.

- Dependencies: `tree-sitter = "=0.27.1"`, `tree-sitter-rust = "=0.24.2"`,
  `streaming-iterator`.
- Optional dependency: `wasmtime = { version = "48", default-features = false, features = ["pulley", "runtime"] }`.
- Features: `wasm = ["tree-sitter/wasm"]` and `pulley = ["wasm", "dep:wasmtime"]`.
- What it does:
  - parses the input with the native and the wasm language;
  - times a full parse, an incremental reparse after inserting `//\n` at the middle, query
    compile and a full `captures()` walk;
  - for wasm, builds `Engine::new(&Config)` with the default (cranelift) target, or
    `config.target("pulley64")`.
- Results were taken on warm runs, three native and two of each wasm variant.
