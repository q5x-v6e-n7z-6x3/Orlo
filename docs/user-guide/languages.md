# Programming languages

Orlo bundles Tree-sitter highlighting for seventeen languages. Fifteen follow
[GitHub Octoverse 2025's ten most-used languages](https://github.blog/news-insights/octoverse/octoverse-a-new-developer-joins-github-every-second-as-ai-leads-typescript-to-1/),
plus Kotlin, Ruby, Swift, Rust and C. Octoverse's Shell entry is represented by Bash. Pony and
Nim use WebAssembly grammars from their language folders. HolyC, FreeBASIC and Rux highlighting
are bundled through Sublime grammars.

Each language is a folder with a `language.json` manifest, bundled in `assets/languages/` or
yours in `~/Library/Application Support/Orlo/Languages/`. [Adding a language](adding-a-language.md)
explains the format and how to write one.

Highlighting works immediately. LSP completion, diagnostics, documentation, navigation, rename,
formatting and code actions use separately installed language servers. Availability depends on
the server's advertised capabilities and its project/toolchain configuration.

## Files and server defaults

The family is the key in Preferences' `language_servers` object. Commands below are shown as
argument arrays; Orlo starts them directly, without a shell.

| Language | Detected files | Family | Default command |
| --- | --- | --- | --- |
| TypeScript | `.ts`, `.mts`, `.cts`, `.tsx` | `typescript` | `["typescript-language-server", "--stdio"]` |
| Python | `.py`, `.pyw`, `.pyi` | `python` | `["pylsp"]` |
| JavaScript | `.js`, `.mjs`, `.cjs`, `.jsx` | `typescript` | `["typescript-language-server", "--stdio"]` |
| Java | `.java` | `java` | `["jdtls"]` |
| C# | `.cs`, `.csx` | `csharp` | `["csharp-ls"]` |
| PHP | `.php`, `.phtml`, `.php3`, `.php4`, `.php5`, `.php7`, `.php8` | `php` | `["intelephense", "--stdio"]` |
| Bash | `.sh`, `.bash`, `.bashrc`, `.bash_profile`, `.bash_login`, `.profile` | `bash` | `["bash-language-server", "start"]` |
| C++ | `.C`, `.cc`, `.cp`, `.cpp`, `.cxx`, `.c++`, `.hh`, `.hpp`, `.hxx`, `.h++` | `clangd` | `["clangd"]` |
| HCL / Terraform | `.hcl`, `.tf`, `.tfvars`, Terraform HCL suffixes below | `hcl` / `terraform` | `[]` / `["terraform-ls", "serve"]` |
| Go | `.go` | `go` | `["gopls"]` |
| Kotlin | `.kt`, `.kts` | `kotlin` | `["kotlin-lsp", "--stdio"]` |
| Ruby | `.rb`, `.rake`, `.gemspec`, `Gemfile`, `Rakefile`, `Guardfile` | `ruby` | `["ruby-lsp"]` |
| Swift | `.swift` | `swift` | `["xcrun", "sourcekit-lsp"]` |
| Rust | `.rs` | `rust` | `["rust-analyzer"]` |
| C | `.c`, `.h` | `clangd` | `["clangd"]` |
| HolyC | `.HC`, `.hc`, `.HH` | — | — (highlighting only) |
| FreeBASIC | `.bas`, `.bi` (case-insensitive) | — | — (highlighting only) |
| Pony | `.pony` | `pony` | `["pony-lsp"]` |
| Nim | `.nim`, `.nims`, `.nimble` | `nim` | `["nimlangserver"]`; formatter `["nph", "-"]` |
| Rux | `.rux` not starting with `<` | — | — (highlighting only) |

JSX and TSX use their own parsers and LSP IDs (`javascriptreact` and `typescriptreact`).
C and C++ share clangd; a `.h` document is sent as C, and clangd uses the build configuration
to determine its compilation context. Bash documents use `shellscript`. Zsh is outside this
language set. Extensionless shebang detection remains available for syntax highlighting;
automatic LSP routing uses the file names and extensions in the table.

### HolyC

HolyC source and header files highlight immediately, and View > Syntax > HolyC lets you
select it manually. The grammar covers HolyC's built-in types, keywords, preprocessor
directives, numbers, strings, character constants, operators and comments. Both line and
block comment commands work. Uppercase `.HH` selects HolyC; lowercase `.hh` retains C++.
HolyC files do not start a language server. Compressed TempleOS `.HC.Z` and `.HH.Z` files
must be decompressed first. See the [TempleOS HolyC reference](https://tinkeros.github.io/WbTempleOS/Doc/HolyC.html)
for the language's syntax.

### FreeBASIC

FreeBASIC source (`.bas`) and header (`.bi`) files highlight immediately, including uppercase
extensions. View > Syntax > FreeBASIC selects the grammar manually. Keywords and built-in types
are case-insensitive; numbers, preprocessor directives, function names and strings are colored.
The grammar follows the default `fb` dialect, including doubled quotes, `!"..."` escaped strings
and `$"..."` non-escaped strings. See the [FreeBASIC literals reference](https://www.freebasic.net/wiki/ProPgLiterals).

Line comments use an apostrophe or `Rem`; block comments use `/'` and `'/` and can be nested.
Both comment commands use FreeBASIC's markers, and typing an apostrophe inserts a single
character. See the [FreeBASIC comments reference](https://www.freebasic.net/wiki/ProPgComments).
FreeBASIC files do not start a language server.

### HCL dialects

All HCL files use the bundled HCL parser. Terraform documents use the
[language IDs required by terraform-ls](https://github.com/hashicorp/terraform-ls/blob/main/docs/USAGE.md):

| File suffix | LSP language ID |
| --- | --- |
| `.tf` | `terraform` |
| `.tfvars` | `terraform-vars` |
| `.tfcomponent.hcl` | `terraform-stack` |
| `.tfdeploy.hcl` | `terraform-deploy` |
| `.tfquery.hcl` | `terraform-search` |
| `.policy.hcl` | `terraform-policy` |
| `.policytest.hcl` | `terraform-policytest` |

Generic `.hcl` files use ID `hcl` and a separate family with an empty default command. Configure
a server that supports your HCL dialect to enable LSP for them. Packer and other generic HCL
files are not sent to terraform-ls. JSON Terraform files (`.tf.json`, `.tfvars.json`) retain JSON
highlighting and do not start the Terraform server.

## Install the servers

Use the maintainers' installation instructions for your platform and versions:

- [TypeScript Language Server](https://github.com/typescript-language-server/typescript-language-server):
  `npm install -g typescript-language-server typescript`; both Node.js and TypeScript are needed.
- [Python LSP Server](https://github.com/python-lsp/python-lsp-server):
  `pipx install 'python-lsp-server[all]'`. Formatter and rename support need their respective
  plugins; the default server does not provide semantic tokens.
- [Eclipse JDT Language Server](https://github.com/eclipse-jdtls/eclipse.jdt.ls): install its
  distribution and expose the `jdtls` launcher. It requires Java 21 or newer. For projects with
  the same directory name, use a launcher wrapper that gives each full project path a unique
  `-data` directory; the upstream launcher's default cache is based on the directory name.
- [C# Language Server](https://github.com/razzmatazz/csharp-language-server):
  `dotnet tool install --global csharp-ls`; the current server requires .NET SDK 10 or newer.
  Put the .NET tools directory on PATH and restore the project's dependencies.
- [Intelephense](https://github.com/bmewburn/intelephense-docs/blob/master/installation.md):
  `npm install -g intelephense`. Some advanced features require its paid license; base LSP
  features work without one.
- [Bash Language Server](https://github.com/bash-lsp/bash-language-server):
  `npm install -g bash-language-server`. ShellCheck adds diagnostics and shfmt adds formatting.
- [clangd](https://clangd.llvm.org/installation): install LLVM/clangd or Apple's command line
  tools. Supply `compile_commands.json` or `compile_flags.txt` for the project's include paths,
  compiler options and C/C++ standard.
- [Terraform Language Server](https://github.com/hashicorp/terraform-ls): install the
  `terraform-ls` binary. Install Terraform and initialize the working directory for provider
  schemas and full project completion.
- [gopls](https://go.dev/gopls/): `go install golang.org/x/tools/gopls@latest`, with a compatible
  Go toolchain; put Go's binary directory on PATH.
- [Kotlin Language Server](https://github.com/Kotlin/kotlin-lsp):
  `brew install JetBrains/utils/kotlin-lsp`, or install the standalone release and expose its
  launcher as `kotlin-lsp`. Orlo uses its
  [stdio mode](https://github.com/Kotlin/kotlin-lsp/blob/main/scripts/neovim.md).
  Project support follows the server's Gradle/Maven and platform support.
- [Ruby LSP](https://shopify.github.io/ruby-lsp/editors.html): install `ruby-lsp` with the project's
  Ruby environment (`gem install ruby-lsp`). Launch Orlo with that Ruby/Bundler environment,
  or configure a version-manager launcher. Formatter and linter availability depend on gems
  in the project; Orlo supports published diagnostics, not pull diagnostics.
- [SourceKit-LSP](https://github.com/swiftlang/sourcekit-lsp): install a Swift toolchain. On
  macOS, `xcrun sourcekit-lsp` selects the active developer toolchain. Swift Package Manager
  projects provide build settings; other builds need their supported build configuration.
- [rust-analyzer](https://rust-analyzer.github.io/book/installation.html):
  `rustup component add rust-analyzer`; use a Rust toolchain compatible with the project.

## Configure and use

Open Preferences with Command-comma. Omitted entries use the defaults above; an empty command
disables a family. Use absolute executable paths when Finder's environment cannot find a tool:

```jsonc
{
  "lsp_enabled": true,
  "language_servers": {
    "clangd": {"command": ["/opt/homebrew/opt/llvm/bin/clangd"], "initialization_options": {}},
    "java": {"command": ["/absolute/path/jdtls"], "initialization_options": {}},
    "bash": {"command": ["/absolute/path/bash-language-server", "start"], "initialization_options": {}},
    "hcl": {"command": [], "initialization_options": {}}
  }
}
```

Open the project folder to give the server an explicit root. For loose files, Orlo searches
ancestors for build manifests (Cargo, Go, Python, npm, Gradle/Maven, Composer, Bundler, SwiftPM,
C/C++ build files, .NET solution/project files and Terraform markers) or `.git`, falling back
to the file's parent directory. Sessions are shared by project root and server family.

Use Control-space for completion, F12 for definitions, Control-K Control-I for documentation,
F2 for rename, Option-Shift-F for formatting, and Control-period for code actions. The Goto >
Language Server menu also exposes these actions, diagnostics and server restart. Missing tools
leave normal editing and highlighting available. Saving changed server settings restarts the
affected sessions.

Set `"inline_diagnostics": true` in Preferences to show one diagnostic beside each visible
source line. It is off by default and supports project, language and path overrides. The
inclusive `"inline_diagnostics_severity"` threshold defaults to `"warning"`; use `"error"`,
`"warning"`, `"information"` or `"hint"`. Errors win over warnings; equal severities use
source order. A wrapped line shows its message on the last row. Hidden line ends, active
text composition and lines without trailing room show no inline message. Long messages are
elided. Use **Goto > Language Server > Diagnostics** for full admitted messages from all
open tabs, including lower-priority messages on the same line. Its selected-message area wraps
and scrolls to show long messages in full; arrow keys change the selected message. Existing underlines and gutter
marks remain visible. These overlays never enter copied/saved text or change text coordinates.

Go to Type Definition and Go to Implementation appear in Goto > Language Server and the
command palette when the active server advertises the corresponding feature. Each also has a
To Side command. One result opens its exact selection; multiple results open a keyboard-searchable
location picker. Positions follow the server's negotiated Unicode encoding. Empty, unsupported,
failed or stale requests leave your source caret in place; these commands use semantic results
only. Editing, changing the source selection, leaving the navigation context or restarting the
server invalidates pending results and old picker choices.

Set `"format_on_save": true` in Preferences to request document formatting before saving
(from the language's [formatter](#formatters) when it has one).
It is off by default and can also be set in project preferences or language/path overrides.
Save, Save All, automatic file saves and saving before close wait for formatting, then apply your save cleanup
policies and write the file. The edit is undoable and updates every view of the document.
Manual Format Document and Format Selection remain available independently.

Format Selection appears in Goto > Language Server and the command palette when the active
server advertises range formatting. It formats every nonempty physical selection, including
multiple carets, using the file's tab settings. Empty carets and selections beyond the line end
contribute no text; they do not trigger document formatting. The complete result is one undoable
edit shared by every view. Overlapping selections or server edits, failed requests, changed text,
changed selections or server restarts discard the whole result. Finish any active text composition
before formatting. Format Document remains a separate command.


If you edit while the formatter is working, its old result is discarded and your edits stay
unsaved. A missing/unsupported server, invalid edit, request error, cancellation or timeout also
stops the save before cleanup or disk writing. Explicit saves report the error; automatic saves
pause that edit and report it in the status bar. Edit, change save settings or save explicitly to
retry. Restarting or disabling language servers cancels pending formatting. Finish composition
in every shared view before formatting.

The formatter has its own five-second deadline. `"format_on_save_timeout_ms": 5000` changes it
in Preferences, project preferences or language/path overrides; use a positive whole number
of milliseconds. Other language requests keep their normal timeout. Save As uses the destination's
preference and the named source's server. When formatting is enabled, an untitled file's first
Save As reports that no source route is available; read-only Save As copies skip formatting.

Choose **File > Save Without Formatting** or **File: Save Without Formatting** in the command
palette to save explicitly after a failure. It runs your usual save cleanup and protected writer,
updates the saved point, and opens Save As for an untitled/deleted file. Custom key bindings can
use `workspace::SaveWithoutFormatting`. **Save a Copy As…** is separate: it writes a snapshot to
another destination and preserves the source's unsaved state without formatting or cleanup.

Server snippet completions use the local snippet field editor: Tab/Shift-Tab visit fields,
mirrors follow edits, and choice fields start with their first choice and a keyboard
list. Up/Down selects, Enter confirms, Escape dismisses the list, Tab confirms and moves
forward, and Shift-Tab dismisses and moves back. Provider shutdown preserves those values
and ordinary field navigation. Completion and additional edits are one undo step.
Server completions apply at the primary caret; local
snippets still support multiple carets. Invalid snippets make no changes, and closing the
completion or changing the source view discards a pending resolved result. Resolution can
finish in the background until the existing ten-second request timeout. Nested fields, supported
regex transforms and a final `$0` use the same navigation as local snippets. Explicit server
replacement ranges may replace the rest of the word after the caret; additional edits use the
server's negotiated Unicode coordinates. Overlapping edits or unsupported snippet syntax report
an error in the status bar and leave the document unchanged. Typing more of a completion prefix
before choosing its item still works; other document changes while resolving discard the result.

Installed Sublime grammars override bundled parsers. View > Syntax includes all bundled names,
including Kotlin, Swift and HCL. PHP template lines containing embedded HTML keep Syntect's
HTML/CSS/JavaScript colors and scopes; PHP code lines use Tree-sitter. Files above 16 MiB fall back to an available Syntect grammar or
plain text; LSP documents are limited to 8 MiB and requests to ten seconds. See
[Language intelligence](../../README.md#language-intelligence) for the supported LSP operations
and edit limits.

## Additional language servers

Create `~/Library/Application Support/Orlo/Languages/gleam/language.json`, then run
**Reload Customizations**. The folder name must match `id`. Use strict JSON, without comments
or trailing commas. `id` is a stable 1–40 character key using lowercase ASCII letters, digits,
`-` or `_`, starting with a letter or digit. The server family uses the same key format; the
protocol `language_id` may differ and contains 1–64 bytes without whitespace:

```json
{
  "schema": 1,
  "id": "gleam",
  "name": "Gleam",
  "scope": "source.gleam",
  "files": {
    "extensions": ["gleam"],
    "names": ["Gleamfile"],
    "globs": ["**/templates/*.gleam-template", "Gleamfile.*"],
    "first_line": "^#!.*\\bgleam\\b"
  },
  "lsp": {
    "language_id": "gleam",
    "server": "gleam",
    "command": ["/absolute/path/gleam", "lsp"],
    "initialization_options": {},
    "root_markers": ["gleam.toml", "*.gleam-project"]
  }
}
```

This enables the ordinary completion, definitions, hover, formatting, rename and other LSP
commands for matching files, subject to the server's capabilities. `lsp_enabled` still controls
all servers. `language_id` is the protocol ID sent to the server; `server` is the family key
for sharing a session and for Preferences' `language_servers`. Omit `server` to use `id`.
Preferences replace that family's command/options; an empty Preferences command disables it.
Commands are argument arrays executed directly, with no shell expansion. The program must be
nonblank; arguments may be empty but cannot contain NUL. Initialization options must be a JSON
object. A malformed family in Preferences produces a warning naming that family and keeps its
defaults; valid sibling overrides remain active.
Omitted manifest command/options keep existing family defaults. For a new family, supply a
command here or in Preferences. If several manifests supply defaults for one family, the first
manifest by ID supplying each field wins; configure the family in Preferences to avoid ambiguity.

Names and extensions match without ASCII case; extension entries omit the dot and the longest
ending wins. Names and extensions are checked before `files.globs`, then `files.first_line`.
Globs use the same case-sensitive syntax as EditorConfig and scoped preferences: `*` and `?`
stay within a path component, `**` crosses components, `**/` also matches no parent, `{a,b}`
selects alternatives, `{1..3}` is an inclusive numeric range, and `[abc]`/`[!abc]` are literal
character sets (a hyphen inside a set is literal). Backslash escapes the next character.
A glob without `/` matches a basename anywhere; a glob with `/` matches the full file path.
Leading `/` characters are ignored in both the pattern and path; `/Users/me/src/*.gleam` is
anchored to that full path, while `**/templates/*.gleam-template` matches any templates folder. Invalid or empty globs are
reported with the manifest path; patterns are compiled once when loading, never per detection.
`files.content` can supply a regular expression over the first 4 KiB to distinguish languages
claiming the same name, extension or glob. First-line patterns also see bounded text and apply
when no manifest file claim matches; use an anchored `^#!` pattern for extensionless shebangs.
Globs are limited to 4096 bytes, and detection regexes to 1024 characters. User languages win
ties before bundled definitions, then by stable `id`; matching content patterns beat definitions
without one. Explicit project roots win; loose files search ancestors for `root_markers` or `.git`.
Markers are exact filenames or a filename suffix beginning with `*`.

A matching manifest supplies its `name` and `scope` for language/scope preference overrides,
including `format_on_save`. A valid folder using a bundled ID replaces that language's file
claims and server routing; include every extension you want to retain. A manifest without
`lsp` has no server. Invalid folders are reported with their paths and skipped, leaving other
languages and bundled fallbacks available. Put `.disabled` in a folder to exclude it.

Reload applies to open files, clears old decorations/cached assistance and cancels pending
signature help. Pending server results, including save formatting, are discarded under the
existing save freshness checks. Only changed server commands/options or document protocol/root
routes restart their affected root/family sessions; unchanged processes and document openings
survive even a no-op reload. A changed family affects that family in each active root.
Missing executables are reported in the LSP status or command error with the program path;
install the tool or correct its command, then reload to retry a failed startup. Invalid manifests
report their path and leave unrelated sessions running. Saving a `language.json` within the
support folder also reloads it.
The manifest's `syntax` (a `.sublime-syntax` in the folder) and `tree_sitter` (a WebAssembly
grammar and its queries) fields colour the language; see [Adding a language](adding-a-language.md).
A language with neither uses the Plain Text fallback and still receives its configured LSP
features. A loose `Syntaxes/` grammar claiming a bundled language's extension still replaces that
language's colours, while its manifest keeps supplying comments, indentation and the server.

These manifest fields do take effect:

```json
{
  "comments": { "line": "//", "block": ["/*", "*/"] },
  "indent": { "tab_size": 2, "translate_tabs_to_spaces": true },
  "formatter": { "command": ["/absolute/path/gleam", "format", "--stdin"] }
}
```

- `comments` gives Toggle Comment, Toggle Block Comment and Wrap Lines their markers for the
  language's files. A manifest that names either marker is used as written: one with only
  `line` has no block comment, one with only `block` comments lines with the pair. Leave
  `comments` out to keep Orlo's built-in markers for a syntax of that name. An apostrophe that
  starts a line comment is typed without a closing pair.
- `indent` is the language's default tab width and tabs-or-spaces choice. It applies only where
  you have set neither anywhere: global or scoped Preferences, project preferences, EditorConfig
  or a modeline each win over it, and so does the indentation detected in the file's own text.
  It replaces Orlo's defaults (4, spaces) and nothing else.
- `formatter.command` is run by **Format Document** (and by `format_on_save`) in place of the
  language server. See [Formatters](#formatters).

Without a configured server, the language-server commands report "No language server is
configured for Gleam" and Goto Definition uses the project index. A family whose command is empty
counts as not configured.

## Formatters

A formatter is a program that reads the document on stdin and writes the formatted document to
stdout. Give one in the language's manifest (`formatter.command`, above) or in Preferences:

```json
"formatters": {
  "nim": ["nph", "-"],
  "go": []
}
```

Keys are language ids (`rust`, `go`, `python`, or a manifest's `id`). A Preferences entry
replaces the manifest's command; `[]` turns the language's formatter off, so Format Document
goes back to the language server. `formatters` is application-wide: project and scoped
`formatters` are rejected with a warning, so a project cannot choose a program to run.

When a language has a formatter, **Format Document** and `format_on_save` use it instead of the
language server (Format Selection still asks the server). Commands are argument arrays run
directly, with no shell, from the file's folder, with `ORLO_FILE` and `ORLO_PROJECT` set. The
output replaces the text only when the program exits with status 0, as one undoable edit that
keeps carets outside the changed part where they were. Any other status, output that is not
UTF-8, more than 4 MiB of output, or an edit made while it runs leaves the text unchanged; the
status bar shows the status and the first line of the program's stderr. Format Document stops a
formatter after 60 seconds (or with **Tools > Cancel Running Command**); on save the
`format_on_save_timeout_ms` deadline applies and stops the formatter and anything it started.
A failed save formatter leaves the edits unsaved, as described above.

Server commands run only from explicit global Preferences or the user support-folder manifests.
Project/scoped `language_servers` and `lsp_enabled` settings are rejected with a warning, and
Orlo does not discover language manifests inside a project. To run a project-local executable,
explicitly name its path in a user manifest or global Preferences. Loading a project, a file,
EditorConfig or a modeline cannot authorize or supply another executable. A server can still
run a project's own code as it works (rust-analyzer builds `build.rs` and proc macros; Gradle and
Maven servers run build scripts), so servers start only in [trusted folders](../../README.md#trusted-folders):
until then the status bar shows **Language server waiting · Trust Folder**.

An explicit **View > Syntax** choice takes precedence over extension manifests for preferences
and server routing, and survives Reload Customizations. Named manifests or bundled language
names supply that choice's server; Plain Text has none. Choices remain per-view: a shared
document has one LSP route at a time. Explicit commands and saving use their source view;
ordinary polling uses the same lowest-ID open view across all windows, so conflicting windows
do not repeatedly restart servers. A poll may restore that stable route after another view's
request. Closing its owner selects the next view. A protocol-ID change within a family restarts
that session to isolate old diagnostics. Decorations follow the selected route, while accepted
text edits update every view of the shared document, regardless of its Syntax choice.
Changing a file's path retains the existing automatic syntax redetection behavior.
