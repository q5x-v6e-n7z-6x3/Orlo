# Adding a language

A language is a folder with a `language.json` manifest. You can add one to your own Orlo by
putting the folder in `~/Library/Application Support/Orlo/Languages/<id>/`, and to everyone's by
opening a pull request that adds it under `assets/languages/<id>/`. Both places use the same
format, and nothing has to be compiled: Orlo reads the folder when it starts and on **Reload
Customizations**.

Start from the template in [`docs/templates/example-language/`](../templates/example-language/).
Its README explains each field of its manifest.

## Levels

What a language gets depends on which manifest fields it has. Every level can also have
`comments`, `indent` and `formatter`.

| Level | Fields | What it buys |
| --- | --- | --- |
| Basic | `syntax`: a `.sublime-syntax` file | Regex highlighting, Goto Symbol from the syntax's symbol scopes, indentation folding |
| Good | `tree_sitter`: a WebAssembly grammar and `queries/highlights.scm` | Tree-sitter highlighting that follows edits incrementally, folds from `queries/folds.scm`, Expand and Shrink Syntax Selection |
| Full | Basic or good, plus `lsp.command` (or a `language_servers` entry in Preferences for the language's server) | Diagnostics, completion, definitions, references, hover, rename, formatting and code actions from a language server |

A folder with both `syntax` and `tree_sitter` is coloured by Tree-sitter. Its `.sublime-syntax`
still supplies Goto Symbol and is used for files over 16 MiB, which Tree-sitter does not parse.
A manifest with neither, only `files` and `lsp`, gives Plain Text highlighting and full language
server support.

`queries/indents.scm` sets how far Enter indents the new line (Enter only; typing does not
re-indent). `queries/tags.scm` gives Goto Symbol, the outline and Goto Symbol in Project their
entries when the language has no `.sublime-syntax`; with one, its symbol scopes are used instead.

## Folder layout

```text
<id>/
  language.json            required
  <Name>.sublime-syntax    basic level
  grammar.wasm             good level
  queries/highlights.scm   required with a grammar
  queries/folds.scm        optional
  queries/indents.scm      optional: Enter's indentation
  queries/tags.scm         optional: Goto Symbol without a .sublime-syntax
  LICENSE                  required for a bundled folder that holds third-party files
```

The folder's name must equal the manifest's `id`. Other files (samples, a grammar's source) may
sit beside these. A folder may hold at most 256 files and folders, counting those in its
subfolders. In `Languages/`, a folder holding a file named `.disabled` is skipped, and so is a
folder whose name starts with a dot.

## The manifest

`language.json` is strict JSON: no comments, no trailing commas, and an unknown field is an
error rather than ignored, so a typo is reported. It may be at most 256 KiB. Every error names
the file it is about, as `path: reason`.

| Field | Rules |
| --- | --- |
| `schema` | Required. Must be `1`. A larger number is reported as needing a newer Orlo |
| `id` | Required. 1–40 lowercase ASCII letters, digits, `-` or `_`, starting with a letter or digit. Equals the folder's name. It is the key for `formatters` and View > Syntax choices and, by default, names the language server |
| `name` | Required. 1–80 characters, no control characters, no leading or trailing spaces. Shown in View > Syntax and the status bar, and matched by `overrides[].language` |
| `hidden` | Default `false`. A hidden language is still detected but is left out of View > Syntax and name lookups (the bundled `jsx` is hidden and named "JavaScript") |
| `scope` | Required. The base scope, such as `source.pony`: at most 8 dot-separated words of ASCII letters, digits, `-`, `_` or `+`. Selectors, scoped preferences and keymap contexts (`syntax == source-pony`) use it |
| `files.extensions` | File endings without the dot. Inner dots are allowed (`d.ts`, `tfvars.json`); the longest ending is tried first. Each is 1–255 bytes without `/`, `\`, whitespace or control characters. Compared ignoring ASCII case, but an exact-case claim wins (`C` is C++, `c` is C) |
| `files.names` | Whole file names (`Makefile`), compared like extensions |
| `files.globs` | EditorConfig globs, case-sensitive and non-empty. One without `/` matches a file name anywhere; one with `/` matches the full path. See [Additional language servers](languages.md#additional-language-servers) for the syntax |
| `files.first_line` | A regular expression tried on the first line when no name, extension or glob claims the file (shebangs, modelines) |
| `files.content` | A regular expression tried on the first 4 KiB to settle a claim several languages make. A claimant whose pattern matches beats one without a pattern; one whose pattern fails is out |
| `comments.line` | The line comment marker. Toggle Comment and Wrap Lines use it |
| `comments.block` | `[open, close]`. Markers must be non-empty and on one line. A manifest that names either marker is used as written |
| `indent.tab_size` | 1–16. The language's default, used only where you have set `tab_size` nowhere (Preferences, overrides, project, EditorConfig, a modeline) and the file's own indentation decides nothing |
| `indent.translate_tabs_to_spaces` | `true` or `false`, with the same precedence |
| `syntax` | A `.sublime-syntax` file in the folder (at most 4 MiB), or `builtin:<name>` for one of the syntaxes View > Syntax lists (`builtin:JavaScript`). The file's own `file_extensions` and `first_line_match` are ignored: the manifest decides which files are this language |
| `tree_sitter.grammar` | `grammar.wasm` (any `.wasm` path in the folder, at most 32 MiB) or `builtin:<name>` for a parser compiled into Orlo: `rust`, `go`, `python`, `javascript`, `typescript`, `tsx`, `c`, `cpp`, `csharp`, `java`, `php`, `ruby`, `swift`, `kotlin`, `bash`, `hcl` |
| `tree_sitter.symbol` | The name the grammar exports (`tree_sitter_<symbol>`), a C identifier of at most 64 characters. Defaults to `id` with `-` changed to `_` |
| `tree_sitter.highlights` | Required with `tree_sitter`. A non-empty list of `.scm` files (each at most 1 MiB) or built-in queries, joined in order; later patterns win ties. Built-ins: `rust/highlights`, `go/highlights`, `python/highlights`, `javascript/highlights`, `javascript/jsx`, `typescript/highlights`, `c/highlights`, `cpp/highlights`, `csharp/highlights`, `java/highlights`, `php/highlights`, `ruby/highlights`, `swift/highlights`, `kotlin/highlights`, `bash/highlights`, `hcl/highlights`, each written `builtin:<name>` |
| `tree_sitter.folds` | A `.scm` file. Each multiline node captured as `@fold` folds below its first line. Without it a wasm grammar keeps indentation folding |
| `tree_sitter.indents` | A `.scm` file using nvim-treesitter's `@indent.begin`, `@indent.end`, `@indent.branch` and `@indent.dedent` (with the `indent.immediate` and `indent.start_at_same_line` properties); other captures are ignored. Enter uses it when the tree is current for the text, and Orlo's bracket rule otherwise, or when the line sits in a top-level parse error |
| `tree_sitter.tags` | A `.scm` file capturing `@definition.<kind>` with a `@name` inside it. `<kind>` (`function`, `class`, `method`, ...) picks the symbol's badge. Used only when the language has no `.sublime-syntax` |
| `tree_sitter.source.repository` | Where a bundled `grammar.wasm` is built from: an `https://` URL without spaces |
| `tree_sitter.source.revision` | The tag or commit, 1–100 characters without spaces. `scripts/build-grammar.sh` needs the full 40-character commit SHA |
| `lsp.language_id` | Required with `lsp`. The `languageId` sent to the server, 1–64 bytes without whitespace |
| `lsp.server` | The server's name, in `id` format; defaults to `id`. Languages naming the same server share a session per project root, and it is the key in Preferences' `language_servers` |
| `lsp.command` | The default server command as an argument array, run without a shell. The program must not be blank; no argument may contain NUL |
| `lsp.initialization_options` | A JSON object sent in `initialize` |
| `lsp.root_markers` | File names that mark a project root, each at most one leading `*` (`*.nimble`). `.git` is the fallback |
| `formatter.command` | An argument array that reads the document on stdin and writes the formatted document to stdout. Applied only when it exits with status 0. See [Formatters](languages.md#formatters) |

Every path in the manifest is relative to the folder: no leading `/`, no `.` or `..`. It must
resolve to a regular file inside the folder; a link is followed only if it stays inside. Files
are checked when the folder loads, and again each time Orlo reads one.

Detection, first hit wins: an explicit View > Syntax choice, then `files.names`, then
`files.extensions`, then `files.globs`, then `files.first_line`. Among equal claimants a user
folder beats a bundled one, then the lower `id` wins. A user folder with a bundled language's
`id` replaces that language whole: copy the bundled folder and edit it, rather than writing only
the fields you want to change. A user folder may still point at built-ins (`"grammar":
"builtin:rust"`).

Detection patterns use a linear-time regex engine, so they cannot hang: lookaround and
backreferences are refused. Each is at most 1,024 characters.

## Capture names Orlo colours

Highlight captures become the Sublime scopes colour schemes style. These names are mapped:

| Capture | Scope |
| --- | --- |
| `@comment` | `comment` |
| `@comment.documentation` | `comment.documentation` |
| `@string` | `string` |
| `@string.special` | `string.special` |
| `@escape`, `@string.escape` | `constant.character.escape` |
| `@number` | `constant.numeric` |
| `@constant` | `constant.other` |
| `@constant.builtin` | `constant.language` |
| `@keyword` | `keyword.control` |
| `@operator`, `@keyword.operator` | `keyword.operator` |
| `@function`, `@function.method` | `entity.name.function` |
| `@function.builtin`, `@function.method.builtin` | `support.function` |
| `@function.macro`, `@constant.macro` | `support.macro` |
| `@type`, `@constructor` | `entity.name.type` |
| `@type.builtin` | `support.type` |
| `@property`, `@property.definition`, `@variable.member` | `variable.member` |
| `@module`, `@module.builtin` | `entity.name.namespace` |
| `@variable` | `variable.other` |
| `@variable.builtin` | `variable.language` |
| `@variable.parameter` | `variable.parameter` |
| `@attribute` | `entity.other.attribute-name` |
| `@tag` | `entity.name.tag` |
| `@label` | `entity.name.label` |
| `@punctuation.bracket` | `punctuation.section` |
| `@punctuation.delimiter`, `@delimiter` | `punctuation.separator` |
| `@punctuation.special` | `punctuation.definition` |

A longer name takes the scope of its longest listed prefix, so nvim-treesitter's names work:
`@keyword.function` is coloured as `@keyword`, `@punctuation.bracket.angle` as
`@punctuation.bracket`. Failing that, the first word decides: `keyword`, `conditional`, `repeat`,
`exception` and `include` are `keyword.control`; `string` and `character` are `string`; `number`
and `float` are `constant.numeric`; `boolean` is `constant.language`; `function` and `method` are
`entity.name.function`; `type` is `entity.name.type`; `variable` is `variable.other`. Other
captures draw in the default text colour. (A language using a `builtin:` grammar gets the
first-word rule but not the prefix rule, so Orlo's own languages keep their colours.)

Predicates: `#eq?`, `#match?`, `#any-of?` and their `not-` and `any-` forms work. `#set!` is
accepted and sets properties (`indents.scm` reads two). Any other predicate in your folder's
queries (`#is?`, `#is-not?`, `#lua-match?`, `#has-ancestor?`, `#contains?`) is an error, because
Orlo would otherwise ignore it silently; rewrite `#lua-match?` as `#match?`.

## Building a grammar

Orlo runs grammars compiled to WebAssembly in a wasmtime sandbox: a grammar that crashes stops
parsing and is reported, and the editor keeps going. It never downloads or builds one itself.

With the [tree-sitter CLI](https://github.com/tree-sitter/tree-sitter/tree/master/crates/cli)
0.27.1 (`cargo install tree-sitter-cli --version 0.27.1 --locked`, or `npm install -g
tree-sitter-cli@0.27.1`), in the grammar's repository:

```sh
tree-sitter build --wasm -o grammar.wasm .
```

The CLI downloads wasi-sdk the first time; it needs neither emscripten nor Docker. Copy
`grammar.wasm` into the language's folder and set `tree_sitter.symbol` if the grammar's name
(the `name` in its `grammar.js`) is not the folder's `id` with `-` changed to `_`. Orlo accepts
grammars generated for ABI 13 to 15.

For a bundled language, put the repository and full commit SHA in `tree_sitter.source` and run:

```sh
scripts/build-grammar.sh <id>
```

It clones that revision, builds it with tree-sitter CLI 0.27.1 (and refuses another version),
and writes `assets/languages/<id>/grammar.wasm`. The built file is committed, so building Orlo
needs no network.

Queries usually come from the grammar's `queries/` folder. Check their node names against the
grammar's `src/node-types.json`, and their capture names against the table above.

## Testing in `Languages/`

1. Put the folder in `~/Library/Application Support/Orlo/Languages/<id>/`. A link to a checkout
   works.
2. Run **Reload Customizations** from the command palette. Saving a `language.json` (or a
   `.sublime-syntax`) inside the support folder from Orlo reloads too.
3. An error appears in the reload alert, naming the file and the reason. The folder is skipped
   and other languages load as if it were not there. Grammars and queries are compiled in the
   background right after the reload, so their errors join the same alert a moment later.
4. Open a sample file. The status bar shows the language's `name`. A grammar that fails while
   parsing (a trap in its scanner) loses its Tree-sitter colours until the next reload, whose
   alert names it.

## Checking a folder

```sh
Orlo --check-language <folder> ...
```

It loads and activates each folder exactly as Orlo does, without opening a window: parse the
manifest, check paths and limits, load the grammar, compile every query against it and refuse
unknown predicates, and build the `.sublime-syntax` with Orlo's bundled syntaxes. It prints
`<id>: ok` on stdout for each good folder and each error on stderr, as `path: reason`, and exits
with status 1 if anything failed. It needs no build of your own:
`/Applications/Orlo.app/Contents/MacOS/Orlo --check-language <folder>` works, and from a checkout
`cargo run -p orlo-app -- --check-language <folder>` does the same. CI runs it on every bundled
folder and on the template.

## Contributing a bundled language

1. Develop the folder in `Languages/` until it works and `Orlo --check-language` passes.
2. Copy it to `assets/languages/<id>/`. Add `tree_sitter.source` and rebuild the grammar with
   `scripts/build-grammar.sh <id>`, so anyone can reproduce the committed `.wasm`.
3. Add a small sample under `samples/` and a test like the ones in
   `crates/syntax/src/languages/bundled_tests.rs` (detection, and that the sample highlights).
4. Run the [development checks](../development/verification.md), then open a pull request.

Bundled folders are compiled into the binary (`crates/syntax/build.rs`), so a language costs
its files' size; keep samples small.

## Licences

A bundled folder that holds anything from someone else (a grammar, its queries, a syntax file
adapted from an editor) needs a `LICENSE`. It opens with a few sentences saying what came from
where: the repository, the revision and the files, and what was changed. The full text of each
licence follows. See `assets/languages/pony/LICENSE` and `assets/languages/rux/LICENSE`.

- Permissive licences (MIT, Apache-2.0, BSD) are fine.
- File-level copyleft (MPL-2.0) is fine for the grammar and its queries: say where their source
  is, as MPL-2.0 section 3.2 requires for the built `.wasm`. `assets/languages/nim/LICENSE` is
  the example.
- GPL and other licences that would cover Orlo as a whole are not accepted.
- Files you wrote yourself are under Orlo's MIT licence; say which they are.
