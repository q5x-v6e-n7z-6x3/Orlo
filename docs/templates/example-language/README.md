# Example language

A starting point for a new language folder. Copy this folder to
`~/Library/Application Support/Orlo/Languages/<your-id>/`, rename it to your language's id, and
edit the files. [Adding a language](../../user-guide/adding-a-language.md) is the full guide;
`Orlo --check-language <folder>` checks a folder without starting Orlo.

As it is, the folder is a working basic-level language: files ending in `.example` are coloured
by `Example.sublime-syntax`.

## `language.json`

The manifest is strict JSON, so it can't carry comments; they are here instead.

```json
{
  "schema": 1,
  "id": "example-language",
  "name": "Example Language",
  "scope": "source.example-language",
  "files": { "extensions": ["example"] },
  "comments": { "line": "#" },
  "indent": { "tab_size": 4, "translate_tabs_to_spaces": true },
  "syntax": "Example.sublime-syntax"
}
```

- `schema`: the manifest format. Always `1`.
- `id`: the language's key, 1–40 lowercase letters, digits, `-` or `_`. It must be the folder's
  name. Preferences use it (`formatters`, View > Syntax choices), and so does the language server
  unless `lsp.server` says otherwise.
- `name`: shown in View > Syntax and the status bar, and matched by `overrides[].language` in
  Preferences.
- `scope`: the base scope colour schemes, selectors and keymap contexts see. Use
  `source.<id>` for code and `text.<id>` for prose or markup. Use the same scope in the
  `.sublime-syntax`.
- `files`: which files are this language.
  - `extensions`: endings without the dot, compared ignoring case.
  - `names`: whole file names, such as `["Examplefile"]`.
  - `globs`: EditorConfig-style path patterns, such as `["**/examples/*.txt"]`.
  - `first_line`: a regular expression for the first line, such as `"^#!.*\\bexample\\b"` for a
    shebang.
  - `content`: a regular expression over the first 4 KiB, to tell this language from another
    that claims the same extension.
- `comments`: `line` is the line comment marker; `block` is a pair, such as `["/*", "*/"]`.
  Toggle Comment uses them.
- `indent`: the default tab width (1–16) and tabs or spaces, used only when nothing else
  (Preferences, EditorConfig, the file's own indentation) decides.
- `syntax`: the `.sublime-syntax` file that colours the language, or `builtin:<name>` to reuse one
  Orlo ships (`builtin:JavaScript`). Leave it out for a language coloured by Tree-sitter alone.

## Adding Tree-sitter

Build the grammar with `tree-sitter build --wasm -o grammar.wasm .` (tree-sitter CLI 0.27.1) in
its repository, put `grammar.wasm` here, write `queries/highlights.scm`, and add:

```json
"tree_sitter": {
  "grammar": "grammar.wasm",
  "symbol": "example_language",
  "highlights": ["queries/highlights.scm"],
  "folds": "queries/folds.scm"
}
```

- `grammar`: the `.wasm` file, or `builtin:<name>` for a parser compiled into Orlo.
- `symbol`: the name the grammar exports (`tree_sitter_<symbol>`): the `name` in its
  `grammar.js`. It defaults to the id with `-` changed to `_`, as here.
- `highlights`: one or more query files, joined in order. `queries/highlights.scm` in this folder
  shows the shape; its patterns are commented out until there is a grammar to check them against.
- `folds`: optional; each multiline node captured as `@fold` can be folded.
- `indents`: optional; `@indent.begin`, `@indent.end`, `@indent.branch` and `@indent.dedent`
  set how far Enter indents the new line.
- `tags`: optional; `@definition.<kind>` around a `@name` lists Goto Symbol entries for a
  language without a `.sublime-syntax`.
- `source`: for a bundled language, `{"repository": "https://…", "revision": "<full commit SHA>"}`
  so `scripts/build-grammar.sh <id>` can rebuild the `.wasm`.

## Adding a language server

```json
"lsp": {
  "language_id": "example",
  "command": ["example-language-server", "--stdio"],
  "root_markers": ["example.toml"]
},
"formatter": { "command": ["example-format", "-"] }
```

- `language_id`: the `languageId` the server is told.
- `server`: optional; the server's name, shared by languages that use one server, and the key in
  Preferences' `language_servers`. Defaults to the id.
- `command`: the program and its arguments, run directly without a shell.
- `initialization_options`: optional JSON object for the server's `initialize`.
- `root_markers`: file names that mark a project's root; `*.ext` matches by ending.
- `formatter.command`: reads the document on stdin and writes the formatted text on stdout. Its
  output is used only when it exits with status 0.

## `LICENSE`

Replace the stub with where each file came from and the full text of each licence involved.
A folder that is entirely your own work needs only a line saying so.
