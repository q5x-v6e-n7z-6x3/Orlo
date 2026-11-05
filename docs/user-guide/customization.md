# Customization and automation

Customization and automation files live in `~/Library/Application Support/Orlo/`. The **Orlo > Settings** and **Tools** menus,
and the command palette, expose the commands below. Preferences, keybindings, macros and
chrome themes accept JSON comments and trailing commas. Package manifests and task files use
strict JSON; syntax definitions use YAML and color schemes use XML.

## Explorer presentation

Set `"explorer_sort": "natural"` in global Preferences to put `chapter-2` before `chapter-10`.
The default `"lexical"` keeps case-insensitive lexical order. Both keep folders before files;
natural order compares arbitrarily long ASCII digit runs, prefers fewer leading zeros for
equal values and breaks case ties using the original filename. Existing cached tree listings
reorder when preferences change, including refreshed and newly expanded folders.

`"explorer_sticky_ancestors": true` pins parent folders above scrolled tree content. It defaults
to false and works independently of sorting. The nearest ancestors fit in short viewports,
leaving one content row. Pinned folders share normal folder clicks, context menus, marking and
sidebar focus; Up/Down and reveal keep selected rows below the pinned area. OPEN FILES and
project roots retain their own identities. These settings are application-wide and rejected in
project preferences and language/path/scope rules. Path Breadcrumbs menus remain lexical.

## Terminal launch preferences

Global preferences accept the following `terminal` object. Existing `shell`, `arguments` and
`scrollback` preferences keep working. Launch preferences apply to new sessions; running shells
keep their executable, arguments and startup directory.

```json
"terminal": {
  "shell": null,
  "arguments": null,
  "shell_integration": false,
  "scrollback": 10000,
  "profiles": {
    "Development": {
      "shell": "/bin/zsh",
      "arguments": ["-l", "-i"],
      "directory": "/Users/me/Projects",
      "scrollback": 10000
    },
    "Plain shell": {
      "shell": "/bin/sh",
      "arguments": [],
      "scrollback": 2000
    }
  },
  "default_profile": null
}
```

Without a default profile, the default shell is `$SHELL`, falling back to `/bin/zsh`;
`shell: null` keeps that behavior. `arguments: null` keeps `-l -i`; `arguments: []` sends no
arguments. Every argument is one literal string, including spaces, quotes, dollar signs and
empty strings. Orlo does not split or expand them. A missing executable appears as a startup
error in that terminal.

Use the terminal header's **Profiles…**, Terminal > New Terminal with Profile, or **Terminal: New
with Profile…** in the command palette to create a session with a named profile. Choose a name
in `default_profile` to use it for ordinary New Terminal and the first session. Null keeps the
legacy launch fields. Profiles require a shell executable; omitted arguments use `-l -i` and
omitted scrollback uses 10000. Omit `directory` (or use null) for the normal active-session,
active-file, project, home, then root-directory fallback. An unavailable, inaccessible or file
startup path uses that fallback too. Directory paths must be absolute; Orlo does not expand `~`
or variables in them. Selecting another profile creates a separate session and keeps existing
sessions alive. If a profile changes while its chooser is open, choose it again.

Use **Terminal: Set Scrollback…** or Terminal > Set Terminal Scrollback to change only the selected
session's live retention. The form accepts 0–100000 rows. Reducing the limit immediately discards
the oldest history above the limit, including primary-screen history while an alternate-screen
application is active. Zero retains the visible screen and discards all history. Increasing the
limit retains available rows; discarded output cannot return. Changing a limit clears obsolete
Find results, output selections and retained/command review views; a reduction also clears the
bounded command archive and in-flight command capture. A fresh Find uses the remaining output.
Submitting the same limit preserves a paused Find view. The shell keeps running with the same
cursor, modes and working directory. Live overrides are session-local and are not saved.

Keep up to 32 profiles, with trimmed names of 1–128 Unicode characters and no controls.
Arguments must contain no NUL and total at most 32768 bytes; directory paths contain no NUL and
are at most 4096 bytes. Invalid options or an unknown default profile produce the usual
preferences warning and keep default terminal settings. Terminal preferences are global and
cannot be overridden by a document or EditorConfig. Larger retention limits use more memory.

Set `terminal.shell_integration` to `true` to enable Orlo's command-boundary and working-directory
hooks for **new** zsh sessions, including named profiles. It defaults to false; changing it keeps
running sessions as they are. Orlo supports executables named `zsh` with `-i`, `-li` or `-il`,
optionally combined with `-l`; arguments must consist only of these login/interactive flags.
Other shells, noninteractive launches and custom flags such as `-f` start normally without Orlo
hooks. Your arguments stay literal. Orlo reads your normal startup files and preserves user
hooks and ZDOTDIR changes. Normal shell history stays in your logical shell directory, and your
custom `HISTFILE` in .zshrc takes precedence. Orlo writes only a private temporary startup
directory and never modifies your shell configuration files. The PTY worker removes that
directory after the shell exits, including after asynchronous terminal close; retained output
can stay open. Cleanup failures appear in diagnostic output. An integration setup error appears
in the terminal status while the plain shell still starts.

Supported OSC133 markers emitted by your own shell or tools are recognized even when Orlo's
injection is off. Terminals without markers work normally; Orlo does not guess commands from
prompt text. See [command output review](text-essentials.md#review-command-output) for navigation
and clipboard actions.


## Key bindings and the command API

**Preferences: Key Bindings** opens `Default.sublime-keymap`. For example:

```json
[
  {"keys":["cmd-k","cmd-i"],"command":"insert","args":{"text":"Hello"},"context":"Editor && !single_line"},
  {"keys":["cmd-j"],"command":"editor::DuplicateLine","context":"Editor && syntax == source-rust"},
  {"keys":["cmd-k","cmd-d"],"command":null,"context":"Editor"},
  {"keys":["cmd-alt-u"],"command":"editor::ConvertCase","args":{"case":"upper"},"context":"Editor && selection"}
]
```

Each `keys` array is a chord; modifier names are `cmd`, `ctrl`, `alt`, `shift`. Later bindings
win, and `null` unbinds a shortcut. Contexts use GPUI's predicates: `Editor`, `Workspace`,
`single_line`, `multiline`, `selection`, `worksheet`, `completing`, `snippet` and `snippet_choice`;
`syntax` is the syntax's base scope with dots replaced by hyphens (`source-rust`, `text-plain`).
`completing` holds while the completion list shows (Tab, Enter, Up, Down, Escape act on it).
`snippet` holds in a snippet session unless text is being composed (Tab, Shift-Tab, Escape).
`snippet_choice` holds while the active field's choice list is open (Up/Down, Enter, Escape).
`&&`, `||`, `!`, equality and ancestor predicates are available. An invalid file leaves the
previous bindings active. Saving the file or **Reload Customizations** applies changes,
including deleted bindings.

The shared API is `{"command":"namespace::Action","args":{...}}`. All ordinary actions
and editor actions with arguments are available; common workspace arguments include
`workspace::SetSyntax` (`name`), `SetTabSize` (`size`), `SetIndentStyle` (`to_spaces`),
`SetWrapColumn` (`col`), `FocusGroup` (`group`), and `RunPackageCommand` (`package`, `name`).
`insert` is an alias for `editor::InsertText`. Motion names use snake case, for example
`{"command":"editor::Move","args":{"motion":"word_right","select":true}}`.

**Edit > Subwords** and the palette expose Previous/Next Subword, Select Previous/Next
Subword and Delete Previous/Next Subword. They have no default shortcuts; ordinary
Option/Control-arrow word navigation and Option-Backspace/Delete keep their behavior.
For example, these custom bindings use the shared command API:

```json
[
  {"keys":["cmd-alt-right"],"command":"editor::Move","args":{"motion":"subword_right","select":false},"context":"Editor"},
  {"keys":["cmd-alt-shift-left"],"command":"editor::Move","args":{"motion":"subword_left","select":true},"context":"Editor"},
  {"keys":["cmd-alt-backspace"],"command":"editor::DeleteSubword","args":{"forward":false},"context":"Editor"}
]
```

Subwords split `camel|Case`, `HTTP|Server`, `snake|_|case` and `item|42|Value`.
Unicode letters and numbers use the same rule; whole grapheme clusters stay together.
Acronyms end before an uppercase letter followed by a lowercase letter; uncased letters
stay in their letter run. Consecutive underscores, whitespace and punctuation each form
their own segment, so moving backward retraces forward boundaries. Deletion removes selected
source text, otherwise the adjacent segment, in one undo step across all carets. Virtual
positions add no padding; backward deletion from a virtual caret first returns to the line end.
A programmatic caret inside a grapheme deletes the whole containing cluster.

**Edit > Prose** adds paragraph/sentence motion and selection without default shortcuts.
Use `editor::Move` with `paragraph_previous`, `paragraph_next`, `sentence_previous` or
`sentence_next` and the existing `select` Boolean. `editor::SelectSentence` and
`editor::DeleteSentence` are bindable and recordable too. See the
[exact punctuation and whitespace rules](text-essentials.md#paragraph-and-sentence-navigation).

**Edit > Text > Transpose Words** is bindable and recordable as `editor::TransposeWords`,
with no default shortcut. For example:

```json
{"keys":["cmd-alt-t"],"command":"editor::TransposeWords","context":"Editor"}
```

See [word and caret rules](text-essentials.md#transpose-words), including atomic refusal when
multicaret swaps overlap. The existing `ctrl-t` Transpose binding is unchanged.

`workspace::GotoSymbol` (`cmd-r`) searches the current buffer;
`workspace::GotoSymbolInProject` (`cmd-shift-r`) searches the window's index.
`workspace::GotoDefinition` (`f12`, `cmd-alt-down`), `workspace::GotoDefinitionToSide`
(palette) prefer semantic server results and fall back to indexed definitions.
`workspace::GotoReference` (`shift-f12`) always searches lexical text matches with previews and
`def` markers. Semantic `workspace::LspReferences` is available explicitly in the
palette/menu and user bindings. `show_definitions` (default true) enables
500 ms indexed hover previews; `workspace::LspHover` (`ctrl-k ctrl-i`) closes that popup before
requesting semantic documentation. These commands are available in menus, the palette and
custom key bindings.

`workspace::LspTypeDefinition` and `workspace::LspImplementation` request those semantic
destinations; `workspace::LspTypeDefinitionToSide` and `workspace::LspImplementationToSide`
open the chosen target in the side group. They have no default key bindings. Their menu and
palette entries follow the active server's advertised capabilities; direct bindings report an
unsupported method without moving the caret. A multiple-result picker retains the same source
and server guards through confirmation.

Git actions `workspace::GitCommit` and `workspace::GitAmend` open the local staged-commit
and Amend HEAD forms. Both are available in the Git menu, the Git panel and the command palette.
Subject is one line; Body and Coauthors accept newlines. Tab moves between fields,
Enter submits from Subject, and Command-Enter submits from any field. In Coauthors, use one
`Name <email>` per line; identical coauthors are deduplicated against the message footer.
Optional Author name and Author email fields change this commit's author only when both are
filled. Leave both blank to use Git's configured author for a normal commit or preserve the
previous author for Amend. Escape closes the form and retains that mode's message and author
draft in memory for retry. The [Git workflow](../../README.md) describes commit scope and errors.

## Scoped preferences, EditorConfig and indentation

Preferences support ordered overrides:

```json
{
  "detect_indentation": true,
  "overrides": [
    {"language":"Rust","settings":{"tab_size":4,"word_wrap":false}},
    {"scope":"source.rust","path":"**/tests/*.rs","settings":{"tab_size":2}}
  ]
}
```

Every supplied selector must match. Language names are case-insensitive; scopes use
syntect/Sublime scope selectors. Paths use EditorConfig globs: a basename pattern matches
at any depth, and slash patterns match the full path. Later matching entries win.

Indentation precedence is a language's manifest `indent` defaults, open-time detection,
explicitly configured global/scoped values, project preferences, EditorConfig, enabled modelines,
then explicit buffer choices. Window display choices and
per-view wrapping take precedence over file defaults. Detection samples a bounded number of
lines and keeps the configured width when tabs give no evidence of a width. **Guess
Indentation** explicitly repeats detection. Neither opening nor guessing rewrites text.

`theme`, `color_scheme`, `distraction_free`, `lsp_enabled`, `language_servers`, `formatters`, `fold_buttons`,
`fade_fold_buttons`, `draw_white_space`, `drag_text`, `auto_complete`, and
`auto_complete_min_chars` are application-wide. Set them in global preferences; project/scoped
values are rejected with a warning.

`line_number_mode` accepts `"absolute"` (default) or `"relative"` in global, scoped and project
preferences. View > Line Numbers > Absolute / Relative, also in the palette under
**View: Line Numbers**, overrides those defaults for this window until Preferences change or
the window reopens. A custom binding uses `workspace::SetLineNumberMode` with
`"args": {"mode":"relative"}` (or `"absolute"`). Invalid modes report the ordinary settings or
command error. The mode only changes labels; it does not enable a modal editing mode or show a
gutter hidden by distraction-free settings.

`cursor_column_highlight` is an optional boolean file preference (default `false`), accepted
globally, in project preferences and in language/path overrides. Enable **View > Highlight
Cursor Columns**, also **View: Highlight Cursor Columns** in the palette, to override this
window until Preferences change or it reopens. Bind `workspace::ToggleCursorColumnHighlight`
with no arguments to toggle it from a custom key. Each view shades the distinct display
columns of its own selection heads, including virtual carets. Tabs, wide and combining text,
wrapped rows, folds and horizontal scrolling use the same geometry as the caret. Filler rows
stay unshaded. The faint bands sit below selections and other editor highlights and never
change clicks, copied text, saved files or printed source.

`.editorconfig` files are read from parent folders, respecting `root = true`, section order,
nearest-file precedence and `unset`. Supported policies are `indent_style`, `indent_size`
(including `tab`), independent `tab_width`, `end_of_line`, `charset` (`utf-8`/`utf-8-bom`),
`trim_trailing_whitespace`, and `insert_final_newline` (including `false`). Widths are 1–16.
Unsupported values/encodings produce a warning and are ignored; Orlo's usual protection
against overwriting lossily decoded text still applies. Save As uses the destination's policy. Save cleanup
is undoable. Repository policy refreshes on open/save, or when an opened `.editorconfig` is saved.

`format_on_save` is an optional boolean file preference (default `false`), accepted globally,
in project preferences and in language/path overrides. It formats the document before cleanup,
with the language's formatter when one is configured and otherwise through its language server. See [formatting on save](languages.md) for failure and Save As behavior.
`formatters` maps a language id to a formatter's argument array (stdin in, stdout out), or `[]`
to turn the language's own formatter off: `"formatters": {"nim": ["nph", "-"]}`. It is
application-wide. See [Formatters](languages.md#formatters).
`format_on_save_timeout_ms` uses the same scopes and defaults to 5000. It accepts whole
milliseconds from 1 through 4294967295; invalid values produce a warning and retain the inherited
value (5000 globally). A formatter failure leaves edits unsaved; **File > Save Without Formatting**
explicitly skips the formatter while retaining ordinary cleanup and save protection.

## Declarative modelines

Set `"modelines": true` in **Orlo > Settings… > Settings** to read file-embedded preferences.
The default is `false`; project settings and language/path/scope overrides can enable or disable
it for selected files. Saving/reloading settings updates existing views.

Orlo supports this small subset (keys are case-sensitive; language names are case-insensitive):

```text
// vim: set ts=8 sw=2 et wrap tw=80 ft=python:
# -*- mode: python; tab-width: 8; standard-indent: 2; indent-tabs-mode: nil; truncate-lines: nil; fill-column: 80; -*-
// kate: tab-width 8; indent-width 2; replace-tabs on; word-wrap on; word-wrap-column 80; syntax Python;
```

| Vim option (aliases) | Emacs field | Kate field | Meaning / accepted values |
| --- | --- | --- | --- |
| `tabstop=N` (`ts`) | `tab-width: N` | `tab-width N` | Tab display width, 1–16 |
| `shiftwidth=N` (`sw`) | `standard-indent: N` | `indent-width N` | Indent width, 1–16, independently of tab width |
| `expandtab` (`et`), `noexpandtab` (`noet`) | `indent-tabs-mode: nil`, `t` | `replace-tabs on`, `off` | Insert spaces, or tabs |
| `wrap`, `nowrap` | `truncate-lines: nil`, `t` | `word-wrap on`, `off` | Enable or disable soft wrapping |
| `textwidth=N` (`tw`) | `fill-column: N` | `word-wrap-column N` | Wrap column and one ruler, 0–1000; 0 wraps at view width and removes the ruler |
| `filetype=NAME` (`ft`), `syntax=NAME` | `mode: NAME`, or `-*- NAME -*-` | `syntax NAME` | Existing language or syntax, as below |

Margin fields alone keep the earlier wrapping on/off setting. Numbers must be unsigned ASCII
decimal integers. Vim accepts `vi:`, `vim:` or `ex:` at line start or following ASCII whitespace,
then `set` or `setlocal`, whitespace-separated options, and a closing colon. Emacs accepts
semicolon-separated `key: value` fields between a single pair of `-*-` markers. Only the first
complete Emacs pair and first complete Vim directive on a line are read. Kate accepts `kate:`
at line start or after ASCII whitespace, followed by semicolon-separated `key value` fields.
On a mixed line, Vim fields overlay Emacs fields, then Kate fields overlay Vim fields.
Later valid options and later lines win field by field.

Only the first five and last five physical LF lines are scanned, once each in source order. Loaded
CR/CRLF files use the editor's normalized line breaks; soft wrapping and Unicode separators do
not introduce scanned lines. A final LF terminates its record; the editor's extra empty display
row does not consume an edge record. A line longer than 4096 UTF-8 bytes (including its line
break) is ignored entirely. Surrounding Unicode
text is preserved. Edits, undo/redo, reopening, external reload and settings/project/syntax
refreshes update the overlay in all shared views. Removing/invalidating a field or disabling
modelines restores its current lower-priority preference. No background modeline job runs.

Modelines override EditorConfig's supported indentation fields, detection and global/scoped/project
defaults. Manual shared-document indentation and window/per-view display choices
still win; distraction-free wrapping keeps its own presentation. Save As resolves the destination's
enablement and EditorConfig; cancelling leaves the current document alone.

Language values are at most 64 ASCII bytes: letters, digits, spaces and `_+.#()-`. Installed
language-folder IDs/names, bundled protocol IDs and available View > Syntax names are accepted.
Aliases are `js` (JavaScript), `ts` (TypeScript), `c++` (C++), `cs`/`c#` (C#),
`sh`/`shell` (Bash), and `text`/`plain`/`plaintext` (Plain Text). A lowercase `-mode` suffix
is also accepted for bundled IDs/aliases and available syntax names. Unknown languages are ignored. A registered language
without an available highlighting grammar keeps plain highlighting and its registered LSP identity.

View > Syntax always wins. A declared language is derived from the source and is recomputed
when restoring a session. It selects language/scope preferences, but the `modelines` opt-in itself
uses the manual or ordinarily detected language, so a declaration cannot turn its own opt-in on
and off. Removing/disabling a declaration returns to ordinary language detection. Parsing or
loading a declaration never starts a language server. Existing admitted routes keep syncing;
ordinary explicit/triggered assistance retains its existing startup policy.

Unknown or malformed fields are silently ignored, and supported fields on that line can still
apply. Vim commands, expressions and escapes, Emacs `eval`/Local Variables blocks, encoding,
shell and plugin options have no effect. Modelines are never executed and cause no process,
filesystem write or network operation. Applying them does not edit text, change read-only
protection or add undo steps. Save-format fields remain under EditorConfig and explicit save
choices; modelines cannot directly set cleanup, formatting, Unicode warnings or virtual space.

## Unusual Unicode warnings

`unicode_warnings` defaults to `true`, so bidi controls and invisible characters in comments and
strings (the "Trojan Source" trick) are marked; set it to `false` to hide the marks. A project
file cannot change the three warning settings. The editor
paints small `!` overlays without replacing text or changing caret, selection or wrap geometry. **Edit > Character Info** (also
in the command palette) describes configured warning scalars in the cluster after the primary
caret. Descriptions remain available when marks are disabled.

Warning ink is independent of the caret color: literal amber `#F9AE58` is used when its
contrast is at least 4.5:1, otherwise whichever of black/white contrasts more. Normal editor,
selection and current Find-match surfaces choose independently; translucent selection/find
backgrounds are composited over the editor background before choosing ink.

The default list is NUL (U+0000), non-breaking space (U+00A0), soft hyphen (U+00AD), Arabic
letter mark (U+061C), zero width space (U+200B), left/right direction marks (U+200E–F), line
and paragraph separators (U+2028–29), direction embeddings/overrides/pop (U+202A–E), narrow
non-breaking space (U+202F), word joiner (U+2060), direction isolates/pop (U+2066–69), and
embedded BOM/zero width non-breaking space (U+FEFF). These may be intentional: the warnings
are visibility aids, not BiDi security validation or a reason to delete text automatically.

Emoji ZWJ sequences, ZWNJ, variation selectors, combining accents, emoji tags and ordinary
language text are **not** warned by default. Active IME composition is not annotated until
committed. Hidden folded text and single-line input widgets show no warnings. Only displayed
text pieces are inspected, including when a very long line is horizontally clipped.

Use **Preferences > Settings** to enable marks, replace the scalar list, or add codepoints/ranges. For example:

```json
{
  "unicode_warnings": true,
  "unicode_warning_characters": {
    "\u00a0": { "glyph": "!", "description": "Non-breaking space" },
    "\u200b": { "glyph": "?", "description": "Zero width space" }
  },
  "unicode_warning_ranges": {
    "U+0300..U+036F": { "glyph": "?", "description": "Configured combining accents" },
    "U+E000": { "glyph": "!", "description": "Private marker" }
  }
}
```

All three keys support global preferences, language/path/scope overrides and project preferences.
The normal settings precedence applies; each supplied `unicode_warning_characters` object
**replaces the entire scalar list**. Each `unicode_warning_ranges` object replaces its entire
range list, which defaults to `{}`. This example warns for two explicit scalars plus the
configured combining range and private-use codepoint. `{}` empties the corresponding map;
omitting a key retains its inherited map. Scalar-list keys are single Unicode scalars,
written literally or as JSON escapes; `glyph` is one scalar other than control/whitespace,
and `description` is a string. Use visible punctuation for glyphs; invisible or combining
glyphs may not be distinct. Invalid entries report a preferences warning and reject that
setting. An explicit list can include joiners or combining marks without changing their text
shaping. Line terminators omitted from displayed rows are described by Character Info only.
Range keys are `U+XXXX` for a single codepoint or inclusive `U+XXXX..U+XXXX` for a range.
Use the literal uppercase `U+` prefix and 4–6 hexadecimal digits (either hex case), with no
spaces. Endpoints must be ascending Unicode scalars, at most U+10FFFF; surrogate endpoints
and ranges crossing U+D800–U+DFFF are rejected. Scalar-list entries take precedence over
ranges, including default scalar entries. Among overlapping ranges, the greatest numeric
start wins, then the greatest end for equal starts, regardless of JSON object order. Distinct
keys for the same numeric interval are invalid: for example, use only one
of `U+E000`, `U+00e000`, and `U+E000..U+E000`. A collision reports a preferences warning
and rejects that range setting. Descriptions
are your supplied names, shown with the actual codepoint in Character Info; Orlo does not
include a complete Unicode-name database. Lookup never expands a range into its members;
it compiles overlapping intervals when settings load and uses a binary search per displayed scalar.

Warning choices are persisted in preferences, independently of document bytes and undo history.

## Themes and languages

Put TextMate `.tmTheme` XML files in `Themes/`, and Sublime `.sublime-syntax` YAML files in
`Syntaxes/`. Enabled local packages can contain either. The **Color Scheme**, **Theme** and
**Syntax** choosers list installed assets. **Reload Customizations** reloads files
and refreshes existing windows, syntax menus and highlighting. Invalid assets report their
paths and leave usable bundled defaults. Removed syntax definitions fall back to Plain Text.

A language folder in `Languages/<id>/` defines a whole language: which files it claims, its
colouring (a `.sublime-syntax`, or a Tree-sitter grammar compiled to WebAssembly with its
queries), comment markers, default indentation, language server and formatter. A folder with a
bundled language's id replaces that language. Reload Customizations reads the folders again,
then compiles their grammars and queries in the background; their errors join the same alert.
See [Adding a language](adding-a-language.md).

Color schemes supply text/background, selection, caret, gutter and other editor colors.
JSON `.orlo-theme` files supply chrome colors, for example `Themes/Slate.orlo-theme`:

```json
{"sidebar_bg":"#182030","panel_bg":"#203040","ui_fg":"#e0e0e0"}
```

Color names follow the lowercase chrome constants in `crates/config/src/themes/mod.rs`; values are
`#RRGGBB` or `#RRGGBBAA`. Store the chooser's exact relative filename in Preferences
`color_scheme` or `theme` to retain it across restarts. Choosing an asset applies it for
the current run. `Mariana` is the default editor scheme; `auto` follows macOS chrome.

## Snippets

Date/time, clipboard and workspace variables share one snapshot per expansion (local and server
snippets): `CURRENT_YEAR`, `CURRENT_MONTH`, `CURRENT_DATE`, `CURRENT_HOUR`, `CURRENT_MINUTE`,
`CURRENT_SECOND`, `CURRENT_SECONDS_UNIX`, `CLIPBOARD`, `WORKSPACE_NAME`, `WORKSPACE_FOLDER`,
`RELATIVE_FILEPATH`. Time components use local time and fixed padding; clipboard text stays
literal. Roots use the deepest containing folder and its custom label; untitled files use the
first root. Missing values use ordinary snippet defaults, and outside-root relative paths use
the filename. See [Snippets](../../README.md#snippets) for details and the safe Insert Date/Time
form (`editor::InsertDateTime` for a keybinding or macro). No variable evaluates commands.


Put `.sublime-snippet` files in `Snippets/` (subfolders are read too). Orlo reads the folder again
when a file in it changes. A file that does not load is named, with the reason, in one alert per
load, and the others still work. **New Snippet…** (Tools > Snippets) asks for a name and opens a
template under `Snippets/`, written by its first save; **Insert Snippet…** lists the snippets for
the caret's scope. The README's [Snippets](../../README.md#snippets) section describes the format,
triggers, fields, variables, session keys and limits.

**File > New From Template…** (palette: **File: New From Template…**,
`workspace::NewFromTemplate` in a keybinding) searches all local snippets, then asks you to
choose a language. Confirming both creates one unsaved untitled file with the snippet's
ordinary variables, editable fields, mirrors and choices. Search snippets by title, filename
or trigger, and languages by name or registry ID; row details distinguish matching names.
File and selection variables refer
to the new empty document; clipboard, date/time and project variables are captured when the
file is created. Registered languages without a grammar use Plain Text highlighting while
keeping the selected language's preferences. Save asks for a filename through normal Save As.
As for ordinary untitled files, Save As re-detects syntax from the chosen filename.
Cancel creates nothing. A removed, invalid or changed snippet is reported before a file is
created; reopen the picker after changing a definition. Finish or cancel text composition
before opening or confirming the picker. Templates do not execute code.

Choice fields use `${1|one,two|}`; escape literal commas, pipes and backslashes with `\,`,
`\|` and `\\`. The first choice is inserted. Up/Down selects, Enter confirms and leaves the
field selected, Escape dismisses the list, Tab confirms and advances, and Shift-Tab dismisses
and moves back. Mirrors and multiple insertion carets follow confirmation and later typing;
choices remain available after the snippet source reloads or its language server stops.

## Macros

**Start / Stop Recording** (`⌃Q`) records committed text and editor commands, including
navigation and command arguments. Palette/form input and playback are excluded. **Replay
Last** (`⌃⇧Q`), **Repeat Last**, **Save Last**, and **Load and Replay** use the same actions.
Saved macros are editable JSON command arrays in `Macros/<name>.json`; saving refuses an
existing name. Playback accepts editing commands only, with at most 10,000 commands per run.
Clipboard commands use the current clipboard. Mouse gestures and external asynchronous
operations are not a portable macro format.

## Shell filters, tasks and worksheets

**Filter Through Shell** (Tools or the command palette) sends each physical nonempty
selection independently to your explicit `/bin/sh -c` command over standard input. Successful
UTF-8 stdout replaces all ranges together in one Undo step, in source order. One physical
caret still filters the whole document. Empty carets mixed with ranges are ignored; multiple
empty carets, virtual endpoints and overlapping ranges refuse. At most 4096 selections are
admitted. Selected text is data on stdin and is never inserted into the shell command.

**Keep Selections Through Shell…** runs the same independent inputs as a predicate: exit 0
keeps the original range, exit 1 drops it, and any other exit or execution error rejects the
whole batch. Its stdout is ignored and need not be UTF-8. It requires nonempty physical ranges;
it never falls back to the document. This changes only selections, preserving direction and
the primary where retained. **Undo Selection** / **Redo Selection** restores the selection
sets. If all ranges are dropped, one caret remains at the original primary head.

Both actions require an editable source, including predicates: read-only, viewer, Follow,
immutable views and native text composition in any shared view refuse. Failure, cancellation,
invalid replacement UTF-8, closed targets or changed document/selection identity leave source
text and selections untouched and show diagnostics. Changes away and back also invalidate
pending results. Successful stderr is shown without entering the document. **Run Shell Command**
retains its existing single-input output-to-scratch behavior. Scripts and external tools,
including `osascript`, can be invoked explicitly this way.

Processes receive `ORLO_FILE`, `ORLO_PROJECT`, `ORLO_SELECTION_START` and
`ORLO_SELECTION_END` (character offsets, updated for each input). Execution is explicit; merely
opening a project does not run commands. Shells run with the user's permissions. One command
batch runs per window, with one child at a time. **Cancel Running Command** (`⌘.`), window close,
timeout and output overflow stop the running process group and skip remaining inputs. The
whole batch shares a 60-second default deadline and 4 MiB combined stdout/stderr limit.

**Run Task** (`⌘B`) lists user `Tasks.json`, the active root's `.orlo-tasks.json`, and enabled
package tasks. **Edit User Tasks** and **Edit Project Tasks** open examples:

```json
{"tasks":[{"name":"Build","shell":"cargo build","cwd":".","timeout_secs":60}]}
```

Task working directories resolve from their root (or package folder); timeouts allow
1–3600 seconds. GCC `path:line[:column]: message` and Rust `--> path:line:column` diagnostics
become navigable results: double-click or use `F4`/`⇧F4`. Compilers/interpreters must be installed.

**New Shell Worksheet** opens editable text. **Run Worksheet Selection / Line** (`⌘↩` in
the worksheet) executes the selection/current line and appends output as editable shell
comments. Each invocation starts a new `/bin/sh`; there is no PTY, interactive stdin, or
persistent `cd`/variables between invocations.

## Local packages and the developer console

**Install Package** copies a local folder into `Packages/<name>/`. It requires a `package.json`:

```json
{
  "name":"Example",
  "commands":[
    {"name":"Uppercase","shell":"tr '[:lower:]' '[:upper:]'","filter":true},
    {"name":"Insert","shell":"printf '%s' '{\"commands\":[{\"command\":\"insert\",\"args\":{\"text\":\"hello\"}}]}'","api":true}
  ],
  "tasks":[{"name":"Check","shell":"./check.sh"}]
}
```

Enabled commands appear in the palette with the package name. Package commands execute in
their installed folder with `ORLO_PACKAGE` set, plus the file/project environment and text
on stdin. Normal commands open output; `filter:true` independently filters physical ranges (or the document at one caret); `api:true` reads UTF-8
JSON `{ "commands": [...], "text": "optional output" }`. API replies allow at most 128
commands, validate the entire list first, and require an unchanged source buffer/selection.

**Manage Packages** lists, enables, disables and removes installed copies; the source folder
is retained. Names and manifests are validated; installation rejects links/special files and
limits trees to 10,000 entries, 64 MiB and depth 32. There is no registry, marketplace or
automatic updater. Packages work through shell commands and the JSON API; there is no
embedded language runtime.

**Developer Console** keeps a transcript and live input. Enter `help` to list command names,
`state` to inspect the active buffer/window, or a JSON command. Commands act on the source
document even while the transcript is visible. The console is also a place to try commands
before adding them to a keybinding or package. Orlo has no native AppleScript dictionary.

Language folders in `Languages/<id>/` also define language servers and formatters
([Adding a language](adding-a-language.md) covers highlighting too).
**Reload Customizations** applies them to open files, and saving a manifest in Orlo reloads it.
The manifest's name/scope match file preference overrides; an explicit **View > Syntax** choice
uses that grammar's name/scope instead. Manifests support names, extensions, path globs and
first-line/shebang regexes; reload preserves unaffected server processes. Executable commands
belong in global Preferences or user support-folder manifests, including explicit paths to a
project-local tool. Project/scoped server commands remain rejected. See [additional language servers](languages.md#additional-language-servers)
for the format, server sharing, precedence and limits.

`virtual_space` accepts `"off"` (the default), `"rectangular"` or `"all"`. Rectangular mode
allows Option-drag and Add Caret Above/Below carets beyond line ends; all mode also allows
ordinary clicks and navigation. Legacy `false` means off and `true` means all. Set the preference
globally, in a project's preferences or in a scoped settings rule. Virtual positions and
rectangular origins are transient and are not persisted in sessions. See [Virtual space](text-essentials.md#virtual-space) for selection, IME and undo behavior.

Existing surrounds are bindable and recordable as `editor::ChangeSurround` and
`editor::DeleteSurround`. `target` and `replacement` accept `(`, `[`, `{`, `'` or `"` (closing
brackets also identify the same target). `count` is a positive whole number and defaults to 1.
The menu/palette forms record their resolved editor action, so replay applies it at the new
carets. For example:

```json
{"keys":["cmd-alt-t"],"command":"editor::ChangeSurround","args":{"target":"(","replacement":"[","count":1},"context":"Editor"}
```

Delete uses the same `target`/`count` arguments without `replacement`. Neither command has a
default shortcut.

Selection-set actions are recordable and bindable as `editor::InvertSelections` (no arguments),
`editor::SplitSelectionsByRegex`, `editor::KeepSelectionsMatchingRegex`, and
`editor::RemoveMatchingSelections`. The regex actions require `pattern` and accept
`ignore_case` (default `false`). For example:

```json
{"keys":["cmd-alt-s"],"command":"editor::SplitSelectionsByRegex","args":{"pattern":","},"context":"Editor"}
```

Menu/palette forms record the resolved editor action, so replay applies the pattern to the new
selections. Empty `pattern` is a valid zero-width regex. These commands change selection metadata
only, including in Viewer and Follow; active/shared composition refuses before dispatch.

Adjust existing selected values with `editor::AdjustValues`. Omit `step` for +1, or provide a
signed decimal **JSON string**, for example `{"command":"editor::AdjustValues","args":{"step":"-2"}}`.
Every explicit step uses a string, including small steps; JSON numbers are rejected so i128
boundaries survive custom bindings and recorded/replayed macros without 64-bit or float loss.
Workspace actions `workspace::IncrementValues`, `workspace::DecrementValues` and
`workspace::AdjustValuesPrompt` provide the menu/palette routes; no default shortcut is added.

Pipelines accept `{"op":"adjust_values","step":"1"}` (default +1). Each existing scoped input
must be one complete supported value; the step does not scan numbers or split lines. With no
selection, the existing pipeline document scope applies; file runs use the complete file.
An invalid input stops that run before application. Existing pipeline sequencing, line scope
when another step requires lines, preview and file admission remain in effect.

Value-adjustment bindings act on the focused document editor. They refuse while another view
of the same document is composing text; finish or cancel that composition, then invoke again.
A native binding records one resolved `editor::AdjustValues` command in a macro.

The four selection-set bindings (`editor::InvertSelections`, `editor::SplitSelectionsByRegex`,
`editor::KeepSelectionsMatchingRegex`, `editor::RemoveMatchingSelections`) act only on the
focused document editor, including bindings without a context. They refuse while any view of
that document is composing text. Finish or cancel composition and invoke again. They remain
available for selecting text in Viewer, Follow and read-only documents; each native key records
one resolved macro command and uses the ordinary selection Undo history.


Field alignment is bindable and recordable as `editor::AlignOnPattern` and
`editor::AlignSelectedFields`. Pattern alignment requires `pattern`, accepts `capture` as a
nonnegative integer (default `0`, validated against the compiled regex), and `ignore_case`
(default `false`). Selected Fields takes no arguments. For example:

```json
{"keys":["cmd-alt-a"],"command":"editor::AlignOnPattern","args":{"pattern":"=","capture":0},"context":"Editor"}
```

`workspace::AlignOnPatternPrompt` opens the menu/palette form and records its resolved editor
action. Native bindings require the source editor to be focused and refuse while any shared
view is composing. JSON commands and public macro replay use the same guarded handlers. No
new default shortcut or preference is installed. See [Align fields](text-essentials.md#align-fields)
for the source scope and field-column rules.
