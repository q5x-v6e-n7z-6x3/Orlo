# Orlo and 48 text editors: feature inventory and gap audit

Audit date: **2026-10-29**. Orlo baseline: **`31c83cbedfaf33322ab92b5b6bd3d8ed54b81e8b`**, version 0.1.0, macOS local alpha. This report changes documentation only.


The practical shortlist is **terminal search/output links/mouse support, LSP snippet completions/signature help, and improvements to folding and text visibility**. Existing editing/search/automation coverage is already broad. See the [gap catalog](#3-missing-and-partial-features), [prioritized shortlist](#4-prioritized-shortlist), [all 48 editors](#2-coverage-registry-all-48-requested-editors) and [excluded features](#5-excluded-appendix-observed-features-that-should-not-expand-orlo).

## Purpose and evidence rules

Keep Orlo a **text editor with Git and a terminal**. Small improvements to completion, navigation, formatting and diagnostics fit that purpose. Discovering a feature elsewhere does not make it a requirement for Orlo. The recommendation tiers below express product judgment, separately from evidence of support.

The inventory follows Orlo's native menus and command catalog into handlers, models and relevant tests across all ten crates. “Implemented” means a reachable implementation was inspected; it does **not** mean every input or native interaction was exercised in this audit. “Partial” means the basic capability exists but a concrete limitation matters to the comparison. “Missing” is reserved for a capability absent from the inspected responsible subsystem and exposed command surface. “Unknown” means the evidence is insufficient. Existing test files cited below were **inspected**, not rerun, unless the verification section says otherwise.

Peer evidence comes from official manuals, release notes and repositories, accessed on the audit date. A release tag identifies a published baseline; a `main`/`master` commit identifies a development snapshot, not a shipped release. Core, bundled packages and optional extensions are distinguished. Archived features remain historical evidence; an archived client is not proof its hosted services still work. A public issue tracker, SDK or extension repository is not the proprietary editor's core source.

Coverage is comprehensive across the **48 requested products and Orlo's identifiable feature families**. It is not a claim to have executed every editor, audited every upstream line, or enumerated every third-party package. Where an official source cannot settle a capability, it stays unknown. No performance, accessibility or operating-system parity claim is inferred from a feature checklist.

## 1. Orlo's existing feature inventory

The shared [native menus](../../crates/ui/src/views/workspace/menus.rs), [command families](../../crates/ui/src/views/workspace/command_entries.rs) and [editor action catalog](../../crates/ui/src/views/editor/actions.rs) establish discoverability. The [architecture map](../architecture/overview.md) establishes ownership. The following rows group related command variants rather than treating every keyboard binding as a separate feature.

| ID | Capability and inspected behavior | State and material boundary | Implementation and relevant inspected tests |
| --- | --- | --- | --- |
| O01 | Text entry, caret/word/line/page motion, grapheme-aware selection, IME, undo/redo, drag editing | Implemented; cross-view/window text dragging has documented limits | [Input](../../crates/ui/src/views/editor/input.rs), [mouse](../../crates/ui/src/views/editor/mouse.rs), [history](../../crates/editor/src/history/mod.rs), [motion tests](../../crates/editor/src/buffer/tests/motion.rs), [UI motion tests](../../crates/ui/src/views/editor/motion_tests.rs) |
| O02 | Multiple carets; rectangular selection; next/all occurrences and skip; selection expansion to words, lines, paragraphs, brackets and indentation; selection undo/redo | Implemented; selection undo is separate from Sublime's mixed text/selection soft undo | [Selections](../../crates/editor/src/selection/mod.rs), [commands](../../crates/ui/src/views/editor/commands.rs), [multi-caret tests](../../crates/editor/src/buffer/tests/multi_caret.rs), [selection tests](../../crates/editor/src/buffer/tests/selections.rs) |
| O03 | Indent/unindent; duplicate, swap, insert, delete, join, transpose and wrap lines; line/block comments; paired brackets; word/line deletion | Implemented; some comment behavior is text/indentation based rather than syntax aware | [Line editing](../../crates/editor/src/commands/line_editing.rs), [typing](../../crates/editor/src/commands/typing.rs), [line tests](../../crates/editor/src/buffer/tests/lines.rs), [editing tests](../../crates/editor/src/buffer/tests/editing.rs) |
| O04 | Cut/copy/paste, paste-and-indent, copy syntax-colored HTML, clipboard history and clearing | Implemented; HTML uses syntax rather than LSP semantic colors | [Editor commands](../../crates/ui/src/views/editor/commands.rs), [clipboard model](../../crates/editor/src/editing/clipboard_history.rs), [history UI](../../crates/ui/src/components/clipboard_history.rs) |
| O05 | Eight case conversions; line/selection sorting, reverse, deduplicate and shuffle; pattern/numeric sorting; duplicate/matching-line processing; prefixes/suffixes; quote conversion; Unicode normalization; diacritic removal; substitution tables; blank-line/whitespace cleanup | Implemented; cleanup policies can also run on save | [Transforms](../../crates/editor/src/editing/transform.rs), [case/line operations](../../crates/editor/src/editing/mod.rs), [forms](../../crates/ui/src/views/workspace/transforms.rs), [transform tests](../../crates/editor/src/buffer/tests/transforms.rs) |
| O06 | Base64, URL-safe Base64 and hex text conversion; MD5/SHA-1/SHA-256/SHA-512 copying; file hashing; document statistics and character information | Implemented; hex conversion is not a binary-file editor | [Codecs](../../crates/editor/src/editing/convert.rs), [statistics](../../crates/editor/src/editing/stats.rs), [tools](../../crates/ui/src/views/workspace/tools.rs) |
| O07 | Column insertion/numbering, padding, CSV/TSV field selection/copy/move/delete/clear; delimiter detection and override | Implemented; display-width approximations and large-document limits are documented | [Columns](../../crates/editor/src/editing/columns.rs), [CSV](../../crates/editor/src/editing/csv.rs), [column tests](../../crates/editor/src/buffer/tests/columns.rs), [guide](../../README.md#columns-and-csvtsv) |
| O08 | JSON format, minify and validate on selections or a document, with all-target validation and undo | Implemented; preserves numeric spelling/order/duplicates; no schema/tree editor | [JSON model](../../crates/editor/src/editing/json.rs), [UI commands](../../crates/ui/src/views/workspace/json.rs), [guide](../user-guide/text-essentials.md#json-tools) |
| O09 | Local Sublime snippets, scope selectors, triggers, multiple-carets, nested fields, mirrors, regex transforms and variables; create/insert pickers | Partial; no choice fields or shell interpolation; LSP snippet completions are a separate unsupported path | [Snippet model](../../crates/editor/src/editing/snippet.rs), [UI](../../crates/ui/src/views/editor/snippets.rs), [tests](../../crates/editor/src/buffer/tests/snippet_tests.rs), [format and limits](../../README.md#snippets) |
| O10 | Literal and regex find/replace; multiline fields; case/word/wrap/selection options; captures, lookaround/backreferences, replacement case transforms and preserve-case | Partial; bounded backtracking, a specified dialect, no `\G` or `\K`; preserve-case is whole-match based | [Pattern engine](../../crates/editor/src/search/pattern.rs), [search model](../../crates/editor/src/search/model.rs), [workflow tests](../../crates/editor/src/buffer/tests/search_workflow_tests.rs), [regex dialect](../../README.md#regular-expressions) |
| O11 | Find in Files over roots/paths/globs/open files; ignore handling, cancellation, result navigation, filters, result checkboxes and context | Implemented within stated limits; files are read whole with a 100 MB per-file limit | [File search](../../crates/editor/src/search/files.rs), [panel](../../crates/ui/src/panels/search/find_in_files.rs), [results](../../crates/ui/src/panels/search/results.rs), [search tests](../../crates/ui/src/views/workspace/search_tests.rs) |
| O12 | Replace in Files with reviewed previews, selective application, stale-result checks, open-buffer edits and safe closed-file writes | Implemented; open-buffer changes and closed-file writes have distinct undo/storage behavior | [Results model](../../crates/editor/src/search/results.rs), [results UI](../../crates/ui/src/panels/search/results.rs), [search tests](../../crates/ui/src/views/workspace/search_tests.rs), [guide](../../README.md#the-results-browser-and-replace-in-files) |
| O13 | Search history; named saved searches with import/export; regex playground and capture inspector; extraction; bookmarks and bookmarked-line operations; five highlight styles; mark/yank | Partial; named searches persist, ordinary query history/highlights do not; bookmarks belong to views | [Saved queries](../../crates/editor/src/search/saved.rs), [playground](../../crates/ui/src/panels/search/playground.rs), [extraction](../../crates/editor/src/search/extract.rs), [bookmark commands](../../crates/editor/src/commands/bookmarks.rs), [marking tests](../../crates/editor/src/buffer/tests/marking_tests.rs) |
| O14 | Saved text pipelines with validation, preview, selection/document execution and batch-file runs with cancellation/reporting | Implemented; transformations are explicit, open files remain unsaved, closed files use protected writes | [Pipeline model](../../crates/editor/src/search/pipeline.rs), [orchestration](../../crates/ui/src/views/workspace/pipelines.rs), [format](../../README.md#text-pipelines) |
| O15 | Open/save/save-as/save-all/revert; Unicode BOM handling; explicit reopen/save encoding; read-only and malformed-input protection; LF/CRLF policies and safe metadata-preserving writes | Partial; encoding enum is UTF-8/UTF-16 LE/BE only; mixed endings normalize on save | [Encoding](../../crates/project/src/documents/encoding.rs), [I/O](../../crates/project/src/documents/io.rs), [save orchestration](../../crates/ui/src/documents/save.rs), [file tests](../../crates/editor/src/buffer/tests/files.rs), [concurrent-save tests](../../crates/ui/src/documents/save_tests.rs) |
| O16 | External-file change detection, clean-buffer reload, dirty-buffer decisions, deletion markers and settings refresh | Partial; focused-window polling, not native FSEvents or a tail/log-follow mode | [Watch decisions](../../crates/project/src/watchers/mod.rs), [polling and UI](../../crates/ui/src/views/workspace/disk.rs), [watch tests](../../crates/editor/src/buffer/tests/watch.rs) |
| O17 | File/folder comparison, aligned side-by-side text and intra-line changes, review/copy operations and conflict-aware writes | Implemented; this is already a diff capability, not a missing feature | [Comparison models](../../crates/project/src/documents/comparison/mod.rs), [UI](../../crates/ui/src/views/workspace/comparison.rs), [reviewed copy](../../crates/project/src/filesystem/reviewed_copy.rs), [comparison copy UI](../../crates/ui/src/views/workspace/comparison/copy.rs) |
| O18 | Opt-in historical disk-byte backups; browse read-only versions, restore as undoable unsaved text, clear history and bounded retention | Implemented; defaults off; separate from session recovery, Git history and disk autosave | [History storage](../../crates/project/src/documents/file_history.rs), [history UI](../../crates/ui/src/views/workspace/history.rs), [guide](../user-guide/text-essentials.md#historical-backups), [capture tests](../../crates/editor/src/buffer/tests/files.rs) |
| O19 | Hot exit, private unsaved/untitled recovery, window/tab/layout/position restoration, project workspaces, corrupt-session recovery, signal handling | Partial; not persistent undo history or restored terminal processes; folds/highlights/search results excluded | [Session storage/tests](../../crates/workspace/src/sessions/mod.rs), [project workspaces](../../crates/workspace/src/sessions/project.rs), [UI sessions](../../crates/ui/src/views/workspace/sessions.rs), [signal tests](../../tests/integration/signals.rs) |
| O20 | CLI files/folders and line/column targets, stdin buffer, `--wait`, literal paths, help/version, one-instance handoff, Dock file opens | Implemented; UTF-8 stdin capped at 1 MiB | [CLI parser/tests](../../crates/workspace/src/navigation/cli.rs), [handoff](../../crates/workspace/src/navigation/handoff.rs), [app entry](../../crates/app/src/main.rs), [CLI integration tests](../../tests/integration/cli.rs) |
| O21 | Multi-root projects, exclusions, `.sublime-project` compatibility, switch/recent projects, explorer keyboard navigation and file operations | Partial; projects open in one window at a time, local filesystem only, cross-volume moves refused; project comments not retained | [Project model](../../crates/project/src/directories/project.rs), [explorer](../../crates/project/src/explorer/mod.rs), [project UI](../../crates/ui/src/views/workspace/projects.rs), [file operations](../../crates/ui/src/views/workspace/fileops.rs) |
| O22 | Preset/nested/resizable groups, named layouts, shared document views across windows, per-group tab strips, tab multiselect, preview/pinned tabs, drag/move and tab lists | Implemented; constrained group geometry; shared text/undo with independent view selections | [Layouts/tests](../../crates/workspace/src/layout/mod.rs), [tabs/tests](../../crates/workspace/src/tabs/mod.rs), [groups UI](../../crates/ui/src/views/workspace/groups.rs), [tab UI](../../crates/ui/src/views/workspace/tabs.rs), [shared documents/tests](../../crates/ui/src/documents/tests.rs) |
| O23 | Soft/hard wrapping, rulers, minimap, whitespace marks, synchronized scrolling, focus/full-screen/distraction-free modes, dockable/resizable panels, auto light/dark appearance and animation settings | Implemented; synchronized scrolling follows logical lines; long-line/width limits remain | [Viewport](../../crates/editor/src/viewport/mod.rs), [appearance](../../crates/ui/src/views/workspace/appearance.rs), [docks](../../crates/ui/src/views/workspace/docks.rs), [distraction mode](../../crates/ui/src/views/workspace/distraction.rs), [variant tests](../../crates/ui/src/views/workspace/variant_tests.rs) |
| O24 | Fold/unfold/all/levels and gutter fold controls, nested folds, exact single-line selection folding and edit mapping | Partial; automatic region discovery is indentation based and fold state is not session-persistent | [Fold model](../../crates/editor/src/rendering/fold.rs), [editor commands](../../crates/ui/src/views/editor/commands.rs), [fold tests](../../crates/ui/src/views/editor/fold_tests.rs), [session schema](../../crates/workspace/src/sessions/mod.rs) |
| O25 | Goto Anything/line/symbol/project symbol/definition/reference, outline, jump history, counterpart and related files, hover previews | Implemented; lexical and semantic references are explicitly distinct; index is bounded | [Project index](../../crates/ui/src/views/workspace/index.rs), [navigation](../../crates/ui/src/views/workspace/goto.rs), [related-file model/tests](../../crates/syntax/src/detection/related.rs), [fuzzy matcher/tests](../../crates/utils/src/fuzzy/mod.rs) |
| O26 | Fifteen bundled Tree-sitter language families, incremental parsing/highlighting, installed Sublime grammar overrides, syntect fallback, HolyC and FreeBASIC grammars | Implemented within parser/size limits; not every language has a bundled parser/server | [Language detection](../../crates/syntax/src/detection/language.rs), [parsers/tests](../../crates/syntax/src/grammars/treesitter.rs), [highlight worker/tests](../../crates/syntax/src/highlighting/highlight.rs), [language guide](../user-guide/languages.md) |
| O27 | Local word/symbol/path/snippet completion; context ranking; server completion on explicit request | Partial; LSP snippet items are filtered out, server results commit at the primary caret, and typing does not request the server | [Local completion/tests](../../crates/syntax/src/languages/complete.rs), [server completion UI](../../crates/ui/src/views/workspace/language.rs), [completion parsing/tests](../../crates/syntax/src/detection/language.rs) |
| O28 | LSP diagnostics, semantic colors, hover, definitions/references, symbol rename, manual document format, selected code actions and restart/status UI | Partial; no signature-help/inlay-hint/range-format feature dispatch, pull diagnostics, server-initiated workspace edits or file operations | [LSP transport/tests](../../crates/syntax/src/languages/lsp/mod.rs), [UI feature dispatch](../../crates/ui/src/views/workspace/language.rs), [fixture UI tests](../../crates/ui/src/views/workspace/language_tests.rs), [real-server smoke tests](../../crates/syntax/src/languages/lsp/real_servers.rs) |
| O29 | macOS spell checking, suggestions, next/previous error, dictionaries, learn/ignore and scope exclusions | Implemented; external platform behavior and VoiceOver were not runtime-tested here | [Spell service/tests](../../crates/syntax/src/languages/spell.rs), [editor spelling](../../crates/ui/src/views/editor/spelling.rs), [workspace spelling](../../crates/ui/src/views/workspace/spelling.rs) |
| O30 | Editable chord/context keymaps; scoped settings; indentation detection; EditorConfig; themes/color schemes/grammars and reload | Partial compatibility with those formats, with explicit supported keys and warnings | [Settings](../../crates/config/src/settings/mod.rs), [keybindings](../../crates/config/src/keybindings/mod.rs), [theme assets](../../crates/config/src/themes/assets.rs), [customization UI](../../crates/ui/src/views/workspace/customization.rs), [guide](../user-guide/customization.md) |
| O31 | Macros; shell filters/commands; user/project/package tasks with navigable diagnostics; worksheets; local shell packages and JSON command API; developer console | Implemented, explicitly invoked; no marketplace, embedded scripting runtime or AppleScript dictionary; worksheet executions do not share shell state | [Automation config](../../crates/config/src/persistence/automation.rs), [automation UI](../../crates/ui/src/views/workspace/automation.rs), [customization UI](../../crates/ui/src/views/workspace/customization.rs), [guide](../user-guide/customization.md#shell-filters-tasks-and-worksheets) |
| O32 | Git repository discovery/init, multi-repository status, file badges, whole-file/line/hunk stage and unstage, staged-only commits | Implemented; saved-file staging and stale-index/unsaved-buffer protections | [Git API](../../crates/git/src/lib.rs), [status](../../crates/git/src/status/mod.rs), [partial staging](../../crates/git/src/staging/selection.rs), [Git panel](../../crates/ui/src/panels/git/mod.rs), [Git tests](../../crates/git/src/tests.rs) |
| O33 | Local branch list/create/switch/rename/delete, linked-worktree repository discovery and branch-delete safeguards | Partial; existing linked worktrees are recognized, but there is no worktree-management UI, remote Git or stash UI | [Branches](../../crates/git/src/branches/mod.rs), [repository discovery](../../crates/git/src/repository/mod.rs), [branch UI](../../crates/ui/src/panels/git/branch.rs), [Git tests](../../crates/git/src/tests.rs) |
| O34 | Git file history through renames, paged older commits, commit diff, blame, working/index/HEAD comparisons, gutter changes, undoable hunk reversion and conflict-marker choices | Implemented; not a repository-wide graph or full three-way merge tool; text-size/encoding constraints apply | [History](../../crates/git/src/history/mod.rs), [blame](../../crates/git/src/blame/mod.rs), [versions](../../crates/git/src/diff/versions.rs), [conflicts](../../crates/git/src/diff/conflicts.rs), [history UI](../../crates/ui/src/panels/git/history.rs), [change UI](../../crates/ui/src/views/workspace/editors.rs) |
| O35 | Integrated login-shell PTY; terminal keys/IME/paste/copy/selection; ANSI color, alternate screen, cursor/device replies, resize, 10,000-line scrollback; hide without killing | Partial; one per window, no terminal find, mouse forwarding, clickable links, exposed profiles or shell-state restoration | [Terminal backend/tests](../../crates/terminal/src/sessions/mod.rs), [emulation/tests](../../crates/terminal/src/emulation/mod.rs), [PTY/tests](../../crates/terminal/src/pty/mod.rs), [panel](../../crates/ui/src/panels/terminal/mod.rs), [input/rendering/tests](../../crates/ui/src/panels/terminal/emulator.rs) |
| O36 | Native macOS window/menu/Dock integration, bundled app tooling and performance instrumentation | Implemented macOS baseline; Windows/Linux, notarized distribution and full accessibility verification are separate limits, not new editor-feature recommendations | [Startup](../../crates/app/src/startup/mod.rs), [native integration](../../crates/ui/src/platform/macos/mod.rs), [instrumentation](../../crates/ui/src/platform/perf.rs), [benchmarks](../../benches/mod.rs), [release assessment](release-readiness.md) |
| O37 | Language-folder manifest validation, registry construction/detection and reload generation | Backend-only/partial groundwork: reload runs, but runtime language-server routing still uses the fixed `Language` enum, and the registry's detection/LSP/formatter/query data is not wired into those editor paths | [Manifest/tests](../../crates/syntax/src/languages/manifest.rs), [registry/tests](../../crates/syntax/src/languages/registry.rs), [reload call](../../crates/syntax/src/highlighting/highlight.rs), [runtime routing](../../crates/ui/src/views/workspace/language.rs), [earlier proposal](../language-support-plan.md) |

No feature was classified as implemented merely because a backend type or old plan mentioned it. In particular, remote Git/stash and LSP inlay hints are **not** existing Orlo capabilities; ordinary local snippets and manual formatting **are**. Historical test totals in the [2026-10-21 audit](feature-audit-2026-10-21.md) are previous evidence, not results from this audit.

### Documentation discrepancy found

The README's [known limits](../../README.md#not-in-this-version) say folds never hide part of a line. The current `Editor::fold` handler explicitly folds an exact nonempty selection within one line, and the [fold tests](../../crates/ui/src/views/editor/fold_tests.rs) exercise partial-line behavior. **Inline selection folding already exists and is not a gap.** Syntax-driven region discovery and restoring folds across launches remain separate questions. The source takes precedence here; the README itself was not changed by this documentation-only comparison.

## 2. Coverage registry: all 48 requested editors

Each name opens its detailed profile below, which covers editing/search, files/recovery, views/customization, languages, Git and terminal evidence. “Unarchived” is a repository flag, not a support guarantee. Historical platform lists do not establish compatibility with current operating systems. Exact commits and source limitations are recorded in the profiles.

| # | Editor | Examined release or snapshot | Platforms / observed state | Official docs and source |
| --- | --- | --- | --- | --- |
| 01 | [Acme](#peer-01) | dev 1f139691 (2026-10-21) | Unix/macOS; unarchived snapshot, no release resolved | [docs](https://9fans.github.io/plan9port/man/man1/acme.html); [source](https://github.com/9fans/plan9port) |
| 02 | [Atom](#peer-02) | 1.60.0 (2022-03-08) | Historical Windows/macOS/Linux; archived | [docs](https://github.com/atom/flight-manual.atom.io); [source](https://github.com/atom/atom) |
| 03 | [Bluefish](#peer-03) | 2.4.2 (2026-07-17) | Linux/BSD/macOS/Windows; 2026 release | [docs](https://bluefish.openoffice.nl/features.html); [source](https://sourceforge.net/p/bluefish/code/9359/tree/tags/bluefish_2_4_2/) |
| 04 | [Brackets](#peer-04) | 2.2.1 (2023-03-22) | Windows asset; macOS asset in 2.2.0; maintenance mode | [docs](https://brackets.io/); [source](https://github.com/brackets-cont/brackets) |
| 05 | [Editra](#peer-05) | 0.7.20; dev b9d547d2 (2019-11-15) | Historical Linux/macOS/Windows; unarchived, current support unverified | [docs](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/PKG-INFO); [source](https://github.com/cjprecord/editra) |
| 06 | [FeatherPad](#peer-06) | V1.6.4 (2026-06-22) | Linux focus; Haiku/macOS/OS2 ports; 2026 activity | [docs](https://github.com/tsujan/FeatherPad/blob/V1.6.4/featherpad/data/help); [source](https://github.com/tsujan/FeatherPad) |
| 07 | [Geany](#peer-07) | 2.1.0 (2025-07-06) | Unix/Windows/macOS; unarchived, 2026 activity | [docs](https://github.com/geany/geany/blob/2.1.0/doc/geany.txt); [source](https://github.com/geany/geany) |
| 08 | [Gedit](#peer-08) | 51.0 (2026-10-15) | Linux/macOS/Windows installation routes; 2026 release | [docs](https://gedit-text-editor.org/features.html); [source](https://gitlab.gnome.org/World/gedit/gedit) |
| 09 | [GNOME Text Editor](#peer-09) | 51.0 (2026-09-14) | GNOME/Linux; 2026 release | [docs](https://apps.gnome.org/TextEditor/); [source](https://gitlab.gnome.org/GNOME/gnome-text-editor) |
| 10 | [Gobby](#peer-10) | 0.6.0 (2021-01-31) | Historical Windows/macOS/Unix claims; unarchived, 2026 CI work | [docs](https://gobby.github.io/); [source](https://github.com/gobby/gobby) |
| 11 | [GPHPedit](#peer-11) | 0.9.91 (2006-07-05 folder) | GNOME2/Unix; old source artifact, current support unknown | [docs](https://sourceforge.net/projects/gphpedit/); [source](https://downloads.sourceforge.net/project/gphpedit/gPHPEdit/0.9.91/gphpedit-0.9.91.tar.gz) |
| 12 | [jEdit](#peer-12) | 5.7.0 (2024-08-03 source) | Java11+; Windows/macOS/Unix, legacy ports; stable vs 5.8pre1 separate | [docs](https://www.jedit.org/index.php?page=features); [source](https://github.com/jEdit-editor/jEdit) |
| 13 | [Kate](#peer-13) | 26.08.2 (2026-10-08) | Linux/BSD/Windows/macOS; 2026 release; plugin availability varies | [docs](https://kate-editor.org/about-kate/); [source](https://invent.kde.org/utilities/kate) |
| 14 | [Komodo Edit](#peer-14) | 12.0.1 (published 2022-12-15) | Historical Windows/macOS/Linux; final vendor build, repo closed notice | [docs](https://github.com/Komodo/komodo-documentation/blob/7f2c35687dcca6247004d97eb76228cd0abc83f6/en-US/editor.html); [source](https://github.com/Komodo/KomodoEdit) |
| 15 | [KWrite](#peer-15) | 26.08.2 (2026-10-08) | Linux and Windows/macOS links; shared source, different app mode | [docs](https://apps.kde.org/kwrite/); [source](https://invent.kde.org/utilities/kate) |
| 16 | [Leo](#peer-16) | 6.8.10 (2026-08-17) | Python/Qt Windows/macOS/Linux; 2026 release | [docs](https://leo-editor.github.io/leo-editor/commands.html); [source](https://github.com/leo-editor/leo-editor) |
| 17 | [Light Table](#peer-17) | 0.8.1 (2016-01-22) | Historical Windows/macOS/Linux; archived | [docs](https://docs.lighttable.com/); [source](https://github.com/LightTable/LightTable) |
| 18 | [Mousepad](#peer-18) | 0.7.0 (2026-03-02) | Xfce/GTK Unix desktop; 2026 release | [docs](https://docs.xfce.org/apps/mousepad/start); [source](https://gitlab.xfce.org/apps/mousepad) |
| 19 | [NEdit](#peer-19) | 5.8.0 (April 2026) | Motif/X11 Unix, legacy VMS; later master is not this release | [docs](https://sourceforge.net/p/nedit/git/ci/5cc0adfcfd8f003f2c4f4af3fcc19e26d62ecfbe/tree/doc/help.etx); [source](https://sourceforge.net/p/nedit/git/) |
| 20 | [Notepad++](#peer-20) | 8.9.8.1 (2026-09-24) | Windows x86/x64/ARM64; 2026 release | [docs](https://github.com/notepad-plus-plus/npp-usermanual); [source](https://github.com/notepad-plus-plus/notepad-plus-plus) |
| 21 | [Pe](#peer-21) | 2.5.0 (2025-05-14) | Haiku; unarchived despite organization name; 2026 activity | [docs](https://github.com/HaikuArchives/Pe/tree/v2.5.0/doc); [source](https://github.com/HaikuArchives/Pe) |
| 22 | [Pluma](#peer-22) | 1.28.1 (2026-01-22) | MATE/GTK Unix desktop; 2026 release | [docs](https://github.com/mate-desktop/pluma/blob/v1.28.1/help/C/index.docbook); [source](https://github.com/mate-desktop/pluma) |
| 23 | [SciTE](#peer-23) | 5.6.7 (2026-09-28) | Open-source Windows/Linux edition; commercial Mac edition separate | [docs](https://www.scintilla.org/SciTEDoc.html); [source](https://www.scintilla.org/scite567.tgz) |
| 24 | [Scribes](#peer-24) | 0.3.3.2 (2008 archive) | Historical GNOME/Python2; explicitly unsupported 0.3; 0.4 source unavailable | [docs](https://scribes.sourceforge.net/features.html); [source](https://sourceforge.net/projects/scribes/files/OldFiles/scribes-0.3.3.2.tar.bz2/download) |
| 25 | [TEA](#peer-25) | 63.3.1 (2025-03-31) | Linux/BSD/macOS/Windows/OS2/Haiku claims; build-dependent features | [docs](https://github.com/psemiletov/tea-qt/blob/63.3.1/manuals/en.html); [source](https://github.com/psemiletov/tea-qt) |
| 26 | [Textadept](#peer-26) | 13.0 (2026-07-01) | Windows/macOS/Linux GUI and TUI; BSD source builds; 13.1 beta excluded | [docs](https://github.com/orbitalquark/textadept/blob/textadept_13.0/docs/manual.md); [source](https://github.com/orbitalquark/textadept) |
| 27 | [TextMate](#peer-27) | 2.0.23 (2021-10-12) | macOS 10.12+ claim; unarchived, 2024 last observed push | [docs](https://macromates.com/textmate/manual/); [source](https://github.com/textmate/textmate) |
| 28 | [Xed](#peer-28) | 3.9.0, master.lmde7 (2026-09-21) | Linux/GTK; distribution release tag, immutable source pinned | [docs](https://github.com/linuxmint/xed/blob/9be0bdcd6f17c119d41543440066da9287c12d0e/README.md); [source](https://github.com/linuxmint/xed) |
| 29 | [ZeroBrane Studio](#peer-29) | 2.01 (2023-09-26) | Windows/macOS/Linux; unarchived, 2026 source activity | [docs](https://studio.zerobrane.com/features); [source](https://github.com/pkulchenko/ZeroBraneStudio) |
| 30 | [BBEdit](#peer-30) | 16.0.3 (2026-08-25) | macOS 14+; current commercial release; license modes differ | [docs](https://www.barebones.com/products/bbedit/comparison.html); Proprietary core unavailable |
| 31 | [Chime](#peer-31) | stable2.2.5; dev84b7afa4 (2026-04-28) | Stable macOS 12+; rewrite needs newer SDK/OS; stable core closed | [docs](https://www.chimehq.com/download); [source](https://github.com/ChimeHQ/Chime) |
| 32 | [CodeEdit](#peer-32) | 0.3.6 (2025-08-26) | macOS 13+; published development release, not production-ready claim | [docs](https://github.com/CodeEditApp/CodeEdit/releases/tag/v0.3.6); [source](https://github.com/CodeEditApp/CodeEdit) |
| 33 | [CotEditor](#peer-33) | 7.1.3 (2026-10-27) | macOS 26+; 2026 release/activity | [docs](https://coteditor.com/); [source](https://github.com/coteditor/CotEditor) |
| 34 | [Lapce](#peer-34) | 0.4.6 (2026-01-21) | Windows/macOS/Linux; unarchived, 2026 activity | [docs](https://docs.lapce.dev/); [source](https://github.com/lapce/lapce) |
| 35 | [Lite XL](#peer-35) | 2.1.8 (2025-06-08) | Windows/macOS/Linux release; additional ports; 2026 activity | [docs](https://lite-xl.com/user-guide/introduction/); [source](https://github.com/lite-xl/lite-xl) |
| 36 | [Nextpad++](#peer-36) | 1.1.2 (2026-10-05) | Examined Mac port requires macOS 12+; Linux separate; 2026 activity | [docs](https://nextpad.org/online-help/); [source](https://github.com/nextpad-plus-plus/nextpad-plus-plus-macos) |
| 37 | [Pulsar](#peer-37) | 1.132.1 (2026-05-20) | Windows/macOS/Linux; unarchived, 2026 activity | [docs](https://docs.pulsar-edit.dev/using-pulsar/); [source](https://github.com/pulsar-edit/pulsar) |
| 38 | [Sublime Text](#peer-38) | Build4215 (2026-09-25) | Windows/macOS/Linux; current proprietary release | [docs](https://www.sublimetext.com/docs/); Proprietary core unavailable |
| 39 | [Nova](#peer-39) | 14.1 (2026-08-06) | macOS 15.6+ on download page; commercial maintained release | [docs](https://help.nova.app/); Proprietary core unavailable |
| 40 | [Rune](#peer-40) | 1.3.2 (2026-10-26) | macOS/Linux x86-64/ARM64 artifacts; 2026 activity | [docs](https://docs.rune.build/); [source](https://github.com/unstablebuild/rune) |
| 41 | [Fresh](#peer-41) | 0.5.4 (2026-10-20) | Linux/macOS/Windows/FreeBSD; GUI artifacts separate maturity; 2026 activity | [docs](https://getfresh.dev/docs); [source](https://github.com/sinelaw/fresh) |
| 42 | [Lite](#peer-42) | 1.11 (2020-07-05) | Windows/Linux release; unarchived, last observed push2023 | [docs](https://github.com/rxi/lite/blob/v1.11/README.md); [source](https://github.com/rxi/lite) |
| 43 | [COSMIC Text Editor](#peer-43) | dev ec8375c6 (2026-10-29) | COSMIC/Linux; no latest GitHub Release resolved; README/product mismatch | [docs](https://system76.com/cosmic/apps); [source](https://github.com/pop-os/cosmic-edit) |
| 44 | [Focus](#peer-44) | 0.3.8 (2025-02-27) | Windows/macOS/Linux64; 2026 main/nightlies distinct from stable | [docs](https://focus-editor.dev/); [source](https://github.com/focus-editor/focus) |
| 45 | [Flow Control](#peer-45) | 0.7.2 (2026-02-14) | Linux/FreeBSD/macOS/Windows/Termux TUI; release/site mismatch | [docs](https://flow-control.dev/docs/); [source](https://github.com/neurocyte/flow) |
| 46 | [Athas](#peer-46) | 0.17.0 (2026-10-15) | macOS/Windows/Linux; browser workbench separate; 2026 activity | [docs](https://athas.dev/docs); [source](https://github.com/athasdev/athas) |
| 47 | [TTT Editor](#peer-47) | 1.9.0 (2026-10-23) | macOS/Linux/Windows/FreeBSD/Android; 2026 release | [docs](https://tttedit.dev/); [source](https://github.com/eugenioenko/ttt) |
| 48 | [Rhun](#peer-48) | 0.19.0 (2026-10-27) | Linuxx64/macOS ARM64/Windowsx64; 2026 release | [docs](https://rhun.app/); [source](https://github.com/vshvedov/rhun) |

## 3. Missing and partial features

These are differences from the named peers, not a requirement to add every feature. **M** = missing built-in workflow; **P** = existing feature with a narrower capability. Each row links Orlo's responsible implementation and specific peer documentation/source. “Worth adding,” “optional” and “defer” are recommendations, not claims that implementation cost has been measured. Manually invoking a shell tool can cover some workflows without a dedicated UI.

### Files and document handling

| ID / state | Difference and user benefit | Orlo evidence | Peer documentation and implementation | Recommendation |
| --- | --- | --- | --- | --- |
| F01 / M | Legacy encodings and UTF-32: read/reinterpret/convert Windows/ISO/CJK files, with clear conversion errors | O15: [Encoding enum][orlo-encoding] contains only UTF-8/UTF-16; invalid input stays protected | [Geany released encoding guide][geany-manual], [codec table/conversion][geany-encoding]; [CotEditor help][cot-encoding] and [save/load code][cot-document] | **Worth adding** when these files occur; medium effort across open/save/search/diff |
| F02 / M | Native paginated printing and system PDF output, optionally selected text, colors and line numbers | O36: [menus][orlo-menus] have no Print action; README explicitly excludes print | [CotEditor print help][cot-print-doc], [print layout][cot-print-src]; [Mousepad guide](https://docs.xfce.org/apps/mousepad/start#printing), [implementation][mp-print] | **Optional**; ordinary editor function, medium effort |
| F03 / P | Explicit read-only log-follow mode that follows appended output without normal editing | O16: [disk watcher][orlo-disk] reloads clean buffers; no monitor/follow command or mode | [Notepad++ Live File Monitoring][npp-doc-views], [dispatch][npp-commands] and [monitoring I/O][npp-io] | **Optional**; useful for text/log work, medium effort |
| F04 / M | Opt-in autosave to the actual document file | O18/O19: [save flow][orlo-save] is explicit; session autosave writes recovery data, not the original file | [Rune release/editor guide][rune-editor], [autosaver][rune-autosave] | **Optional**; keep separate from existing recovery and backups |
| F05 / M | Native remote single-file open/save | O21: [document I/O][orlo-io] and [file actions][orlo-files] use local paths, with no URI/remote-document backend | [Pluma help][pl-help], [GIO loader][pl-loader]; [BBEdit FTP/SFTP docs](https://www.barebones.com/products/bbedit/benefitscommand.html), proprietary core unavailable | **Defer**; external mounts/tools can cover it; broadens I/O and credential scope |
| F06 / M | Transparent gzip text-document editing | O15: [I/O][orlo-io] decodes file bytes without a gzip-document layer | [jEdit feature documentation](https://www.jedit.org/index.php?page=features), [released gzip load path][jedit-gzip] | **Defer**; useful but less central than ordinary text files |
| F07 / M | Rescue of untitled documents after an explicit discard/close | O19: [close flow][orlo-files] and [session flow][orlo-sessions-ui] distinguishes Quit recovery from Discard All, which loses untitled text; ordinary hot exit already exists | [BBEdit feature comparison](https://www.barebones.com/products/bbedit/comparison.html) documents discarded-untitled Document Rescue; **documentation-only**, proprietary core unavailable | **Optional**; distinct recovery enhancement, not “add backups” |

### Editing, display and document state

| ID / state | Difference and user benefit | Orlo evidence | Peer documentation and implementation | Recommendation |
| --- | --- | --- | --- | --- |
| E01 / P | Syntax-driven fold discovery for braces/comments/markup rather than indentation alone | O24: [fold commands][orlo-fold-command] call indentation-region functions; exact selected-range folding already works | [Atom folding guide][atom-fold-doc], [Brackets bundled syntax helpers][brackets-fold]; maintained [Fresh LSP folding docs][fresh-lsp-doc] corroborate the capability | **Worth adding**; improve the existing feature, medium effort |
| E02 / P | Restore collapsed regions across close/reopen or session restore | O19/O24: [session schema][orlo-session] stores positions/bookmarks but not fold ranges | [Textadept 13 manual][ta-manual] and [session serializer][ta-session]; [TextMate fold metadata][tm-document] and [view restore][tm-editor] | **Worth adding**; small visible extension of existing session state; medium effort |
| E03 / P | Configurable warning glyphs/list for NBSP, zero-width and other troublesome codepoints | O06/O23: [settings][orlo-settings] expose whitespace marks, while [character info][orlo-stats] is manual; no configurable warning-codepoint list | [CodeEdit0.3.6 release](https://github.com/CodeEditApp/CodeEdit/releases/tag/v0.3.6), [warning settings][ce-settings], [list UI][ce-warning-list] | **Worth adding**; focused text visibility; not a claim of complete BiDi security |
| E04 / M | Overtype mode with an explicit insert/overwrite indicator | O01: [editor actions][orlo-actions] and [typing][orlo-typing] provide insertion but no overwrite mode | [Notepad++ editing manual][npp-doc-edit], [toggle dispatch][npp-notify] | **Optional**; small-to-medium effort, avoid accidental mode confusion |
| E05 / P | General virtual-space caret editing beyond EOL | O02/O07: [selection model][orlo-selection] stores document offsets; [column tools][orlo-columns] explicitly pad, rather than exposing a configurable virtual-space mode | [Notepad++ virtual-space docs][npp-doc-edit], [Scintilla option wiring][npp-main] | **Optional**; existing column padding already covers many cases |
| E06 / M | Limited declarative modelines embedded in files | O30: [settings resolution][orlo-settings] supports scoped preferences/EditorConfig, not Vim/Emacs/Kate modelines | [gedit modeline docs](https://gedit-text-editor.org/user-manuals/gedit/gedit-plugins-modelines.html), [Pluma parser][pl-modelines-parser] and [plugin integration][pl-modelines-plugin] | **Optional**; settings-only subset, not executable per-file scripts |
| E07 / P | Named, reusable clipboard registers saved across runs | O04/O13: [clipboard history][orlo-clipboard] plus mark/yank exist; no named persistent register store in [session schema][orlo-session] | [jEdit registers/kill-ring features](https://www.jedit.org/index.php?page=features), [released Registers implementation][jedit-registers] | **Defer**; existing history/macro facilities cover much of the value |
| E08 / P | Elastic tabstops that align tab-separated blocks to their contents | O07/O23: [viewport tab expansion][orlo-viewport] uses fixed tab stops; explicit pad/CSV commands are different | [jEdit indentation guide](https://www.jedit.org/users-guide/indent.html), [elastic tab-width implementation][jedit-elastic] | **Defer**; adds layout complexity, not a missing CSV editor |
| E09 / M | A real optional Vim/modal command interpreter | O01/O30: [action/keymap system][orlo-actions] is modeless; rebinding shortcuts does not provide operator/motion state | [GNOME released feature docs][gnome-readme], [Vim input-context integration][gnome-vim]; [Kate compatibility docs](https://docs.kde.org/stable_kf6/en/kate/katepart/vi-input-mode-chapter.html) | **Defer**; large behavioral surface; not needed for the stated product |

### Lightweight language support

| ID / state | Difference and user benefit | Orlo evidence | Peer documentation and implementation | Recommendation |
| --- | --- | --- | --- | --- |
| L01 / P | Accept LSP snippet-format completions with editable fields | O09/O27: [completion_items][orlo-lsp-ui] drops insertTextFormat=2; [client capabilities][orlo-lsp] advertise snippetSupport=false despite local snippets working | [Lapce completion overview][lapce-readme], [completion insertion][lapce-editor], [snippet parser/tabstops][lapce-snippet]; exact subtype behavior is **source-verified** | **Worth adding**; reuse existing snippet concepts, medium effort |
| L02 / P | Signature help showing the current call and parameter | O28: [Feature dispatch][orlo-lsp-ui] and [capabilities][orlo-lsp] do not request signatureHelp; hover is already supported | [Fresh released LSP guide][fresh-lsp-doc], [requests/rendering][fresh-lsp-src]; [TTT released LSP guide][ttt-lsp-doc], [client][ttt-lsp-src] | **Worth adding**; small language aid, not a debugger |
| L03 / P | Format the selected range rather than only the document | O28: [LspFormat][orlo-lsp-ui] requests textDocument/formatting with document-level options; no rangeFormatting dispatch | [Fresh range-format docs][fresh-lsp-doc], [request implementation][fresh-lsp-src]; [TTT client][ttt-lsp-src] | **Optional**; document formatting already exists |
| L04 / P | Opt-in formatter on save | O28/O30: [settings][orlo-settings] only expose save cleanup policies; [save flow][orlo-save] has no formatter hook | [TTT format-on-save docs][ttt-lsp-doc]; Fresh [external-formatter save hook][fresh-save-src] is source evidence for that variant, not proof it always formats through LSP | **Optional**; preserve explicit control and existing save behavior |
| L05 / P | Configure a language server for an arbitrary additional language/filetype without recompiling | O26/O37: [Language::detect][orlo-language-detect] and [server routing][orlo-lsp-ui] use fixed variants; [manifest loader][orlo-manifest] is groundwork, not end-to-end integration | [Fresh custom-language/server guide][fresh-lsp-doc], [language/server configuration maps][fresh-config-src] and [language-keyed server manager][fresh-lsp-manager]; [Geany filetype configuration][geany-manual] is a separate lexical reference | **Optional**; connect existing language data before adding a plugin runtime |

### Git workflows

These are built-in UI differences. All remain possible through Git in the existing terminal. Orlo's local-only policy is explicit, so these are product choices rather than accidental omissions.

| ID / state | Difference and user benefit | Orlo evidence | Peer documentation and implementation | Recommendation |
| --- | --- | --- | --- | --- |
| G01 / M | Fetch/pull/push and remote/upstream controls | O32/O33: [Git exports][orlo-git] and [Git panel][orlo-git-ui] expose local operations; README explicitly promises no remote contact | [Pulsar Git guide][pulsar-git-doc], [bundled Git command implementation][pulsar-git-src]; [CodeEdit fetch][ce-fetch], [pull][ce-pull], [push][ce-push] | **Optional**, only if deliberately changing the local-only policy |
| G02 / M | Stash list/create/apply/pop/drop with preview | O33: [Git API][orlo-git] has no stash workflow | [Kate Git guide](https://kate-editor.org/post/2023/2023-02-01-using-kate-git-support/), [stash dialog][kate-stash]; [Athas Git guide](https://athas.dev/docs/git-integration), [stash commands][athas-stash] | **Optional**; terminal already provides it |
| G03 / P | Create/open/remove/prune worktrees from the UI | O33: [repository discovery][orlo-git-repo] recognizes linked worktrees and [branches][orlo-branches] protect them; no lifecycle commands | [Athas worktree guide](https://athas.dev/docs/git-worktrees), [lifecycle commands][athas-worktree], [window/workspace opening][athas-worktree-open] | **Optional**; basic open/create first if demand appears |
| G04 / P | Amend controls and assisted commit composition/coauthor fields | O32: [commit backend][orlo-commit] runs commit --message only; manually typed trailers and Git-configured signing already work through Git | [Pulsar commit guide][pulsar-git-doc], [amend/trailer command construction][pulsar-git-src]; [Nova guide](https://help.nova.app/git/sidebar/) is proprietary documentation corroboration | **Optional**; do not call ordinary commits or signing entirely missing |
| G05 / P | Repository-wide history browser, beyond active-file history | O34: [history backend][orlo-git-history] is file-specific and [history UI][orlo-git-history-ui] is attached to a file | [TTT released Git guide][ttt-git-doc], [log implementation][ttt-git-src]; Nova's [history/graph guide](https://help.nova.app/git/sidebar/) is documentation-only | **Defer**; a full graph/client is not necessary to the editor |

### Terminal usability

| ID / state | Difference and user benefit | Orlo evidence | Peer documentation and implementation | Recommendation |
| --- | --- | --- | --- | --- |
| T01 / P | Search retained terminal output without restarting the command | O35: [terminal state/API][orlo-term-state] has screen/selection/scroll, but no query or match navigation; editor Find is separate | [Pulsar terminal guide][pulsar-term-doc], [find palette][pulsar-term-find]; [Athas terminal guide](https://athas.dev/docs/terminal), [regex/case/word UI][athas-term-search] | **Worth adding**; practical daily terminal feature |
| T02 / P | Click output paths/URLs, including file:line:column resolved after cd | O35: [terminal view][orlo-term-ui] handles selection, not link activation; cwd is the initial PathBuf and [emulation callbacks][orlo-term-emulation] do not track OSC7 | [Fresh clickable-path/OSC7 docs][fresh-term-doc], [path detector][fresh-path-src], [cwd parser][fresh-term-src]; [Athas web/file links][athas-term-addons] | **Worth adding**; start with file paths and cwd, medium effort |
| T03 / P | Forward requested application mouse reporting while retaining a selection override | O35: [mouse handlers][orlo-term-ui] always select text; [terminal input][orlo-term-input] has key/paste sequences only | [Fresh forwarding contract][fresh-term-doc], [policy and input routing][fresh-mouse-src]; [Rhun manual][rhun-guide] and [released event handling][rhun-term-src] | **Worth adding**; compatible TUI use within the terminal |
| T04 / P | More than one terminal tab/session per window, optionally split | O35: [terminal panel][orlo-term-panel] owns one terminal entity; editor splits do not create additional PTYs | [Fresh terminal tabs/splits docs][fresh-term-doc]; [Athas nested terminal split implementation][athas-term-splits] | **Optional**; small tab set before a terminal multiplexer |
| T05 / P | User-selected shell/arguments and scrollback retention settings | O35: [login shell][orlo-shell] uses SHELL with fixed login flags; [configuration][orlo-term-config] defaults to 10,000, without an exposed [settings][orlo-settings] field | [Lapce terminal profiles](https://docs.lapce.dev/get-started/terminal), [profile model][lapce-term]; [TTT terminal guide][ttt-term-doc] and [startup][ttt-term-src] | **Optional**; backend configuration is not yet user-facing |
| T06 / P | Prompt/command boundaries, exit status and copy-last-output navigation | O35: [emulator callbacks][orlo-term-emulation] answer CSI queries; [terminal state][orlo-term-state] does not model OSC133 command ranges | [Athas terminal guide](https://athas.dev/docs/terminal), [OSC133 marker handling][athas-shell-src], [shell launcher][athas-shell-launch]; [CodeEdit hooks][ce-shell] | **Optional**; defer the extra UI until simpler terminal gaps are addressed |
| T07 / M | Send selected editor text into the existing interactive terminal | O31/O35: [shell filters/worksheets](../user-guide/customization.md#shell-filters-tasks-and-worksheets) start separate commands; [terminal actions][orlo-term-panel] only show/close | [Kate terminal guide](https://docs.kde.org/stable_kf6/en/kate/kate/kate-application-plugin-konsole.html), [slotPipeToConsole implementation][kate-terminal]; [Geany VTE implementation][geany-vte] | **Optional**; distinguish insert/paste from executing with a newline |

## 4. Prioritized shortlist

The smallest useful direction is to improve capabilities Orlo already has. This audit does not authorize implementing the entire catalog.

**Worth adding:** terminal output search (T01), cwd-aware output links (T02), requested terminal mouse support (T03), LSP snippet completions (L01), signature help (L02), syntax-driven folding (E01), fold restoration (E02), and configurable unusual-character warnings (E03). Legacy encoding support (F01) is also valuable if real files require it; it touches more storage paths than a chooser alone suggests.

**Optional:** print/PDF, log follow, disk autosave, discarded-draft rescue, overtype/virtual space, modelines, range formatting, format on save, additional-language LSP routing, a few terminal profiles/tabs, selection-to-terminal and simple command-boundary integration. Remote Git, stash, worktree and amend UI fit Git scope, but should be chosen deliberately instead of adding a large Git client by default.

**Defer:** remote document transports, compressed/archive editing, named persistent registers, elastic tabstops, modal emulation and repository-wide history/graphs. These are valid editor features, but lower priorities for this product. Existing terminal tools already cover several of them.

For a first small feature release, choose **terminal search plus output links**, or **LSP snippet completion plus signature help**. These are suggested alternatives, not a requirement to bundle both. Each eventual implementation needs its own behavior specification and focused tests.

## 5. Excluded appendix: observed features that should not expand Orlo

The source dossiers below preserve details and distinguish actual features from plans. These exclusions are product decisions based on the requested scope.

| Direction | Representative official evidence | Why excluded from the recommended feature set |
| --- | --- | --- |
| Debuggers, breakpoints, watches, remote/device debugging and DAP | [ZeroBrane features](https://studio.zerobrane.com/features); [Kate app/plugins](https://kate-editor.org/about-kate/); [Rhun extension guide][rhun-guide] | Belongs in the separate IDE project |
| Advanced semantic refactoring suites and project analysis | [Komodo edition-labelled manual](https://github.com/Komodo/komodo-documentation/blob/7f2c35687dcca6247004d97eb76228cd0abc83f6/en-US/editor.html); [Pulsar IDE packages](https://docs.pulsar-edit.dev/ide-features/community-packages/) | Keep ordinary existing rename/code actions; do not build a new refactoring platform |
| AI chat/agents, agent session orchestration and generated commit messages | [Rune released manual][rune-editor]; [BBEdit licensed feature comparison](https://www.barebones.com/products/bbedit/comparison.html); [Rhun guide][rhun-guide] | Independent assistant/IDE scope, not an editing requirement |
| Live evaluation, notebooks and browser-connected programming | [Light Table workflow][lt-workflow]; [ZeroBrane features](https://studio.zerobrane.com/features) | Runtime tooling rather than text editing |
| HTML/CSS Quick Edit, browser live preview, site publishing/deployment | [Brackets](https://brackets.io/); [Bluefish features](https://bluefish.openoffice.nl/features.html); [Nova](https://help.nova.app/) | Specialized web-development environment |
| Collaboration servers, live participant cursors, chat and ACLs | [Gobby release README](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/README.md); [Atom sunset announcement](https://github.blog/news-insights/product-news/sunsetting-atom/) | Multi-user service architecture; historical services may no longer operate |
| Editable outline databases, node clones and literate-programming document models | [Leo directives](https://leo-editor.github.io/leo-editor/directives.html) | Changes what an Orlo document is; symbol outline already exists |
| SQL/database browsers, cloud/container development and remote execution platforms | [Kate plugins](https://kate-editor.org/about-kate/); [Fresh LSP/remote guide][fresh-lsp-doc]; [Athas docs](https://athas.dev/docs) | Separate IDE/infrastructure concerns |
| Hosted PR/review/account integrations | [TTT Git guide][ttt-git-doc]; [Pulsar Git guide][pulsar-git-doc] | Basic local Git does not require a hosting-service client |
| Embedded general-purpose plugin runtimes and large extension marketplaces | [jEdit features](https://www.jedit.org/index.php?page=features); [Lite XL IDE setup](https://lite-xl.com/user-guide/ide-setup/); [Sublime package docs](https://www.sublimetext.com/docs/packages.html) | Existing local shell packages, grammars, macros and JSON command API already provide customization |
| Rich Markdown/image/archive previews, ebook/media readers and graphical canvases | [Pulsar core packages](https://docs.pulsar-edit.dev/core-packages-and-features/); [TEA manual][tea-manual] | Convenience beyond the chosen text/Git/terminal core |
| Per-file notes, clipboard collectors and elaborate workspace dashboards | [TEA manual][tea-manual]; [Leo technical introduction](https://leo-editor.github.io/leo-editor/preface.html) | Plain scratch files/worksheets cover basic note taking; no new organizer required |
| Full terminal daemon, remote multiplexer and automatic process resurrection | [Fresh session guide][fresh-session-doc]; [Rune](https://docs.rune.build/) | A much larger process-management product; restoring text/layout is different |
| Ports to every peer's operating system, mobile/web clients and compatibility with all legacy platforms | [Coverage registry](#2-coverage-registry-all-48-requested-editors) | Record platform differences; do not turn them into a macOS-first porting backlog |

Native source printing is deliberately **not** excluded as word processing. Read-only log following is **not** excluded as IDE tooling. Conversely, syntax-language counts, toolkit choice and claims of speed are not standalone parity gaps.

## 6. Unknowns, limitations and already-covered features

- **Do not add duplicates:** column numbering with increment/repetition/base/padding already matches the Nextpad++ example; CSV tools, JSON format/validate, regex playground, pipelines, clipboard history, snippets, macros, spelling, CLI waits/stdin, hot exit, file history, diff/conflict tools, multi-root projects, shared views and local Git already exist. See O01–O36. Code folding includes inline selection folding.
- **Accessibility and BiDi need runtime evidence:** Kate's [accessible text interfaces][kate-accessible] and [direction-aware renderer][kate-renderer] are useful references. Orlo's VoiceOver, mixed Arabic/Hebrew/Latin caret geometry and assistive-technology exposure were not validated here. Do not turn that uncertainty into either “fully supported” or “definitely missing.” Configurable Unicode warning glyphs are a narrower, separately verified UI gap.
- **Further LSP differences:** Fresh documents multiple servers per language and feature routing; Rhun documents extension-backed inlay hints/code lens. Orlo's [client capabilities][orlo-lsp] and [Feature dispatch][orlo-lsp-ui] lack those workflows. Orlo does not request pull diagnostics, advertises no resource operations, and rejects server-initiated workspace/applyEdit. Exact peer implementations for every such protocol detail were not audited, so these are documented follow-up differences rather than source-verified backlog entries. Do not add a general IDE/LSP framework merely to close them.
- **Other Orlo bounds:** query history/highlights/folds and undo history have different persistence rules; find-in-files/index/LSP/diff have different file-size limits; preserve-case and regex dialect are bounded; mixed line endings normalize; some very long lines approximate layout/coloring; native disk events are replaced by focused-window polling. These remain documented limits, not proof that every peer solves each case. See [current known limits](../../README.md#not-in-this-version), [search engine](../../crates/editor/src/search/pattern.rs), [viewport](../../crates/editor/src/viewport/mod.rs) and the source inventory.
- **Closed source:** BBEdit, Sublime Text and Nova are documentation-only for their editor cores. Chime 2.x is closed, whereas the public3.x rewrite is a development snapshot. Public SDKs/scripts/packages are not core implementation evidence.
- **Historical or incomplete sources:** Atom/Light Table are archived; Komodo Edit has a final-vendor-build/closure notice; Editra is an old source snapshot. The requested GPHPedit is the exact SourceForge 0.9.91 artifact. Scribes 0.3 is explicitly unsupported and the advertised0.4 development source was inaccessible. No maintained fork was silently substituted.
- **Access failures:** final HTTP checks returned 404 for three former Komodo12 manual URLs; they were replaced with reachable official 9.2-era manual source, with the release mismatch explicit in P14. Some SourceForge tree/raw views returned 403 to automated HTTP requests; official SVN/Git/source archives were inspected where documented, and those access restrictions are not feature-absence evidence.
- **Moved or misleading sources:** official jEdit now points to GitHub rather than the requested old SVN core. KWrite disables Kate application plugins despite sharing engine/source. COSMIC Terminal is separate from COSMIC Text Editor. A TUI, REPL, shell worksheet or build-output panel is not automatically an embedded PTY terminal.
- **Release/docs mismatches:** gedit 51 removed/consolidated older plugins; FeatherPad/TEA no longer support their old broad codec lists; current Textadept docs are 13.1 beta while this audit pins 13.0. Lite and Lite XL remain distinct. Brackets 2.2.1 only supplies a Windows artifact; preceding 2.2.0 supplies Mac/Windows. Flow's current site and release README disagree about completion. Details remain in the profiles.
- **No comparative runtime benchmarks:** no startup/typing/large-file memory rankings, reliability rankings, accessibility conformance scores or whole-product parity percentages were measured. A native toolkit or a faster data structure alone does not prove better behavior.

## 7. Detailed primary-source dossiers

These profiles preserve the broad feature evidence behind the registry. Each covers the requested editor rather than substituting a successor, fork or another edition. **A feature mentioned here is not automatically missing from Orlo**: only section3 classifies confirmed differences. “Not established”/“NV” means unknown from the examined evidence, not absence. Build-dependent plugins and separately installed packages remain labeled. All source inspection is static; the peer applications were not installed or exercised interactively.

<a id="peer-01"></a>

### P01. Acme (plan9port)

| Area | Observed evidence |
|---|---|
| Version and status | Examined **development snapshot** [`1f1396913fb5372408f442ed26933f5ae4b399cd`](https://github.com/9fans/plan9port/commit/1f1396913fb5372408f442ed26933f5ae4b399cd), 2026-10-21. GitHub releases list was empty; repository API reported `archived=false`. [Repository/API](https://api.github.com/repos/9fans/plan9port), [source repo](https://github.com/9fans/plan9port). |
| Platforms | Official plan9port page lists Linux, FreeBSD, macOS, NetBSD, OpenBSD, SunOS, and DragonFly BSD. Its architecture list is visibly historical, so do not infer a current architecture support matrix. Windows is only linked as an early reference port. [Platforms](https://9fans.github.io/plan9port/). |
| Editing and search | Text editing, undo/redo, literal Look, regex/line/character addresses, and the Sam editing command language. Mouse chords select/cut/paste/execute; ordinary displayed text can act as a command. [Acme manual](https://9fans.github.io/plan9port/man/man1/acme.html). |
| Files, recovery, views | Files and directories are editable windows arranged in adjustable columns. Zerox creates another view. Dump/Load save/restore window state; an unexpected exit writes `acme.dump`, with a documented limitation that restarting external commands can lose their prior information. [Acme manual](https://9fans.github.io/plan9port/man/man1/acme.html). |
| Customization and languages | Per-window tag command text, font and tab controls; external programs interact with Acme's served file interface. This is a language-agnostic text environment; bundled syntax highlighting, LSP, printing and codec menus were not established. [Acme(4)](https://9fans.github.io/plan9port/man/man4/acme.html), [manual](https://9fans.github.io/plan9port/man/man1/acme.html). |
| Git and terminal | Arbitrary shell commands and `<`, `\|`, `>` filters; `win` embeds a command/shell in an Acme text window. That is a distinct interaction model from a modern VT terminal. A native Git UI was NV; shell execution is not evidence of one. [Manual, Support programs](https://9fans.github.io/plan9port/man/man1/acme.html). |

Distinctive transferable behavior: recognize `path:line` and regex/character addresses in any text using the current document's directory as context. Inspected [`src/cmd/acme/look.c`](https://github.com/9fans/plan9port/blob/1f1396913fb5372408f442ed26933f5ae4b399cd/src/cmd/acme/look.c), including filename expansion and address navigation. This is only a candidate for Orlo's navigation audit. Its 9P programmable UI and mouse-chord command environment are architecture/input-system differences, not automatic backlog requirements.

<a id="peer-02"></a>

### P02. Atom

| Area | Observed evidence |
|---|---|
| Version and status | Latest observed stable **1.60.0**, released 2022-03-08; 1.61.0-beta0 is explicitly a prerelease. Release source [`03c9ad93793b5e78ea7fbcdcdabf38fd4065fcc0`](https://github.com/atom/atom/commit/03c9ad93793b5e78ea7fbcdcdabf38fd4065fcc0). GitHub API reports archived; README announces 2022-12-15 archival. [Release](https://github.com/atom/atom/releases/tag/v1.60.0), [README](https://github.com/atom/atom/blob/master/README.md), [repo](https://github.com/atom/atom). |
| Platforms | Historical packages for macOS, Windows, and 64-bit Linux. These are historical distribution claims, not present-day OS compatibility guarantees. [README installation sections](https://github.com/atom/atom/blob/master/README.md). |
| Editing and search | Multiselections, line/word manipulation, snippets, fuzzy navigation, bookmarks; regex and project find/replace; syntax/indent-based folds and arbitrary selected-region folds, fold all and fold at indentation level. [Flight Manual source](https://github.com/atom/flight-manual.atom.io/tree/678ba0247a61b836a6691521810cb5417b052eef/content/using-atom/sections), [folding](https://github.com/atom/flight-manual.atom.io/blob/678ba0247a61b836a6691521810cb5417b052eef/content/using-atom/sections/folding.md). |
| Files, recovery, views | Tree/project navigation, split panes and pending preview items are documented. Editor serialization and encoding APIs exist; a full crash-durability guarantee was not established in this pass. [Panes](https://github.com/atom/flight-manual.atom.io/blob/678ba0247a61b836a6691521810cb5417b052eef/content/using-atom/sections/panes.md), [editor source](https://github.com/atom/atom/blob/03c9ad93793b5e78ea7fbcdcdabf38fd4065fcc0/src/text-editor.js). |
| Customization and languages | Packages/themes, CSS-style scoped keymaps, user styles, initialization scripts, language-scoped settings and custom extension-to-grammar associations. Bundled spell checking and live Markdown preview. [Customization](https://github.com/atom/flight-manual.atom.io/blob/678ba0247a61b836a6691521810cb5417b052eef/content/using-atom/sections/basic-customization.md), [writing](https://github.com/atom/flight-manual.atom.io/blob/678ba0247a61b836a6691521810cb5417b052eef/content/using-atom/sections/writing-in-atom.md). |
| Git and terminal | Bundled GitHub package documented repository initialization/clone, branches, file/hunk/line staging, discard, commit/amend/undo, push/fetch/pull and conflict handling. GitHub PR/review functionality was service dependent and is historical. A bundled interactive terminal was NV. [GitHub package manual](https://github.com/atom/flight-manual.atom.io/blob/678ba0247a61b836a6691521810cb5417b052eef/content/using-atom/sections/github-package.md). |

Candidate details: arbitrary folding and depth folding, project-wide bookmark picker, spell checking, language-scoped settings and Markdown preview. Inspected [`src/text-editor.js` fold methods](https://github.com/atom/atom/blob/03c9ad93793b5e78ea7fbcdcdabf38fd4065fcc0/src/text-editor.js#L4797) and [`src/text-editor-registry.js`](https://github.com/atom/atom/blob/03c9ad93793b5e78ea7fbcdcdabf38fd4065fcc0/src/text-editor-registry.js). The archived package registry and GitHub/Teletype-style services are not current operational feature evidence. The final Flight Manual commit postdates 1.60; candidate implementation links use released core code.

<a id="peer-03"></a>

### P03. Bluefish

| Area | Observed evidence |
|---|---|
| Version and status | Official site announces **2.4.2**, 2026-07-17; 2.4.1 made side-by-side view stable. Release tag `bluefish_2_4_2` appears in official SVN, created at revision 9359. A 2026 release and later SVN changes provide maintenance evidence; no archive declaration was observed. [News/home](https://bluefish.openoffice.nl/), [source tag](https://sourceforge.net/p/bluefish/code/9359/tree/tags/bluefish_2_4_2/), [repo](https://sourceforge.net/p/bluefish/code/). |
| Platforms | Officially listed Linux, macOS, Windows, FreeBSD, OpenBSD. GVfs remote files/site upload and other optional functions have explicit native Windows/macOS limitations. [Home](https://bluefish.openoffice.nl/), [feature footnotes](https://bluefish.openoffice.nl/features.html). |
| Editing and search | PCRE search/replace with subpattern replacements, files-on-disk search, recursive opening by filename/content patterns, external filters, snippets, bookmark navigation and code folding. [Feature list](https://bluefish.openoffice.nl/features.html). |
| Files, recovery, views | Project-specific restoration, many tabs/documents, GVfs remote files across several protocols, modified-document crash recovery, fullscreen and now-stable side-by-side view. [Features](https://bluefish.openoffice.nl/features.html), [2.4.1 release](https://bluefish.openoffice.nl/). |
| Customization and languages | Editable language definitions, nested language completion, tag/bracket matching, reference information, and language-aware spell checking of comments/strings (requires Enchant). Character map needs Gucharmap. Multiple encodings convert through internal UTF-8. [Features](https://bluefish.openoffice.nl/features.html). |
| Git and terminal | External program/build/lint/filter integration is documented; a bundled Git UI or VT terminal was NV in examined evidence. [External program/filter entries](https://bluefish.openoffice.nl/features.html). |

Concrete candidate evidence: [encoding docs](https://bluefish.openoffice.nl/features.html) plus inspected [`src/document.c`](https://sourceforge.net/p/bluefish/code/9359/tree/tags/bluefish_2_4_2/src/document.c), `buffer_find_encoding` around line 1850 (in-file charset declarations, UTF-8 validation, GLib conversion); [folding docs](https://bluefish.openoffice.nl/features.html) plus inspected [`src/bftextview2.c`](https://sourceforge.net/p/bluefish/code/9359/tree/tags/bluefish_2_4_2/src/bftextview2.c), `block_fold_tags` around 2088 and toggle methods. HTML toolbars, image/thumbnail insertion and site publishing are web-development scope, not default Orlo gaps. The online “Definitive Guide” still labels itself version 1.0.6 (2006), so current release/source takes precedence.

<a id="peer-04"></a>

### P04. Brackets (brackets-cont)

| Area | Observed evidence |
|---|---|
| Version and status | Latest observed release **2.2.1**, 2023-03-22; pinned source [`e41ba100d1955f027abf9223cc2cd38e3b7c9adb`](https://github.com/brackets-cont/brackets/commit/e41ba100d1955f027abf9223cc2cd38e3b7c9adb). Current README explicitly says maintenance mode, security patches only, with active work moved to Phoenix Code. API archive flag is false, so do not call this repo archived. [Release](https://github.com/brackets-cont/brackets/releases/tag/v2.2.1), [README](https://github.com/brackets-cont/brackets/blob/46a4252f3e77d23ca2ec3b80d08069fc269fa005/README.md), [repo](https://github.com/brackets-cont/brackets). |
| Platforms | Observed 2.2.1 asset is Windows-only (`Brackets-2.2.1.exe`); preceding 2.2.0 provides Windows and macOS installers. Do not infer a current Linux build or a 2.2.1 macOS artifact from historic Brackets material. [2.2.1 assets](https://github.com/brackets-cont/brackets/releases/tag/v2.2.1), [2.2.0 assets](https://github.com/brackets-cont/brackets/releases/tag/v2.2.0). |
| Editing and search | Code editor with project/folder search/replace, language hints and bundled folding. Folding loads brace/comment/Markdown/indent/selection helpers, supports collapse/expand all, and restores valid saved folds. [FindInFiles source](https://github.com/brackets-cont/brackets/blob/e41ba100d1955f027abf9223cc2cd38e3b7c9adb/src/search/FindInFiles.js), [CodeFolding source](https://github.com/brackets-cont/brackets/blob/e41ba100d1955f027abf9223cc2cd38e3b7c9adb/src/extensions/default/CodeFolding/main.js). |
| Files, recovery, views | Working sets and horizontal/vertical splits; per-path encoding state is retained by document commands. Crash recovery guarantees were NV. [MainViewManager](https://github.com/brackets-cont/brackets/blob/e41ba100d1955f027abf9223cc2cd38e3b7c9adb/src/view/MainViewManager.js), [DocumentCommandHandlers](https://github.com/brackets-cont/brackets/blob/e41ba100d1955f027abf9223cc2cd38e3b7c9adb/src/document/DocumentCommandHandlers.js). |
| Customization and languages | Project/user preference scopes, themes/extensions; HTML/CSS/JS emphasis. Distinctive inline CSS editing finds matching rules for an HTML element. [PreferencesManager](https://github.com/brackets-cont/brackets/blob/e41ba100d1955f027abf9223cc2cd38e3b7c9adb/src/preferences/PreferencesManager.js), [inline feature docs](https://brackets.io/), [CSSInlineEditor](https://github.com/brackets-cont/brackets/blob/e41ba100d1955f027abf9223cc2cd38e3b7c9adb/src/editor/CSSInlineEditor.js). |
| Git and terminal | Site lists Git as a popular **extension**, rather than a core feature. A bundled interactive terminal was NV. [Official feature/extension page](https://brackets.io/). |

Relevant candidate is the range/depth/persisted folding behavior. Inline CSS Quick Edit, browser-connected HTML/CSS Live Preview, preprocessor workflows and JS refactoring are web IDE capabilities; exclude from an ordinary editor parity list unless separately approved. Do not replace the requested Brackets entry with Phoenix's new capabilities.

<a id="peer-05"></a>

### P05. Editra

| Area | Observed evidence |
|---|---|
| Version and status | GitHub release **0.7.20** was uploaded 2015-03-30 as a GoogleCode transfer. Inspected source snapshot [`b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8`](https://github.com/cjprecord/editra/commit/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8), 2019-11-15; `src/info.py` still declares 0.7.20. API says `archived=false`; no later code activity was observed, without inferring an official discontinued status. [Release](https://github.com/cjprecord/editra/releases/tag/0.7.20), [version file](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/src/info.py), [repo](https://github.com/cjprecord/editra). |
| Platforms | README records tested Linux, OS X and Windows XP/7 and requires Python 2-era wxPython. These are historical testing claims. [README](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/README). |
| Editing and search | Source includes code folding, bookmarks, autoindent/completion, text transforms and search; Vim command processor implements normal/insert/replace/visual state and motions. [Editor source](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/src/ed_stc.py), [search](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/src/ed_search.py), [Vim module](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/src/ed_vim.py). |
| Files, recovery, views | Editor has automatic backup settings/implementation; notebook, session and perspective modules are in the source. Print/page setup and print preview are concrete implemented functions. [ed_stc](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/src/ed_stc.py), [EdPrinter](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/src/ed_print.py). |
| Customization and languages | Package description claims syntax support for 70+ languages; style/theme/syntax modules and plugins exist. Treat number as that historic package's claim, not a modern language-coverage score. [PKG-INFO](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/PKG-INFO), [source tree](https://github.com/cjprecord/editra/tree/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/src). |
| Git and terminal | Repo contains terminal, FTP, project, codebrowser, macro and language-runner **plugin source directories**; that does not prove they ship enabled. Native core Git was NV. [Plugins tree](https://github.com/cjprecord/editra/tree/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/plugins). |

Candidate source pointers: print preview [`src/ed_print.py`](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/src/ed_print.py) (`Preview`, `Print`, `PageSetup`) and modal editing [`src/ed_vim.py`](https://github.com/cjprecord/editra/blob/b9d547d27a4a904f6dfa4b8066fa0cd23ba430e8/src/ed_vim.py). Standalone current feature documentation was unavailable, so source docstrings/package documentation are the official evidence; no functionality should be marked absent because of that. The GitHub release tag unexpectedly resolves to a 2008-dated commit; the examined 2019 snapshot is explicitly not presented as a release-exact implementation.

<a id="peer-06"></a>

### P06. FeatherPad

| Area | Observed evidence |
|---|---|
| Version and status | Latest observed release **V1.6.4**, published 2026-06-22; pinned source [`d66c33c2e139b6dd4bb38601346be2b48fa37ddc`](https://github.com/tsujan/FeatherPad/commit/d66c33c2e139b6dd4bb38601346be2b48fa37ddc). API not archived; 2026-10-27 default-branch commit provides later maintenance evidence. [Release](https://github.com/tsujan/FeatherPad/releases/tag/V1.6.4), [repo](https://github.com/tsujan/FeatherPad). |
| Platforms | README describes Linux as target and credits added Haiku, macOS and OS/2 support. Do not claim equal packaging/testing on those ports. [README](https://github.com/tsujan/FeatherPad/blob/V1.6.4/README.md). |
| Editing and search | Column selection, line jump, regex search and replacement capture groups, live match highlighting, separate search/replace widgets and per-tab/shared search history. [Shipped help](https://github.com/tsujan/FeatherPad/blob/d66c33c2e139b6dd4bb38601346be2b48fa37ddc/featherpad/data/help). |
| Files, recovery, views | Detachable/reattachable tabs, virtual-desktop awareness, filterable sorted side pane, named sessions/cursor positions, auto-saving, symlink actions, printing and zoom. [README](https://github.com/tsujan/FeatherPad/blob/V1.6.4/README.md), [help](https://github.com/tsujan/FeatherPad/blob/d66c33c2e139b6dd4bb38601346be2b48fa37ddc/featherpad/data/help). |
| Customization and languages | Custom shortcuts, syntax colors, MIME/name language detection with manual override, Hunspell checking, whitespace/line-end display. Current help explicitly says legacy encodings were removed with Qt 6; UTF-8 is default. [Help](https://github.com/tsujan/FeatherPad/blob/d66c33c2e139b6dd4bb38601346be2b48fa37ddc/featherpad/data/help). |
| Git and terminal | Can run executable script files via configured terminal or show captured output in a dialog. `-s` starts a separate process for use as a Git editor. These do not establish a bundled Git UI or terminal emulator. [Help](https://github.com/tsujan/FeatherPad/blob/d66c33c2e139b6dd4bb38601346be2b48fa37ddc/featherpad/data/help). |

Inspected [`printing.cpp`](https://github.com/tsujan/FeatherPad/blob/d66c33c2e139b6dd4bb38601346be2b48fa37ddc/featherpad/printing.cpp): dedicated QPrinter/QPainter flow, syntax formatting, page numbers/reverse order, PDF output behavior. Inspected [`encoding.cpp`](https://github.com/tsujan/FeatherPad/blob/d66c33c2e139b6dd4bb38601346be2b48fa37ddc/featherpad/encoding.cpp): UTF-8 validator, then ISO-8859-1 fallback; **not** evidence of comprehensive legacy encodings. Useful candidates: printing, spell check, explicit process lifetime for external editor use. Code folding was NV; do not infer absent.

<a id="peer-07"></a>

### P07. Geany

| Area | Observed evidence |
|---|---|
| Version and status | Latest observed release **2.1.0**, 2025-07-06, source [`cb5ff3cf0b63971010e60ea2feedd1a0181c1ace`](https://github.com/geany/geany/commit/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace). API not archived; default branch commit 2026-10-19. [Release](https://github.com/geany/geany/releases/tag/2.1.0), [repo](https://github.com/geany/geany). |
| Platforms | Linux/BSD/Unix distribution packages plus official Windows/macOS binaries. [README](https://github.com/geany/geany/blob/2.1.0/README.rst). |
| Editing and search | Rectangular editing, syntax folding, bookmarks, symbol lists/navigation, autocomplete/calltips, regex and find-in-files, text transformations. [Release manual](https://github.com/geany/geany/blob/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace/doc/geany.txt). |
| Files, recovery, views | Many document tabs, projects/session restoration; broad charset detection/conversion, manual Reload As versus change-on-save Set Encoding, BOM control; printing with syntax, line/page numbers and preview. SaveActions is a bundled plugin for additional save/backup behavior; do not equate it with unconditional core crash recovery. [Manual](https://github.com/geany/geany/blob/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace/doc/geany.txt), [release fixes](https://github.com/geany/geany/releases/tag/2.1.0). |
| Customization and languages | Configurable filetypes/extension patterns, shebang/XML and regex filetype extraction, snippets, keybindings, per-filetype build commands, plugin API. [Manual](https://github.com/geany/geany/blob/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace/doc/geany.txt), [filetypes.c](https://github.com/geany/geany/blob/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace/src/filetypes.c#L724). |
| Git and terminal | Optional embedded VTE loaded when `libvte.so` is available; shell options and send-selection/path-following workflows documented. It is not an all-platform terminal promise. Git UI is not established as core by these sources; third-party GeanyVC must be counted separately. [VTE manual section](https://github.com/geany/geany/blob/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace/doc/geany.txt#L456), [vte.c](https://github.com/geany/geany/blob/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace/src/vte.c). |

Inspected `src/encodings.c`, `src/filetypes.c` and `src/vte.c`: concrete encoding table (Windows-125x, ISO-8859, Shift-JIS, GB18030, etc.), detection/conversion, shebang-before-extension detection, and VTE command/path integration. Printing implementation is [`src/printing.c`](https://github.com/geany/geany/blob/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace/src/printing.c). The released manual's NUL-byte warning is stale relative to the 2.1 release fix, so do not repeat its historical truncation description as current behavior. Geany calls itself an IDE, but its compiler/build-run integration does not require adopting debugger/project-system scope in Orlo.

<a id="peer-08"></a>

### P08. gedit

| Area | Observed evidence |
|---|---|
| Version and status | Official GitLab tags and NEWS establish stable **51.0**, 2026-10-15, commit [`b7651f36c9337d20694ae6fe41f0fbe33a043f41`](https://gitlab.gnome.org/World/gedit/gedit/-/commit/b7651f36c9337d20694ae6fe41f0fbe33a043f41). This is newer than the website's 50.0 blog entry and much newer than download.gnome.org's old gedit index. [Tags API](https://gitlab.gnome.org/api/v4/projects/World%2Fgedit%2Fgedit/repository/tags?per_page=5), [release NEWS](https://gitlab.gnome.org/World/gedit/gedit/-/blob/b7651f36c9337d20694ae6fe41f0fbe33a043f41/NEWS), [repo](https://gitlab.gnome.org/World/gedit/gedit). |
| Platforms | Official installation page lists Linux, macOS (Homebrew), and Windows (Microsoft Store). [Install](https://gedit-text-editor.org/install.html). |
| Editing and search | Regex find/replace, autoindent, bracket highlighting, word completion, spell checking, bookmarks and text statistics. [Features](https://gedit-text-editor.org/features.html), [manual](https://gedit-text-editor.org/user-manuals/gedit/). |
| Files, recovery, views | Tab groups, remote-file editing, backups and print/print preview. Released source implements the print job and preview UI. [Tab groups](https://gedit-text-editor.org/user-manuals/gedit/gedit-tab-groups.html), [printing](https://gedit-text-editor.org/user-manuals/gedit/gedit-printing.html), [print source](https://gitlab.gnome.org/World/gedit/gedit/-/blob/b7651f36c9337d20694ae6fe41f0fbe33a043f41/gedit/gedit-print-job.c). |
| Customization and languages | Fonts/colors, syntax highlighting, dynamic C plugins; bundled Modelines plugin reads a subset of Emacs/Kate/Vim options. Its settings override preference-dialog defaults. [Features](https://gedit-text-editor.org/features.html), [modeline docs](https://gedit-text-editor.org/user-manuals/gedit/gedit-plugins-modelines.html). |
| Git and terminal | Core Git or VT terminal was NV. Importantly, **External Tools, Snippets, Python Console were removed in 48.1; Python plugins removed in 48.2**. Word Completion/Smart Spaces/Draw Spaces/Bookmarks moved into main gedit for 51.0; do not use old gedit-plugins lists as the current bundle. [NEWS](https://gitlab.gnome.org/World/gedit/gedit/-/blob/b7651f36c9337d20694ae6fe41f0fbe33a043f41/NEWS). |

Inspected released [`plugins/modelines/modeline-parser.c`](https://gitlab.gnome.org/World/gedit/gedit/-/blob/b7651f36c9337d20694ae6fe41f0fbe33a043f41/plugins/modelines/modeline-parser.c), [`plugins/bookmarks/gedit-bookmarks-plugin.c`](https://gitlab.gnome.org/World/gedit/gedit/-/blob/b7651f36c9337d20694ae6fe41f0fbe33a043f41/plugins/bookmarks/gedit-bookmarks-plugin.c) and `gedit-print-job.c`. Useful candidates: limited declarative modelines, persistent bookmark navigation, spell check, and printing. Browser tools had trouble fetching GNOME GitLab HTML; official API and the 51.0 source archive were successfully read directly.

<a id="peer-09"></a>

### P09. GNOME Text Editor

| Area | Observed evidence |
|---|---|
| Version and status | Latest official tar index/tag observed **51.0**, 2026-09-14, commit [`0eda8f023bc3a60256c1c1f064beced1805a3847`](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/commit/0eda8f023bc3a60256c1c1f064beced1805a3847). Release/translation and accessibility fixes are recorded in NEWS; no archival statement observed. [Release index](https://download.gnome.org/sources/gnome-text-editor/51/), [tags API](https://gitlab.gnome.org/api/v4/projects/GNOME%2Fgnome-text-editor/repository/tags?per_page=3), [repo](https://gitlab.gnome.org/GNOME/gnome-text-editor). |
| Platforms | GNOME/Linux app with Flatpak distribution and GTK4/Libadwaita/GtkSourceView dependencies. Other OS builds were not established. [App page](https://apps.gnome.org/TextEditor/), [released README](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/README.md). |
| Editing and search | Syntax highlighting, PCRE2 regex search/replace, inline spell checking and integrated Vim keybindings. [Released feature documentation](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/README.md). |
| Files, recovery, views | Deliberate automatic persistence of unsaved drafts and session state, including work never assigned a filename. Tabs/windows, fullscreen and document properties are reflected in released source/NEWS; printing is built in. [Draft-folder help](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/help/C/basics-draft-folder.page), [NEWS](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/NEWS), [printing source](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/src/editor-print-operation.c). |
| Customization and languages | `.editorconfig` and modeline providers; fonts/colors/dark mode, indentation and syntax choice. Encoding error UI lets the user choose another encoding and reload. This is a concrete UI path, not just “Unicode support.” [README](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/README.md), [encoding dialog](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/src/editor-encoding-dialog.c#L167). |
| Git and terminal | No bundled Git UI or terminal established by examined feature documentation/source. Treat as NV, not a proven product absence. [README feature boundary](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/README.md). |

Inspected `src/editor-print-operation.c`: syntax and line-number print compositor, header/footer/page numbers, ignoring spelling underlines. Inspected [`src/editorconfig/editor-page-editorconfig.c`](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/src/editorconfig/editor-page-editorconfig.c) and [`src/modelines/modeline-parser.c`](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/src/modelines/modeline-parser.c). Inspected [`src/editor-page-vim.c`](https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/src/editor-page-vim.c): GtkSource Vim input context wired to editor save/edit/quit. Features are scoped to simple editing, with no need to infer IDE functionality. Help pages are internally marked draft and sparse; release source is stronger evidence for details.

<a id="peer-10"></a>

### P10. Gobby

| Area | Observed evidence |
|---|---|
| Version and status | Latest observed release **0.6.0**, 2021-01-31, commit [`6ce877d00e29508acb4c93484feaba99dd9e2918`](https://github.com/gobby/gobby/commit/6ce877d00e29508acb4c93484feaba99dd9e2918). API not archived; default branch has a 2026-05-02 CI change. This demonstrates repository maintenance, not a recent feature release. [Release](https://github.com/gobby/gobby/releases/tag/v0.6.0), [repo](https://github.com/gobby/gobby). |
| Platforms | README claims Windows, Linux, macOS and other Unix-like platforms. GTK3 dependency and build/platform practicalities mean this is not a modern installer matrix. [Release README](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/README.md). |
| Editing and search | Multiple collaboratively edited documents, local-group undo, syntax highlighting, autoindent/tab width, find/replace and go-to-line handlers. [README](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/README.md), [edit commands](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/code/commands/edit-commands.cpp). |
| Files, recovery, views | Session document/server browser and document tabs; open/import, save/export operations, autosave commands with configurable interval. Saving converts from UTF-8 using the chosen encoding and EOL style. This is collaborative-session storage, not a generic remote-filesystem editor promise. [Autosave](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/code/commands/autosave-commands.cpp), [save operation](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/code/operations/operation-save.cpp). |
| Customization and languages | GtkSourceView syntax presentation; preferences/GSettings and GTK integration. HTML export preserves styled text and user information. [Release notes](https://github.com/gobby/gobby/releases/tag/v0.6.0), [HTML export source](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/code/operations/operation-export-html.cpp). |
| Git and terminal | Git UI and terminal emulator were NV in this collaborative editor's examined docs/source. The documented server, TLS, ACL, remote-cursor and chat functions are collaboration features, not Git/terminal substitutes. [README](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/README.md). |

Useful narrow references: encoding/EOL conversion and styled HTML export. Inspected [`operation-export-html.cpp`](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/code/operations/operation-export-html.cpp) (`dump_buffer`, `dump_tags_style`, `export_html`) and its [`task-export-html.cpp`](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/code/commands/file-tasks/task-export-html.cpp) UI. Exclude realtime collaborative editing, participant identity, chat, dedicated servers and access control from ordinary text-editor parity. The bundled manual identifies itself as Gobby 0.5/manual 2009 and contains obsolete authentication remarks; do not treat those as 0.6 limitations. [Manual source](https://github.com/gobby/gobby/blob/6ce877d00e29508acb4c93484feaba99dd9e2918/help/C/index.docbook).

<a id="peer-11"></a>

### P11. gPHPEdit (requested SourceForge project)

| Area | Observed evidence |
|---|---|
| Version and status | Requested SourceForge project's latest download is **0.9.91**, with folder timestamp 2006-07-05; project “Last Update” is 2013-03-22. These are different observations, not a 2013 release date. No official archive declaration observed. [Files](https://sourceforge.net/projects/gphpedit/files/gPHPEdit/), [project](https://sourceforge.net/projects/gphpedit/), [download](https://sourceforge.net/projects/gphpedit/files/gPHPEdit/0.9.91/gphpedit-0.9.91.tar.gz/download). |
| Platforms | Project describes a GNOME2 editor for PHP/HTML/CSS. The inspected 0.9.91 configure/build source uses GNOME/GTK/Unix facilities. No current macOS/Windows support evidence established. [Project description](https://sourceforge.net/projects/gphpedit/), [official source artifact](https://downloads.sourceforge.net/project/gphpedit/gPHPEdit/0.9.91/gphpedit-0.9.91.tar.gz). |
| Editing and search | Source `src/find_replace.c` implements literal/regex, match-case and whole-word search/replace. Scintilla-based language modules and folding preferences are present. [0.9.91 source archive](https://downloads.sourceforge.net/project/gphpedit/gPHPEdit/0.9.91/gphpedit-0.9.91.tar.gz), paths `src/find_replace.c`, `src/preferences.c`, `src/tab_php.c`. |
| Files, recovery, views | Tabbed document UI, file open/save/save-as, undo/redo and a saved-session preference are in `main_window.c`/`preferences.c`. A robust crash-recovery mechanism, printing and split views were NV. [Source archive](https://downloads.sourceforge.net/project/gphpedit/gPHPEdit/0.9.91/gphpedit-0.9.91.tar.gz). |
| Customization and languages | Source includes templates, plugin scanning, PHP calltips/syntax checking, class/function browser and CSS/SQL/C++/Python/Perl editor modules. These describe the historical source package, not modern language support. [Source archive](https://downloads.sourceforge.net/project/gphpedit/gPHPEdit/0.9.91/gphpedit-0.9.91.tar.gz), `src/plugin.c`, `src/templates.c`, `src/classbrowser.c`, `src/syntax_check.c`, `src/tab_*.c`. |
| Git and terminal | Bundled Git or interactive terminal was NV in this artifact. Shell-like plugin execution must not be assumed equivalent. [Source artifact](https://downloads.sourceforge.net/project/gphpedit/gPHPEdit/0.9.91/gphpedit-0.9.91.tar.gz). |

Evidence limitation: the official website redirects to SourceForge and its Code page did not expose a usable per-file source tree. Actual 0.9.91 source tarball was downloaded/inspected (SHA-256 `64b55916747171edf4cc1d90f038ca56b68d56d821e0b6c8f514266abb8e9b39`, also published by [SourceForge download page](https://sourceforge.net/projects/gphpedit/files/latest/download)). No invented Git commit or direct blob URLs. No unique proposed gap comes solely from this limited historical entry; folding is better evidenced by maintained editors. PHP syntax checking/class browsing are code-development features largely overlapping LSP or outside the plain-editor focus. Do not replace this exact requested project with an unrelated/newer fork.

<a id="peer-12"></a>

### P12. jEdit

| Area | Observed evidence |
|---|---|
| Version and status | Official download page labels **5.7.0** stable and 5.8pre1 continuous/development. Examined 5.7.0 source archive and corresponding commit [`388517e3b685d723db004d295d255a200c7784a3`](https://github.com/jEdit-editor/jEdit/commit/388517e3b685d723db004d295d255a200c7784a3), 2024-08-03. Official site links the new `jEdit-editor/jEdit` core repo; SourceForge SVN records removal of core tags after migration. Do not call old SVN activity the core release history. [Downloads](https://www.jedit.org/index.php?page=download), [requested SVN](https://sourceforge.net/p/jedit/svn/HEAD/tree/), [current official source](https://github.com/jEdit-editor/jEdit). |
| Platforms | Java application; stable 5.7 needs Java 11+. Platform installation links cover macOS, Windows, Unix/Linux, OS/2, VMS. Such historical platform instructions are not proof all current Java/OS combinations work. [Download requirements](https://www.jedit.org/index.php?page=download). |
| Editing and search | Rectangular/discontinuous selections; registers and kill ring; persistent markers; regex/project HyperSearch and script-computed replacements; indentation/explicit folding and narrowing. [Features](https://www.jedit.org/index.php?page=features), [folding](https://www.jedit.org/users-guide/folding.html), [markers](https://www.jedit.org/users-guide/markers.html). |
| Files, recovery, views | Two-stage save, autosave/crash recovery/backups, many split views/buffer sets, Java charset codecs with reload/detection, transparent gzip I/O, printing/preview. [Stable source manual](https://github.com/jEdit-editor/jEdit/blob/388517e3b685d723db004d295d255a200c7784a3/doc/users-guide/files.xml), [encoding docs](https://www.jedit.org/users-guide/encodings.html), [print docs](https://www.jedit.org/users-guide/printing.html). |
| Customization and languages | Global/mode/file properties, XML modes (official feature page lists 233 in 5.7), macros/BeanShell, custom keymaps/toolbars/menus, plugin manager, docked extension windows. Elastic tabstops are separately documented. [Features](https://www.jedit.org/index.php?page=features), [indentation](https://www.jedit.org/users-guide/indent.html), [stable guide source](https://github.com/jEdit-editor/jEdit/blob/388517e3b685d723db004d295d255a200c7784a3/doc/users-guide/source-edit.xml). |
| Git and terminal | Feature list explicitly describes IDE, external file protocols/archive access and richer tooling through plugins; do not count them all as core. A bundled Git GUI/terminal was not established. [Core/plugin distinction](https://www.jedit.org/index.php?page=features). |

Inspected stable archive files: `IndentFoldHandler.java`, `ExplicitFoldHandler.java`, `ElasticTabstopsTabExpander.java`, `BufferIORequest.java`, `BufferLoadRequest.java`, `Registers.java` and related user-guide source. Narrow candidates include persistent named registers/kill ring, fold/narrow commands, elastic tab alignment, transparent gzip, and detailed conversion errors. The online guide still titles itself 5.5; stable 5.7 source is the release-specific cross-check. Current GitHub core API is not archived; latest inspected default-branch commit is 2025-08-14, which is repository activity, not a release.

<a id="peer-13"></a>

### P13. Kate

| Area | Observed evidence |
|---|---|
| Version and status | Official app release list and tag establish **26.08.2**, 2026-10-08; source [`2b6465a886b9d3bfc6eab25065c14ffe025e6ac3`](https://invent.kde.org/utilities/kate/-/commit/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3). Recent releases provide maintenance evidence. Shared editor-engine code independently inspected at **KTextEditor 6.31.0**, commit `ea14509c132f2c016d55b38f7a1ec24cc6f9e7d2`; this is an explicitly separate component version, not a claim every Kate package uses that exact dependency. [App/release list](https://apps.kde.org/kate/), [repo](https://invent.kde.org/utilities/kate), [engine tags API](https://invent.kde.org/api/v4/projects/frameworks%2Fktexteditor/repository/tags?per_page=2). |
| Platforms | Officially Linux, BSD, Windows and macOS. Built-in terminal is an optional Konsole plugin and must not be assumed available in every platform package. [App page](https://apps.kde.org/kate/), [terminal manual](https://docs.kde.org/stable_kf6/en/kate/kate/kate-application-plugin-konsole.html). |
| Editing and search | Multiple cursors/rectangular selection, syntax folding, bookmarks/scroll marks, incremental/multiline/regex and multi-file search, Vi mode, smart spell checking, language-aware indentation. [Features](https://kate-editor.org/about-kate/), [KatePart manual](https://docs.kde.org/stable_kf6/en/kate/katepart/index.html). |
| Files, recovery, views | Split/tabbed views and sessions, backups/swap recovery, remote protocols, multiple encodings/conversion, line-ending detection, print support via shared engine, document-change handling and minimap. [Features](https://kate-editor.org/about-kate/), [app page](https://apps.kde.org/kate/), [component configuration](https://docs.kde.org/stable_kf6/en/kate/katepart/config-dialog.html). |
| Customization and languages | JavaScript indentation/command scripting, plugins, syntax definitions/themes, modelines/`.kateconfig` and optional EditorConfig; LSP, CTags and formatters. Bidirectional rendering is explicitly documented. [Features](https://kate-editor.org/about-kate/), [document-variable docs](https://docs.kde.org/stable_kf6/en/kate/katepart/config-variables.html), [app page](https://apps.kde.org/kate/). |
| Git and terminal | Bundled optional plugins provide status, line/hunk staging, commits/amend/sign-off, branches/history/blame, push/pull and stash. Konsole plugin supports selected text → terminal, automatic/manual cwd synchronization, run-current-file, `EDITOR='kate -b'`, and focus controls. [Git documentation](https://kate-editor.org/post/2023/2023-02-01-using-kate-git-support/), [terminal docs](https://docs.kde.org/stable_kf6/en/kate/kate/kate-application-plugin-konsole.html). |

Inspected release source [`addons/project/gitwidget.cpp`](https://invent.kde.org/utilities/kate/-/blob/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3/addons/project/gitwidget.cpp), [`stashdialog.cpp`](https://invent.kde.org/utilities/kate/-/blob/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3/addons/project/stashdialog.cpp) and [`addons/konsole/kateconsole.cpp`](https://invent.kde.org/utilities/kate/-/blob/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3/addons/konsole/kateconsole.cpp). Shared engine inspection verifies BiDi direction/cursor rendering and actual accessible text/edit interfaces, not merely translated UI labels. DAP/GDB, SQL/database tools, compiler explorer and language-specific project tooling are excluded IDE extensions. Handbook revision banners lag the application release; shipped source cross-checks narrower behaviors.

<a id="peer-14"></a>

### P14. Komodo Edit

| Area | Observed evidence |
|---|---|
| Version and status | Official GitHub release **12.0.1** was published 2022-12-15 and explicitly calls itself “The last build ActiveState made for Komodo Edit.” Release source [`9e6c3e53504a009b73f2ed776518a4e4d55b0931`](https://github.com/Komodo/KomodoEdit/commit/9e6c3e53504a009b73f2ed776518a4e4d55b0931) has a 2020-02-10 commit date, so distinguish publication from code age. Current issue template says repo closed and redirects reports; API archive flag is false. [Release](https://github.com/Komodo/KomodoEdit/releases/tag/12.0.1), [closure template](https://github.com/Komodo/KomodoEdit/blob/c0a0f05f0c5eb0b01d0dca65dc06cca420ab3ea9/.github/ISSUE_TEMPLATE/custom-template.md), [repo](https://github.com/Komodo/KomodoEdit). |
| Platforms | Historical Windows/macOS/Linux download/build documentation. No present-day compatibility inferred from old installers or Python2/Mozilla build requirements. [README](https://github.com/Komodo/KomodoEdit/blob/master/README.md). |
| Editing and search | The historical official manual source (9.2-era, not a verified 12.0.1 manual) documents syntax coloring, indentation guides, soft matching characters, clipboard-history picker, bookmarks, line/paragraph transforms, folding/Vi mode, regex/multiline and project search/replace with confirmation/undo. IDE-only markers in that manual must be respected. [Editor](https://github.com/Komodo/komodo-documentation/blob/7f2c35687dcca6247004d97eb76228cd0abc83f6/en-US/editor.html), [search](https://github.com/Komodo/komodo-documentation/blob/7f2c35687dcca6247004d97eb76228cd0abc83f6/en-US/search.html). |
| Files, recovery, views | Tabbed views, templates, file properties, print and encoding menus are documented/implemented. Full Edit-specific remote-file/recovery support was not independently separated from the shared IDE manual in this pass. [Files](https://github.com/Komodo/komodo-documentation/blob/7f2c35687dcca6247004d97eb76228cd0abc83f6/en-US/files.html), [editor source](https://github.com/Komodo/KomodoEdit/blob/9e6c3e53504a009b73f2ed776518a4e4d55b0931/src/chrome/komodo/content/editor.js), [encoding menu](https://github.com/Komodo/KomodoEdit/blob/9e6c3e53504a009b73f2ed776518a4e4d55b0931/src/chrome/komodo/content/library/encodingmenu.js). |
| Customization and languages | Multi-language editor; file associations, view-as-language, keybindings/toolbox/snippets/themes/addons are documented. Do not transfer the shared manual's full code-intelligence/debugger claims wholesale to Edit. [Editor manual](https://github.com/Komodo/komodo-documentation/blob/7f2c35687dcca6247004d97eb76228cd0abc83f6/en-US/editor.html), [source tree](https://github.com/Komodo/KomodoEdit/tree/9e6c3e53504a009b73f2ed776518a4e4d55b0931/src). |
| Git and terminal | The historical editor manual labels its Source Control menu **Komodo IDE only**. No 12.0.1 Edit-specific Git workflow was established; do not transfer IDE features to Edit. Command output/run facilities must not be described as a verified VT terminal without separate evidence. [Edition-labelled editor manual](https://github.com/Komodo/komodo-documentation/blob/7f2c35687dcca6247004d97eb76228cd0abc83f6/en-US/editor.html). |

Useful historical UX references are clipboard history, bookmarks, raw-character insertion and explicit language/encoding menus. Inspected `editor.js` and `library/encodingmenu.js`; no whole implementation port proposed. Exclude debugger/breakpoints, proprietary IDE source control, database/collaboration and IDE-only Rx Toolkit/formatters. The former vendor 12 manual URLs returned HTTP 404 during final link checks. The linked official documentation-source snapshot is from 2015 (9.2-era) and shared across editions, not release-matched 12.0.1 documentation. Treat manual-only capabilities as historical reference; inspected 12.0.1 source supports the narrower file/encoding/editor observations. Edition labels and source conditionals outrank broad screenshots. Repo closure is not the same as GitHub archival.

<a id="peer-15"></a>

### P15. KWrite

| Area | Observed evidence |
|---|---|
| Version and status | Official release list **26.08.2**, 2026-10-08, same Kate source tag/commit [`2b6465a886b9d3bfc6eab25065c14ffe025e6ac3`](https://invent.kde.org/utilities/kate/-/commit/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3). Maintained with that application source; no archive declaration observed. [App/release list](https://apps.kde.org/kwrite/), [requested repo](https://invent.kde.org/utilities/kate). |
| Platforms | Linux app plus official Windows/macOS platform links/nightly installers. Exact capabilities depend on packaging. [App page](https://apps.kde.org/kwrite/). |
| Editing and search | Shared KatePart/KTextEditor supplies syntax highlighting, word completion/autoindent, Vi input, regex editing/search, bookmarks, folding and print functionality. [KWrite app features](https://apps.kde.org/kwrite/), [handbook explicitly delegates engine docs](https://docs.kde.org/stable_kf6/en/kate/kwrite/index.html), [KatePart manual](https://docs.kde.org/stable_kf6/en/kate/katepart/index.html). |
| Files, recovery, views | Shares Kate view-manager code and editor-engine file/codec/backup features, but **named Kate session management is disabled in KWrite mode**. Do not repeat the 2021 handbook's “one document per window” as definitive for 26.08.2 without runtime checking. [Startup source](https://invent.kde.org/utilities/kate/-/blob/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3/apps/lib/kateapp.cpp#L534), [main window](https://invent.kde.org/utilities/kate/-/blob/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3/apps/lib/katemainwindow.cpp#L401). |
| Customization and languages | Engine syntax definitions, colors, indentation, modelines/`.kateconfig`/EditorConfig and BiDi behavior are shared. However the app page's generic “plugin support” cannot be read as all Kate plugins: 26.08.2 plugin manager explicitly returns with no plugins in KWrite mode. [Engine variables](https://docs.kde.org/stable_kf6/en/kate/katepart/config-variables.html), [plugin-manager source](https://invent.kde.org/utilities/kate/-/blob/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3/apps/lib/katepluginmanager.cpp#L50). |
| Git and terminal | Do **not** transfer Kate Project/Git/Konsole plugin capabilities to KWrite: application plugin loading is deliberately disabled, and KWrite creates only the view manager rather than Kate's output tool view. [Plugin manager](https://invent.kde.org/utilities/kate/-/blob/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3/apps/lib/katepluginmanager.cpp#L50), [main-window early return](https://invent.kde.org/utilities/kate/-/blob/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3/apps/lib/katemainwindow.cpp#L411). |

Relevant candidates are the common text-engine features (codec handling, folding, printing, modelines, Vi, BiDi/accessibility); see the gap catalog. Source inspection explicitly resolves the stale/overbroad KWrite docs. It would be misleading to score Kate and KWrite as identical products solely because both use KatePart.

<a id="peer-16"></a>

### P16. Leo

| Area | Observed evidence |
|---|---|
| Version and status | Latest observed release **6.8.10**, 2026-08-17; pinned source [`c82e40df3d4a69d26f2ed33c2644d5211492eb34`](https://github.com/leo-editor/leo-editor/commit/c82e40df3d4a69d26f2ed33c2644d5211492eb34). API not archived; 2026-10 default-branch activity provides later maintenance evidence. [Release](https://github.com/leo-editor/leo-editor/releases/tag/v6.8.10), [repo](https://github.com/leo-editor/leo-editor). |
| Platforms | Official installation guide covers Windows, Linux and macOS, using Python/Qt. Its older OS instructions are historical; no unsupported compatibility promises. [Installation](https://leo-editor.github.io/leo-editor/installing.html). |
| Editing and search | Outline/body editor, cloned nodes sharing underlying text, body/headline/outline searches, regex find/change and clone-find workflows, rectangle editing, minibuffer commands and Vim mode. This is not merely another tabbed file editor. [Technical introduction](https://leo-editor.github.io/leo-editor/preface.html), [commands/reference](https://leo-editor.github.io/leo-editor/commands.html), [find implementation](https://github.com/leo-editor/leo-editor/blob/c82e40df3d4a69d26f2ed33c2644d5211492eb34/leo/core/leoFind.py), [Vim docs](https://leo-editor.github.io/leo-editor/vimBindings.html). |
| Files, recovery, views | `.leo` outlines and `@file`/`@auto`/`@clean`/`@edit` external-file synchronization; clones and hoisting create alternate views of structure. Print/preview can target body, subtree or nodes. Generic crash-recovery guarantees were NV. [External-file directives](https://leo-editor.github.io/leo-editor/directives.html), [printing source](https://github.com/leo-editor/leo-editor/blob/c82e40df3d4a69d26f2ed33c2644d5211492eb34/leo/core/leoPrinting.py). |
| Customization and languages | Outline-based settings/themes, keybindings/input modes, Python scripts/plugins, per-node language/indent/encoding directives. [Customizing](https://leo-editor.github.io/leo-editor/customizing.html), [directives](https://leo-editor.github.io/leo-editor/directives.html). |
| Git and terminal | Release includes clone-diff-PR; source file-edit commands include Git/diff-oriented outline workflows. These do not establish a complete Git client or an integrated VT terminal. Python execution/console scripts are a separate capability. [6.8.10 release](https://github.com/leo-editor/leo-editor/releases/tag/v6.8.10), [editFileCommands](https://github.com/leo-editor/leo-editor/blob/c82e40df3d4a69d26f2ed33c2644d5211492eb34/leo/commands/editFileCommands.py). |

Inspected `leo/core/leoFind.py`, [`leoVim.py`](https://github.com/leo-editor/leo-editor/blob/c82e40df3d4a69d26f2ed33c2644d5211492eb34/leo/core/leoVim.py), `leoPrinting.py` and `leo/commands/editFileCommands.py`. Narrow transferable candidates: optional Vim input and print/preview scopes; possibly focus/narrowing workflows. Exclude full outline database/clones, literate programming, Python object scripting, Jupyter/server integrations and PR-to-outline workflows from ordinary editor parity. Changing Orlo's document model to match Leo would be a separate product design, not a missing checkbox.

<a id="peer-17"></a>

### P17. Light Table

**Reference, platforms and observed state:** [Light Table 0.8.1](https://github.com/LightTable/LightTable/releases/tag/0.8.1), published 2016-01-22; [commit `53a90bee4c2c6758e4b5dffdffbc32256a05e00d`](https://github.com/LightTable/LightTable/commit/53a90bee4c2c6758e4b5dffdffbc32256a05e00d); Release assets for Linux, macOS, Windows. Compatibility with current OS versions was not tested.; [Official repository](https://github.com/LightTable/LightTable) explicitly archived. Metadata `pushed_at` was 2022-06-17; archive state, rather than release age, supports this classification.

**Official feature documentation:** [user guide](https://docs.lighttable.com/), [version-pinned workflow][lt-workflow], [behavior/keymap configuration][lt-config], and [0.8.1 release notes](https://github.com/LightTable/LightTable/releases/tag/0.8.1).

- **Editing/search:** CodeMirror editing; core in-file find/replace, replace-all, and workspace search. The workflow documents language-aware completion, inline documentation and jump-to-definition for evaluated Clojure/ClojureScript vars. Those semantics depend on evaluation context and are not general LSP support. [Find implementation][lt-find] and [workflow][lt-workflow].
- **Files/recovery:** workspace tree supports files/folders, create/rename/delete, file watching and recent workspaces. [Workspace source][lt-workspace] serializes workspace data and reconstitutes the last workspace. [File source][lt-files] reads as UTF-8 and writes content; an atomic-save/recovery guarantee was **not** established.
- **Views/customization:** multiple tabsets, windows, command palette, themes, contextual keymaps, and editable behavior configuration. Plugins can extend substantial editor behavior; Vim/Emacs bindings are explicitly installed plugins, not native modes. [Guide](https://docs.lighttable.com/) and [configuration][lt-config].
- **Languages/Git/terminal:** Clojure, ClojureScript, JS/HTML/CSS and Python interactive connections appear in documentation; extra languages are plugins. Console shows evaluation stdout/stderr; it is not evidence of an embedded PTY terminal. Release note “Git friendly” refers to handling external file changes during rebases/branch switches, **not an integrated Git client**.

**Distinctive:** inline results and live browser-linked evaluation. These are out of scope as debugger/live-programming/browser-preview expansion. Most ordinary editor features overlap Orlo; no strong new in-scope gap is established by Light Table alone. Archived plugin metadata/service references should not be assumed operational today.

**Actually inspected source:** [find][lt-find], [files][lt-files], [workspace][lt-workspace]; immutable release commit throughout.

<a id="peer-18"></a>

### P18. Mousepad

**Reference, platforms and observed state:** [Mousepad 0.7.0](https://archive.xfce.org/src/apps/mousepad/0.7/), 2026-03-02; tag `mousepad-0.7.0`, commit `e9b6bebf91bfcf20a5c3a4d23a476471c98ee456`; Xfce/GTK desktop on Linux/Unix-like systems. GTK3/GtkSourceView dependencies; other native OS packages not established.; Official [Xfce documentation](https://docs.xfce.org/apps/mousepad/start) identifies 0.7.0 as latest release; inspected NEWS includes 2026 maintenance changes. No archive/deprecation notice observed.

**Official documentation:** [Xfce Mousepad guide](https://docs.xfce.org/apps/mousepad/start), including printing, keybindings, GtkSourceView dependencies and plugins; [pinned NEWS][mp-news].

- **Editing/search:** GtkSourceView syntax highlighting, indentation and wrapping settings; incremental find/replace, case/whole-word/regex options and match counts are documented in NEWS. Core deliberately targets fast text editing rather than an IDE. [NEWS][mp-news].
- **Files/recovery:** explicit open/save encoding choice, BOM/line ending handling, conversion errors, file monitoring, configurable save backups, session restore and unsaved-buffer autosave. [File implementation][mp-file] calls conversion APIs for decode/encode and stores separate autosave data. [Encoding table][mp-encoding] includes ISO8859 families, CP125x, Japanese/Chinese/Korean sets, UTF7/16/32, etc.
- **Views/customization:** tabs/windows, appearance and editor preferences, configurable keybindings, GtkSourceView schemes and CSS class. GSettings also exposes hidden preferences. Shortcuts editor and gspell are **bundled/build-dependent plugins**; spell checking requires optional gspell.
- **Languages/Git/terminal:** highlighting through GtkSourceView, not a demonstrated LSP/semantic service. No built-in Git client or PTY terminal established from official guide/source examined.

**Distinctive/in-scope candidates:** a small editor still supports native printing, configurable print settings retained across runs, and a broad character-set chooser. [Print implementation][mp-print] uses GTK print operations and a source print compositor. The [guide’s printing section](https://docs.xfce.org/apps/mousepad/start#printing) describes persistent print settings. These are concrete references for **legacy encoding** and **print** work, subject to Orlo absence verification.

**Limits:** remote locations rely on GIO/GVfs and system/backend availability (NEWS explicitly mentions GVfs for Flatpak); this is not an embedded SSH development environment.

<a id="peer-19"></a>

### P19. NEdit

**Reference, platforms and observed state:** [NEdit v5.8.0](https://sourceforge.net/p/nedit/git/ci/v5.8.0/tree/), April 2026 release; commit `5cc0adfcfd8f003f2c4f4af3fcc19e26d62ecfbe`; Motif/X11 Unix systems; README includes Linux, BSD/Unix, macOS through X11, and OpenVMS build instructions. Not a native Cocoa/Windows claim.; Official SourceForge repository has May 2026 changes. Master README says 5.8.1, May 2026, but the **examined release is 5.8.0**, not an assumed published 5.8.1. [Release notes][ne-release] describe 2026 tooling/rehosting work.

**Official documentation:** [version-pinned complete help][ne-help], [README][ne-readme], [release notes][ne-release], [official source tree](https://sourceforge.net/p/nedit/git/ci/5cc0adfcfd8f003f2c4f4af3fcc19e26d62ecfbe/tree/).

- **Editing/search:** rectangular selections, secondary selections, tab emulation, fill/reflow, bracket matching, configurable syntax highlighting and smart-indent macros; regex, incremental search, replacement and multi-window operation. The help documents these as editor operations, not extensions.
- **Files/recovery:** tabs and separate windows, client/server opening with `nc` in this release, recent files, periodic `~filename` crash backups and optional previous-version `.bck` files. The implementation in [source/file.c][ne-file] includes backup writes/removal and normal file save paths.
- **Views/customization:** split panes, X resources, menus, bindings, macro language, and shell command/filter menus. The `backlightCharTypes` preference highlights classes of characters such as control characters and spaces. [Help][ne-help].
- **Languages/Git/terminal:** pattern-based highlighting, ctags/calltips, smart indentation; no LSP or built-in Git workflow established. Shell commands can consume selection/buffer and send output back; external X terminals are distinct from embedded PTY terminal UI.

**Distinctive/in-scope candidates:** selective character backlighting; flexible selection/filtering. Compare this with Orlo’s existing whitespace display and multi-cursor tools (O02/O23). `PrintWindow`/`PrintString` in [file.c][ne-file] dispatch to a configurable external print command: this differs from GTK/Cocoa paginated print preview.

**Limits:** byte-oriented historical character-set constraints should not be presented as modern Unicode/BiDi support. This research did not validate broad Unicode behavior. Debugger IDE expansion is not a candidate. SourceForge master changed after the pinned release; 5.8.1 README text does not silently upgrade these findings.

<a id="peer-20"></a>

### P20. Notepad++

**Reference, platforms and observed state:** [Notepad++ v8.9.8.1](https://github.com/notepad-plus-plus/notepad-plus-plus/releases/tag/v8.9.8.1), 2026-09-24; [commit `dd40fe45598791e9aa38f3aeb200e10fb3d6338a`](https://github.com/notepad-plus-plus/notepad-plus-plus/commit/dd40fe45598791e9aa38f3aeb200e10fb3d6338a); Windows; release has x86, x64, ARM64 packages. Wine use is not native platform support.; Official repository not archived; release and ongoing repository changes observed in 2026.

**Official documentation:** first-party [manual repository](https://github.com/notepad-plus-plus/npp-usermanual); examined manual snapshot `41d96945ec8d11d4100356293f4f30e60c1418c0`. Relevant pages: [editing][npp-doc-edit], [search][npp-doc-search], [views][npp-doc-views], [encoding][npp-doc-encoding], [preferences][npp-doc-prefs], [session][npp-doc-session], [plugins][npp-doc-plugins].

- **Editing/search:** multi-edit and rectangular selections, column editor, macros, line operations/sorting, completion and function list; normal/extended/regex searches, selected/all-open/directory search and replacement, marking/bookmarks and results navigation.
- **Files/recovery:** broad codepage decoding/conversion, BOM/line-ending choices, auto-detection, remembered sessions and periodic snapshots/backup copies. [Encoding mapper][npp-encoding] establishes supported encoding identifiers; [I/O source][npp-io] handles snapshot restoration and fold state.
- **Views/customization:** two editor views with clone/move and synchronized scrolling, document map/list, project panels/folder workspace, visible whitespace/control characters, RTL/LTR view direction, themes, key remapping, UDLs and plugins.
- **Languages/Git/terminal:** bundled lexical highlighting/completion and configurable Run commands. Git clients, LSP, SFTP, a real terminal, and hex editing must be attributed to the specific external plugin when used; this inspection does not establish them as core features. “Run” alone is not PTY terminal support.

**Additional verified editing modes:** [manual editing page][npp-doc-edit] documents Insert-key/status-bar overwrite mode and virtual space beyond line ends. [NppNotification.cpp][npp-notify] toggles `SCI_SETOVERTYPE`; [Notepad_plus.cpp][npp-main] configures Scintilla virtual-space options for both views. Virtual space materializes padding only when editing and is distinct from scrolling past the last line.

**Strong candidates:** [Live File Monitoring][npp-doc-views] is a read-only `tail -f` mode; the inspected [command dispatch][npp-commands] checks unsaved changes and starts/stops monitoring, and [I/O source][npp-io] runs filesystem change monitoring. [Printing preferences][npp-doc-prefs] document syntax colors, margins, line numbers and header/footer variables; [Printer.cpp][npp-print] implements pagination. [RTL direction docs][npp-doc-views] explicitly document a DirectWrite limitation; do not generalize this into unrestricted modern BiDi support. Persistent session fold state is directly visible in [NppIO.cpp][npp-io].

**Limits:** manual is current documentation, so feature additions after8.9.8.1 should be checked against release/source before claiming parity. HexEditor is an extension, not a native binary hex view.

<a id="peer-21"></a>

### P21. Pe

**Reference, platforms and observed state:** [Pe v2.5.0](https://github.com/HaikuArchives/Pe/releases/tag/v2.5.0), 2025-05-14; [commit `c94b68ddfbefd591a4a890389a386d87b8d70af2`](https://github.com/HaikuArchives/Pe/commit/c94b68ddfbefd591a4a890389a386d87b8d70af2); Haiku. Historical documentation refers to BeOS.; Repository **not archived** despite the `HaikuArchives` organization name. Release lists substantive fixes; repository metadata showed pushes in July 2026.

**Official documentation:** shipped [menus][pe-menus], [state preferences][pe-state], [file preferences][pe-files-doc], [invisible character preferences][pe-invisibles], [worksheet preferences][pe-worksheet], [README][pe-readme]. These docs preserve historical BeOS wording and can lag current implementation.

- **Editing/search:** language syntax coloring and structural/function views, glossary insertion, language add-ons, regular-expression find/replace, multi-file searching, and a built-in visual differences workflow. [Menus][pe-menus] and source [CDocIO.cpp][pe-io].
- **Files/recovery:** groups/projects, file attributes for per-document state, tilde backup retention, native encoding conversion and line ending choices. [CDocIO.cpp][pe-io] converts between disk encoding and internal UTF-8 and reports conversion failures. Core FTP open/save is documented and implemented; it is **FTP**, not evidence of SFTP.
- **Views/customization:** per-document font, wrap, tab/invisible display, persistent state options, configurable shortcuts, glossary, extensions/add-ons. Haiku file attributes offer a useful model for reopening the document with its prior appearance and position; cross-platform storage would need a different design.
- **Languages/Git/terminal:** syntax modules and structural navigation plus Makefile/Jamfile integration. Worksheets execute commands with stdout/stderr handling. No full built-in Git client or PTY terminal established. Worksheet output redirection is not terminal emulation.

**Distinctive/in-scope candidates:** encoding conversion with actionable errors and invisible-character visualization; remote single-file workflows are historically core. Source [CFtpStream.cpp][pe-ftp] and [CDocIO.cpp][pe-io] establish implementation paths; FTP security/modern transport support must not be assumed.

**Limits/exclusions:** menu docs explicitly say printing was not implemented; no print path was found in inspected menus. Treat that as the examined version’s evidence, with historical-doc caution. BeIDE/project build support is not a reason to add IDE build systems to Orlo.

<a id="peer-22"></a>

### P22. Pluma

**Reference, platforms and observed state:** [Pluma v1.28.1](https://github.com/mate-desktop/pluma/releases/tag/v1.28.1), 2026-01-22; [commit `837036d73ed49a390cae6bbaac9a4f1e6d5b6de9`](https://github.com/mate-desktop/pluma/commit/837036d73ed49a390cae6bbaac9a4f1e6d5b6de9); MATE/GTK desktop, Linux/Unix-like systems; other native packages not established.; Official repository not archived; 2026 release and later source activity observed.

**Official documentation:** [pinned README][pl-readme], [complete shipped help][pl-help], [1.28.1 release](https://github.com/mate-desktop/pluma/releases/tag/v1.28.1), official [MATE project](https://mate-desktop.org/).

- **Editing/search:** tabbed multi-document editing; undo/redo, selection/editing operations, wrapping/indentation and GtkSourceView highlighting; find/replace. Completion, snippets, spell checking and tools must be labeled by their plugin, not all called core semantics.
- **Files/recovery:** remote URI editing through GIO/VFS, file revert, character encoding selection and conversion, autosave/backup preferences. [Encoding definitions][pl-encoding] and [document loader][pl-loader] establish charset handling and remote mounting/load paths; available protocols depend on system VFS backends.
- **Views/customization:** tabs, document/file-browser panels, themes, font/preferences and plugin engine. The inspected source ships plugins for spell checking, file browser, sort, time, tag list, document statistics, modelines, quick open, snippets, Python console and external tools. Enabled/default/build availability varies.
- **Languages/Git/terminal:** GtkSourceView highlighting and tag list are lexical/editor assistance. Python console and external tools are **bundled plugins**, not a general shell PTY. No core Git workflow or LSP established.

**Verified modelines:** the bundled modelines plugin reads a limited Emacs/Kate/Vim option subset for tab/indent width, spaces, wrapping and margin. [Shipped help][pl-help] specifies scan ranges and precedence over dialog preferences; [plugin README][pl-modelines-readme] and [modeline-parser.c][pl-modelines-parser] establish the implementation. This is a data-only per-document settings feature, not a reason to execute arbitrary content.

**Strong candidates:** help’s [printing chapter][pl-help] covers print preview, printer/PDF/PostScript output, syntax highlighting, headers, every-N-line numbering, wrapping and separate fonts. [pluma-print-job.c][pl-print] uses GtkSourcePrintCompositor and GTK print operations, with preview support. Remote text document open/save is a core text-editor workflow distinct from remote IDE execution.

**Limits:** some help text is historical; source is authoritative when commands/settings differ. No unsupported claim about every GIO protocol is made. Snippets/macros/external tools overlap existing Orlo facilities and are not gaps by themselves.

<a id="peer-23"></a>

### P23. SciTE

**Reference, platforms and observed state:** [SciTE 5.6.7](https://www.scintilla.org/SciTEDownload.html), 2026-09-28; inspected [official source archive](https://www.scintilla.org/scite567.tgz). Official [.hgtags](https://sourceforge.net/p/scintilla/scite/ci/default/tree/.hgtags) maps `rel-5-6-7` to `faccadbfdf959954854418a701be7a785b7227e2`.; Open-source SciTE for Windows 7+ and Linux/GTK; release has Windows x86/x64/ARM64 and Linux x64. Commercial macOS edition is separate, and its core was not inspected.; Official download/history page lists 5.6.7 and 2026 changes; no archive/deprecation notice observed.

**Official documentation:** [full manual](https://www.scintilla.org/SciTEDoc.html), [download and platforms](https://www.scintilla.org/SciTEDownload.html), [release history](https://www.scintilla.org/SciTEHistory.html). Manual and source docs from the downloaded5.6.7 archive were examined together.

- **Editing/search:** Scintilla rectangular/multiple selections, indentation, matching, lexical folding, regex find/replace, find-in-files and filter views. Core search has documented regex limits; do not assume all engines support multiline matching identically.
- **Files/recovery:** configurable multiple buffers/tabs (up to100), sessions, remembered recent files/positions, save-on-deactivate options, encoding choices via `code.page`/`character.set`, BOM/coding-cookie detection. A session primarily records file names/state, so this is not automatically a guarantee of unsaved-content crash recovery.
- **Views/customization:** split editor/output panes, bookmarks, whitespace/EOL and indentation guides, properties layered by user/directory/file, Lua scripting, abbreviations and key mappings. Session folds/bookmarks/read-only state can be persisted with explicit properties.
- **Languages/Git/terminal:** broad bundled Lexilla lexers, API-file completion/calltips, configurable build/run commands and clickable output errors. These are not equivalent to LSP or an integrated Git client. Program output pane is distinct from a general PTY terminal.

**Strong candidate:** manual’s **Sessions** and `session.folds`/`session.bookmarks` property entries precisely document preserving collapsed regions and bookmarks. Inspected `scite/src/SciTEBuffers.cxx` implements `SaveFolds`, `RestoreFolds`, and session serialization. This offers a narrowly scoped document-state improvement.

**Source/evidence limit:** official [5.6.7 source archive](https://www.scintilla.org/scite567.tgz), SHA256 `ad31f6e89aefe9c4e2c48f64c720f311e6a43369288e7b109d90b507ca623198`; inspected files `scite/src/SciTEBuffers.cxx`, `scite/doc/SciTEDoc.html`, `scite/README`. [Official source browser](https://sourceforge.net/p/scintilla/scite/ci/default/tree/) and [.hgtags](https://sourceforge.net/p/scintilla/scite/ci/default/tree/.hgtags) were reachable, but pinned file-view/raw requests failed in this environment. Use the Textadept immutable file below for a browser-readable fold-persistence implementation reference. **Intel HEX** in the language list is a textual firmware format lexer, not binary hex editing.

<a id="peer-24"></a>

### P24. Scribes

**Reference, platforms and observed state:** **Historical Scribes 0.3.3.2**, [official SourceForge archive](https://sourceforge.net/projects/scribes/files/OldFiles/scribes-0.3.3.2.tar.bz2/download), dated 2008-01-01. `configure.ac` confirms version; bundled README is stale and says 0.3.2.9.; GNOME/Linux/Unix with Python2/PyGTK2/GNOME libraries; source README gives legacy dependency versions. Present-day build compatibility untested.; Official [download page](https://scribes.sourceforge.net/download.html) explicitly calls 0.3 unsupported and directs users to **0.4-dev-build954**/Launchpad. That development source was unreachable in this environment. No claim that 0.4 is currently maintained or that the project is formally archived is made.

**Official documentation:** [features](https://scribes.sourceforge.net/features.html), [requirements](https://scribes.sourceforge.net/docs_requirements.html), [downloads/support warning](https://scribes.sourceforge.net/download.html), [historical release listing](https://sourceforge.net/projects/scribes/files/OldFiles/).

- **Editing/search:** official feature page describes automatic word completion/correction, pairs, indentation, snippets, text transforms and bookmarks. The inspected 0.3.3.2 archive contains bundled find/replace and incremental-search plugins. Do not silently assert every 0.4 website feature shipped unchanged in0.3.
- **Files/recovery:** inspected `SCRIBES/FileLoader.py` has URI/file metadata handling and writable remote schemes `ssh`, `sftp`, `smb`, `dav`, `davs`, `ftp` through **GNOME VFS**. `SCRIBES/NewFileSaver.py` contains timed save scheduling and a D-Bus save processor. This is historical automatic saving, not a verified current crash-recovery contract.
- **Views/customization:** Python plugin architecture, document switcher, templates/themes, bookmark browser. `plugins/RemoteDialog/Dialog.py` includes remote URI and encoding entry; print dialog/job/preview are **bundled plugins**, not separately installed evidence of current platform integration.
- **Languages/Git/terminal:** GtkSourceView syntax coloring (official page says30+ languages); automatic word completion, not demonstrated language-server semantics. No core Git client or PTY terminal established.

**Distinctive/in-scope lead:** small, low-chrome editing plus remote file opening and proactive text completion. For new Orlo work, prefer maintained references for remote handling; this legacy implementation depends on obsolete GNOME/Python infrastructure.

**Exact source evidence:** official [0.3.3.2 archive](https://sourceforge.net/projects/scribes/files/OldFiles/scribes-0.3.3.2.tar.bz2/download), SHA256 `4e995ecfaf6c730ea4fee64c9f9c56bd4303f44392c833f193fc70d2e97c5cbf`; inspected `configure.ac`, `NEWS`, `README`, `SCRIBES/FileLoader.py`, `SCRIBES/NewFileSaver.py`, `plugins/RemoteDialog/Dialog.py`. No independent immutable source-file viewer was established; filenames refer to this exact archive. 0.4-dev-build954 and official Bazaar branch requests failed/time out; no source claim is made for that unavailable version. Official site explicitly says 0.3 is unsupported; current 0.4 maintenance remains unknown.

<a id="peer-25"></a>

### P25. TEA

**Reference, platforms and observed state:** [TEA 63.3.1](https://github.com/psemiletov/tea-qt/releases/tag/63.3.1), 2025-03-31; [commit `9599b129100e68cff13a5946679e30819997264b`](https://github.com/psemiletov/tea-qt/commit/9599b129100e68cff13a5946679e30819997264b); Official README lists Linux, BSD, Mac, Windows, OS/2, Haiku; actual release asset inspected is a Qt5 Windows installer, with other platforms source/distribution dependent.; Official repository not archived; website lists 63.3.1 and repository metadata showed October 2026 pushes. Not evidence that all named platform builds are equally maintained.

**Official documentation:** [release/news page](https://tea.ourproject.org/), [pinned README/build options][tea-readme], [shipped English manual][tea-manual], [63.3.1 release](https://github.com/psemiletov/tea-qt/releases/tag/63.3.1).

- **Editing/search:** many built-in text/data/markup transforms, snippets, templates, word/list operations, paragraph processing, search/replace and search-in-files. The manual says search-in-files assumes UTF-8. Existing Orlo transforms should be checked before proposing TEA’s functions.
- **Files/recovery:** file-manager sidebar, recent file/encoding/caret state, saved sessions, temporary unsaved buffers on exit and configurable timed autosaving of designated files. Notes attach to a file through a `.notes` companion; a storage file can gather selected/clipboard text. [Manual][tea-manual] and [tea.cpp][tea-source].
- **Views/customization:** docking toolbars, tabs, palettes/themes, configurable shortcuts, custom highlighters, scripts, snippets and virtual on-screen keyboards. Spell checking can use Aspell/Hunspell/Nuspell and installed dictionaries; speech uses optional Speech Dispatcher and is explicitly experimental.
- **Languages/Git/terminal:** shipped XML-based syntax definitions, lexical code assistance and external scripts/tools. No integrated Git client, LSP or PTY terminal established. Reading PDF/DjVu/ebook text is import/extraction behavior, not a rich document editor.

**Useful candidates:** per-file notes and an opt-in collection/storage document; selected-text speech for accessibility; print support if compiled with `USE_PRINTER`. [tea.cpp][tea-source] has print-dialog and speech command implementations; [speech.cpp][tea-speech] is a separate actual source path.

**Important correction:** TEA 63 intentionally removed its old broad encoding support; official news says only UTF8/16 and popular Cyrillic encodings remain. [enc.cpp][tea-encoding] contains these conversion tables. Do not cite older “hundreds of encodings” descriptions. CMake defaults omit printing and several optional libraries; capabilities vary by build. AI/debugger/deploy features are not candidates.

<a id="peer-26"></a>

### P26. Textadept

**Reference, platforms and observed state:** [Textadept 13.0](https://github.com/orbitalquark/textadept/releases/tag/textadept_13.0), 2026-07-01; [commit `6139db0156b02a56bdf9890c881e0686ae63c3ea`](https://github.com/orbitalquark/textadept/commit/6139db0156b02a56bdf9890c881e0686ae63c3ea); Version-pinned manual: Windows 10+ x64/ARM, macOS 13+, Linux Qt5/GTK3 GUI and ncurses TUI; BSD supported through source builds.; Official repository not archived. Current online manual is **13.1 beta**, so release feature claims below use the **13.0 source manual**, not assumed beta behavior.

**Official documentation:** [13.0 pinned manual][ta-manual] and [13.0 release](https://github.com/orbitalquark/textadept/releases/tag/textadept_13.0). The live website currently describes 13.1 beta, so release-specific implementation links use13.0.

- **Editing/search:** multiple/rectangular selections, snippets, bookmarks, folding, transforms and shell filters; file/directory find/replace, regex/Lua patterns, result navigation and incremental search.
- **Files/recovery:** quick-open projects inferred from VCS root, sessions, recent files, changed-file handling and encoding detection/conversion. [file_io.lua][ta-io] defaults to UTF8, ASCII, CP1252 and UTF16; users can add iconv-supported encodings or reinterpret misdetected content. `nil` encoding denotes binary bytes, not a hex grid.
- **Views/customization:** arbitrarily split GUI/TUI views, buffers, Lua command entry, menus/keybindings/themes/lexers and scripting. [session.lua][ta-session] saves/reloads collapsed fold lines, bookmarks, views, sizes and encoding. GUI and terminal sessions have separate state files.
- **Languages/Git/terminal:**100+ lexers, basic word completion, commands/build/test output; language-aware completion/docs come from an **external LSP module**. Recognizing a Git directory as a project root is not integrated staging/commit/remote support. TUI is a real terminal-mode editor; output commands are not evidence of a nested interactive PTY emulator.

**Strong candidates:** explicit legacy encoding reinterpret/convert controls; persistent fold/bookmark/view state; the same lightweight editor available over terminal/SSH through its TUI. [Manual Encoding chapter][ta-manual] plus [file_io.lua][ta-io] are precise encoding evidence. [Manual sessions][ta-manual] plus [session.lua][ta-session] are direct state-persistence evidence.

**Limits:** the normal session file tracks file-backed buffer state, not a proven unsaved-buffer crash-rescue system. Binary byte loading is not a verified safe read-only hex inspector. LSP/debugging modules should not be counted as built-in capabilities or used to expand Orlo into an IDE.

<a id="peer-27"></a>

### P27. TextMate

**Reference, platforms and observed state:** [TextMate 2.0.23](https://github.com/textmate/textmate/releases/tag/v2.0.23), 2021-10-12; [commit `1e85a16eff2478fcae96ccd91137af6c19819752`](https://github.com/textmate/textmate/commit/1e85a16eff2478fcae96ccd91137af6c19819752); macOS 10.12+ per official repository description. Current OS compatibility untested.; Official repository not archived; metadata last push 2024-05-18. Release age alone does not establish abandonment.

**Official documentation:** [TextMate2 manual](https://macromates.com/textmate/manual/), [opening files/remote helper/binary imports](https://macromates.com/textmate/manual/opening-files), [version control](https://macromates.com/textmate/manual/version-control), [pinned changes][tm-changes]. The old `/manual/en/` manual is for1.5.1 and was not used as proof of2.x-only behavior.

- **Editing/search:** multiple carets/column selections, snippets with placeholders/transforms, macros, bundles, navigation/bookmarks, regex and project search/replace. Bundle commands can filter text or run tools.
- **Files/recovery:** folder projects/file browser, quick-open, `mate` command-line opening/waiting, `rmate` remote helper, automatic backup/resume, selectable encodings/newlines. [OakDocument.mm][tm-document] handles backup files and extended attributes; [encoding.mm][tm-encoding] supplies encoding logic.
- **Views/customization:** scopes, grammars, themes, `.tm_properties`, bundles, snippets and commands. **Persistent fold state** is stored as `com.macromates.folded` with a content CRC, alongside selection/bookmarks; [OakDocumentEditor.mm][tm-editor] passes serialized folds to/from layout. This is precise implementation evidence, not just a generic “has folding” claim.
- **Languages/Git/terminal:** syntax grammars and bundle-based semantic/tools integration. Core SCM watches badges/status/filtering; actual commit/diff/etc actions are **VCS bundles**. `mate`/`rmate`, shell commands and HTML output are distinct from a core PTY terminal. The examined docs do not establish built-in LSP.

**Strong candidates:** persistent fold/position metadata; remote editing through a small SSH-side helper; documented import/export commands for binary formats with textual representations. The last is conversion via bundle commands, **not an arbitrary binary hex editor**.

**Limits:** remote helper requires installation/connection setup. The source package does not prove every separately distributed bundle’s current behavior. No claim of formal abandonment follows from the old release. HTML output/live previews and IDE build bundles are excluded feature directions.

<a id="peer-28"></a>

### P28. Xed

**Reference, platforms and observed state:** **Xed 3.9.0**, official [package release tag `master.lmde7`](https://github.com/linuxmint/xed/releases/tag/master.lmde7), 2026-09-21; [commit `9be0bdcd6f17c119d41543440066da9287c12d0e`](https://github.com/linuxmint/xed/commit/9be0bdcd6f17c119d41543440066da9287c12d0e); Linux/GTK; inspected release assets are Debian/LMDE package artifacts, including amd64 changes file. Other native platforms not established.; Official Linux Mint repository not archived; published package artifacts and 3.9.0 changelog observed. The tag is a distribution tag, so immutable commit is the source pin.

**Official documentation:** [pinned README][xed-readme], [3.9.0 package/changelog][xed-changelog], shipped source [print job][xed-print], [tab/file loading][xed-tab], and [encoding selector][xed-encoding].

- **Editing/search:** standard GtkSourceView text editing, syntax highlighting, indentation/wrap, search/replace and file revert. Bundled plugins provide case conversion, join lines, sorting, date/time, modelines, word completion and more.
- **Files/recovery:** remote file editing through GIO/GtkSourceView, open/save encoding selection, conversion-error UI, optional backups and timed autosaves. [xed-tab.c][xed-tab] configures GtkSourceFileLoader/Saver, their encodings and backup flags. No stronger unsaved-document crash recovery guarantee is inferred from “autosave”.
- **Views/customization:** tabs, side panels/file browser, preferences, color schemes and plugin engine. Spell checker, statistics, file browser, trailing whitespace removal, tag list and word completion are **shipped plugins**; installation/build/default enablement may vary.
- **Languages/Git/terminal:** lexical syntax highlighting, word completion and tag insertion, not demonstrated built-in LSP. No integrated Git client or PTY terminal established. External programs/system VFS are not equivalent capabilities.

**Strong candidates:** printer and print-preview UI, syntax/line-number/font formatting through a source compositor; remote-document I/O and encoding selection. [xed-print-job.c][xed-print] is an actual inspected implementation and [README][xed-readme] explicitly documents both print/preview and remote editing.

**Distinctive:** a focused desktop editor with mature file/print integration and optional plugin conveniences. Most other capabilities overlap Orlo’s existing editing/transform/customization layers. Avoid creating separate backlog entries for every bundled text transform without checking existing commands.

<a id="peer-29"></a>

### P29. ZeroBrane Studio

**Reference, platforms and observed state:** **ZeroBrane Studio 2.01**, official [download](https://studio.zerobrane.com/download), 2023-09-26; tag `2.01`, [commit `483ef0dcb593f9599ff27715472425e39a0588e8`](https://github.com/pkulchenko/ZeroBraneStudio/commit/483ef0dcb593f9599ff27715472425e39a0588e8); Windows, macOS, Linux, per official docs/release.; Official repository not archived; source activity observed through September 2026. No GitHub “latest release” object exists; official site and tag/changelog establish 2.01.

**Official documentation:** [features](https://studio.zerobrane.com/features), [preferences](https://studio.zerobrane.com/doc-general-preferences), [tips](https://studio.zerobrane.com/doc-tips-and-tricks), [pinned changelog][zb-changelog], [README][zb-readme].

- **Editing/search:** Scintilla editing, selections, code folding, markers, configurable lexical highlighting110+ formats, word/API completion, fuzzy file/symbol navigation, scope-aware Lua information, regex/file find/replace and refreshable search-result pages.
- **Files/recovery:** project view with automatic filesystem refresh, directory mapping, per-project open tabs, file backups, auto-recovery and opt-in hot exit. [commands.lua][zb-commands] serializes/restores recovery data and uses a periodic session timer; docs distinguish `hotexit` from ordinary backup options.
- **Views/customization:** splittable/dockable tabs/panes, outline, output/console, Lua configuration, themes and a separate extension package repository. Output ANSI styling is configurable; Lua console is a REPL, not a general PTY shell terminal.
- **Languages/Git/terminal:** strong built-in Lua completion/analyzer/interpreter integrations; other syntax lexers are not full language IDE support. No built-in Git workflow established. Extensions must be counted separately. Debugger, watches, remote/on-device debugging and live coding are explicitly excluded from Orlo scope.

**In-scope candidates:** paginated source printing with preview, headers/footers/line numbers and syntax formatting. [print.lua][zb-print] implements wxLuaPrintout page drawing and print/preview flows. Reusable/refreshable search-result tabs may be a smaller editor workflow improvement if not already in Orlo.

**Limits:** [commands.lua][zb-commands] detects binary content, loads raw bytes and changes caret style; this does not establish a hex inspector. “Remote” on the homepage primarily refers to debugging/evaluation, not verified remote-file SFTP editing. No source claim from post2.01 master is merged into this release record.

<a id="peer-30"></a>

### P30. BBEdit

**Reference, platforms and observed state:** **BBEdit 16.0.3**, official [updates page](https://www.barebones.com/support/bbedit/updates.html), 2026-08-25; macOS 14+; universal Intel/Apple silicon; vendor says compatible with macOS 26.; Commercial maintained product; official 2026 maintenance release. **Proprietary core source unavailable**; source-code comparison cannot be performed.

**Official documentation:** [16.0.3 updates](https://www.barebones.com/support/bbedit/updates.html), [feature/license-mode comparison](https://www.barebones.com/products/bbedit/comparison.html), [text transforms/search](https://www.barebones.com/products/bbedit/benefitsexercise.html), [file/server workflows](https://www.barebones.com/products/bbedit/benefitscommand.html), [customization](https://www.barebones.com/products/bbedit/benefitswork.html), [language/navigation](https://www.barebones.com/products/bbedit/benefitsomnipotence.html), [SCM/worksheets](https://www.barebones.com/products/bbedit/benefitsintegrate.html).

- **Editing/search:** rectangle selection, multiple undo/clipboards, transformations, advanced PCRE, multi-file search/replace/extraction, simultaneous searches and text factories. Pattern Playgrounds and text factories are licensed features; simple “free BBEdit has everything” claims are false.
- **Files/recovery:** encoding choices, compressed/archive documents, remote FTP/SFTP open/save/browser, file/folder differences and projects. Official comparison lists save backups, discarded-untitled Document Rescue, resume of unsaved documents and periodic autosaving as free-mode functions.
- **Views/customization:** multiple documents/windows and split editing, color schemes, configurable menu keys, scripts, clippings, per-language settings, EditorConfig and codeless language modules. Projects, clippings, notes/scratchpad and some command conveniences are license dependent.
- **Languages/Git/terminal:** lexical navigation, ctags, built-in LSP integration with server-dependent completion/parameter help/diagnostics/formatting/definitions/references. Git/Subversion integration and editable shell worksheets are licensed; worksheets/CLI tools do not imply a conventional embedded PTY terminal.

**Useful documentary leads:** remote single-file editing, archive-aware document browsing, recovery of deliberately discarded untitled documents, and native spell checking. Existing Orlo worksheets/transforms/macros/LSP/Git should be compared before proposing equivalents.

**Source/evidence limitation:** BBEdit core is proprietary and unavailable. Vendor SDK examples, scripts, language modules, or third-party GitHub snippets are **not the core implementation** and were not represented as such. This record is documentation-only; stronger implementation-backed candidates should cite open-source peers. AI conversation worksheets, website deployment, HTML live preview and web-site validation are excluded from Orlo’s intended direction.

<a id="peer-31"></a>

### P31. Chime

**Reference, platforms and observed state:** **Chime development snapshot**, [commit `84b7afa4153acce27fc9b28ced457b7cb1acb477`](https://github.com/ChimeHQ/Chime/commit/84b7afa4153acce27fc9b28ced457b7cb1acb477), 2026-04-28. Stable website offers **2.2.5**, 2023-11-29; 3.0 alpha releases are distinct.; Stable2.2.5: macOS 12+. Inspected development README requires macOS 26.2+/Xcode26.4+ to build. iOS/visionOS are “in the works,” not established supported releases.; Repository not archived. README explicitly describes rewrite as mostly non-functional and stable2.x as **closed source**. Alpha205 release says editor component now functional “but not good”; do not combine stable marketing, alpha, and development source into one mature capability claim.

**Official documentation:** [stable2.2.5 download](https://www.chimehq.com/download), [official site](https://www.chimehq.com/), [pinned source README/project state][ch-readme], [3.x alpha releases](https://github.com/ChimeHQ/Chime/releases).

- **Editing/search:** development README lists multiple cursors, completions, document/project search, textual/symbol quick-open and structure highlighting. These are stated goals/current component claims under an explicit “mostly non-functional” application warning; they should not be ranked as proven release-wide parity. [TextDocument.swift][ch-document] and [SearchBar.swift][ch-search] were inspected to locate actual document/search integration.
- **Files/recovery:** file navigator and Cocoa document integration; [TextDocument.swift][ch-document] opts into in-place autosave and delegates file loading/writing to the text system. This does not prove mature recovery, arbitrary charset conversion or remote-file behavior.
- **Views/customization:** UI themes, EditorConfig, Quick Look preview extension and Shortcuts text-highlighting extension. Native ExtensionKit-based language extensions are bundled due to platform installation restrictions. OS-level preview integration is a concrete differentiator.
- **Languages/Git/terminal:** pattern/tree-sitter/LSP-based syntax/semantic layers and language extensions for multiple ecosystems. Stable2.x and rewrite may differ. No mature built-in Git workflow or PTY terminal is established from the examined snapshot; iOS/visionOS are plans.

**Distinctive/in-scope lead:** reuse syntax highlighting in macOS Quick Look and Shortcuts without opening a whole editor window. This is a platform integration idea, not a demonstrated Orlo gap. It may be lower priority than encoding/print/document-state work.

**Limits/exclusions:** stable2.2.5 is **closed source**, while public3.x is a rewrite; never treat open source as the stable2.2.5 implementation. Do not call the app abandoned or archive it based on date. Debug Adapter Protocol is a contribution wish, not established shipped functionality; debugger expansion is outside scope anyway.

<a id="peer-32"></a>

### P32. CodeEdit

**Reference, platforms and observed state:** [CodeEdit v0.3.6](https://github.com/CodeEditApp/CodeEdit/releases/tag/v0.3.6), 2025-08-26; [commit `05704cb7d2b19892ffcb9742b0ab3cc6116af165`](https://github.com/CodeEditApp/CodeEdit/commit/05704cb7d2b19892ffcb9742b0ab3cc6116af165); macOS 13+ deployment target in inspected Xcode project; Swift/AppKit/SwiftUI native app.; Repository not archived. Official website/README explicitly says in development, not recommended for production. Published0.3.6 features are separated from README aspirations.

**Official documentation:** [0.3.6 release notes](https://github.com/CodeEditApp/CodeEdit/releases/tag/v0.3.6), [pinned README/development warning][ce-readme], [Git DocC topic][ce-git-doc], and inspected setting/client/view sources below. Website still labels the app in development.

- **Editing/search:** native code editor, project find/replace and language services.0.3.6 explicitly adds external-file-change reload, editor restoration, global search shortcut and configurable invisible/warning characters. README’s broader debugger/review/extensions list is aspirational until independently established.
- **Files/recovery:** editor restoration preserves position across files/tabs/sessions; filesystem changes synchronize to open editors. [FileEncoding.swift][ce-encoding] models encoding choices. This alone does not validate all encoding round trips or a complete recovery contract.
- **Views/customization:** split editor/terminal areas, themes/preferences, key commands, invisible-character render replacements and a configurable Unicode-codepoint warning list. [InvisiblesSettingsView.swift][ce-invisibles] has separate toggles/replacements for spaces, tabs, LF, CR, paragraph/line separators; [settings model][ce-settings] has default warning codepoints and descriptions.
- **Languages/Git/terminal:** built-in LSP integration;0.3.6 language-server installer is explicitly experimental. Git client/source-control navigator implements local operations **and remote fetch/pull/push/clone/stash**. [GitClient+Stash.swift][ce-stash], [GitClient+Fetch.swift][ce-fetch] and [stash dialog][ce-stash-view] establish actual commands/UI. Integrated terminal uses a terminal emulator;0.3.6 task output also uses it.

**Strong candidates:** configurable warning glyphs for NBSP/zero-width/other troublesome Unicode; a focused Git remote/stash UI if deliberately expanding Orlo’s local-only policy; optional shell integration. [ShellIntegration.swift][ce-shell] documents bash/zsh injection of pre/post-execution hooks so the UI can receive process notifications without editing user rc files. Existing PTY availability alone does not establish such semantic shell integration in Orlo.

**Limits:** CodeEdit warning list does **not** by itself establish comprehensive Trojan Source/BiDi-control detection; default list inspected does not include every directional control.0.3.6 is a published development-stage release, not proof of production maturity. Debugger, advanced refactoring, account-hosted code review and AI are excluded directions.

<a id="peer-33"></a>

### P33. CotEditor

**Reference, platforms and observed state:** [CotEditor](https://github.com/coteditor/CotEditor); [7.1.3](https://github.com/coteditor/CotEditor/releases/tag/7.1.3), 2026-10-27; macOS 26+; Unarchived; pushed 2026-10-29; e188605143e61255699c4b1a49e993f38576641a.

Official entry: [coteditor.com](https://coteditor.com/). Examined 7.1.3 release plus [release README](https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/README.md). macOS-only native plain-text editor; the release explicitly requires macOS 26+. Source is Swift/Cocoa, Apache-2.0 for code with separate image-resource licensing.

- **Editing and search:** Multiple insertion points/rectangular selection, line/text tools, Unicode character inspection, outlines, and ICU regex find/replace. Release help documents folder-wide text/regex searches, case sensitivity, saved file scopes with AND/OR filename/path/extension conditions, inclusion controls and result limits. Folder searches skip symlinks and files over 1 GB. [shipped folder-search help](https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Resources/CotEditor.help/Contents/Resources/en.lproj/pgs/howto_find_in_folder.html).
- **Files and recovery:** Broad legacy encoding detection/conversion, explicit Convert versus Reinterpret, warnings for unrepresentable characters, automatic document backup/autosave. The implementation checks whether content can be encoded before saving; it does not silently guarantee arbitrary lossy conversion is safe. [encoding help](https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Resources/CotEditor.help/Contents/Resources/en.lproj/pgs/howto_change_encoding.html); [document loading/saving implementation](https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Sources/Document/Document.swift).
- **Views and customization:** File/folder browser, shared document split panes, vertical text mode useful for CJK, outline/character/statistics inspectors, native settings, keybindings, syntax definitions/themes, snippets and scripts using shell languages, AppleScript or JavaScript. Homepage establishes these broad capabilities; [release help corpus](https://github.com/coteditor/CotEditor/tree/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Resources/CotEditor.help/Contents/Resources/en.lproj/pgs) provides detailed topic coverage.
- **Languages, Git and terminal:** Syntax highlighting and outline extraction are core; no full LSP client, Git management workflow or embedded PTY terminal was established in this examination. “Scriptable” and a script console do not establish an integrated terminal.
- **Distinctive candidates:** Encoding incompatibility inspector with original/converted characters and editor navigation; configurable code printing/PDF with colors, line numbers, invisibles, page information and selection-only output. Pattern sorting supports CSV-column or regex-capture keys, numeric/language-aware comparison and keeping a header; Orlo already has extensive text/CSV tools, so that last item requires a detailed baseline check. [incompatible-character help](https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Resources/CotEditor.help/Contents/Resources/en.lproj/pgs/howto_inspect_usedchar.html); [inspected incompatibility inspector](https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Sources/Document%20Window/Inspector/IncompatibleCharactersView.swift); [pattern-sort help](https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Resources/CotEditor.help/Contents/Resources/en.lproj/pgs/howto_pattern_sort.html); [inspected printing implementation](https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Sources/Document/Print/PrintTextView.swift).

<a id="peer-34"></a>

### P34. Lapce

**Reference, platforms and observed state:** [Lapce](https://github.com/lapce/lapce); [v0.4.6](https://github.com/lapce/lapce/releases/tag/v0.4.6), 2026-01-21; Windows, macOS, Linux; Unarchived; pushed 2026-10-09; b012cef466741528f338879fa708355217fc40bd.

Official documentation: [Lapce docs](https://docs.lapce.dev/); [release README](https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/README.md). Examined v0.4.6, not the older homepage version. Native Rust/Floem GUI, Apache-2.0, Windows/macOS/Linux.

- **Editing, search, files and views:** Modeless and built-in Vim-style modal editing, multi-cursor editing, fuzzy file/symbol navigation, project explorer, in-file and global search, editor tabs/splits, inline/history diffs, and remote workspaces are core. Source includes separate find/global-search/editor-tab implementations; database persistence records workspace/doc/panel state and unsaved editor state is not inferred solely from “workspace” storage. [examined app modules](https://github.com/lapce/lapce/tree/b012cef466741528f338879fa708355217fc40bd/lapce-app/src); [inspected persistence implementation](https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/lapce-app/src/db.rs).
- **Customization and languages:** Settings/keymaps/themes plus WASI plugins. Tree-sitter highlighting and an LSP client are built in; language-server/plugin installation is a separate requirement. Completion, diagnostics, code actions and rename are established. [release README](https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/README.md); [modal editing documentation](https://docs.lapce.dev/get-started/modal-editing).
- **Git:** Core source-control panel collects selected file diffs and sends a commit with its message. Branch/tag state and diffs are present. This inspection does not establish a complete remote/stash/worktree UI or all partial-stage cases. [inspected commit panel model](https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/lapce-app/src/source_control.rs).
- **Terminal:** Built-in emulator, terminal tabs, shell profiles with command, arguments, workdir and environment, and per-platform default-profile selection. Official docs mark profiles experimental and incompletely integrated. [inspected profile implementation](https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/lapce-app/src/config/terminal.rs); [terminal profile documentation](https://docs.lapce.dev/get-started/terminal). The release changelog documents terminal tabs, regex-terminal-search fixes, and format-on-save settings; do not infer these from the homepage alone. [release changelog](https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/CHANGELOG.md).
- **Distinctive useful source evidence:** LSP snippet-format completions are parsed and inserted with editable placeholder ranges and next/previous navigation, rather than discarded. [completion application implementation](https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/lapce-app/src/editor.rs#L1910-L2055); [snippet parsing/tabstop implementation](https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/lapce-app/src/snippet.rs). The public prose docs describe completion generally; the exact InsertTextFormat behavior is source-level evidence.

<a id="peer-35"></a>

### P35. Lite XL

**Reference, platforms and observed state:** [Lite XL](https://github.com/lite-xl/lite-xl); [v2.1.8](https://github.com/lite-xl/lite-xl/releases/tag/v2.1.8), 2025-06-08; Windows, macOS, Linux; additional ports documented; Unarchived; pushed 2026-03-11; 0af4dae0e2748d6afe7bded41c13d96f627cb520.

Official guides: [introduction](https://lite-xl.com/user-guide/introduction/) and [IDE setup](https://lite-xl.com/user-guide/ide-setup/). Examined v2.1.8, a C/Lua editor forked from Lite, MIT. Windows/macOS/Linux release downloads; the homepage also documents FreeBSD/Android/web and other ports, which are not all equivalent release artifacts.

- **Bundled editing/search:** Multi-cursors, panes, command palette, fuzzy file opening, literal/regex find-replace, and case-sensitive/regex/fuzzy project search. The introduction explicitly says project-wide replacement is not supported in this core guide. Do not turn this limitation into a suggestion for Orlo, which already has that behavior.
- **Files/recovery/views:** Project tree supports file operations; multiple project directories, LF/CRLF toggle and indentation detection/settings. Bundled autoreload/workspace plugins restore view-tree structure, selection/scroll data and new unsaved document text. This is narrower than claiming proven crash-safe recovery for every dirty file. [inspected workspace persistence](https://github.com/lite-xl/lite-xl/blob/0af4dae0e2748d6afe7bded41c13d96f627cb520/data/plugins/workspace.lua); [bundled plugins](https://github.com/lite-xl/lite-xl/tree/0af4dae0e2748d6afe7bded41c13d96f627cb520/data/plugins).
- **Customization/languages:** Lua user configuration/plugins; bundled syntax plugins, themes, word completion, macros, reflow, tabularize, line wrapping and whitespace tools. [inspected project-search implementation](https://github.com/lite-xl/lite-xl/blob/0af4dae0e2748d6afe7bded41c13d96f627cb520/data/plugins/projectsearch.lua); [inspected paragraph reflow](https://github.com/lite-xl/lite-xl/blob/0af4dae0e2748d6afe7bded41c13d96f627cb520/data/plugins/reflow.lua); [release README](https://github.com/lite-xl/lite-xl/blob/0af4dae0e2748d6afe7bded41c13d96f627cb520/README.md).
- **Git/terminal/LSP:** Official IDE setup identifies lsp, lintplus, lsp_snippets, formatter, terminal, scm, gitblame and build as separately installed plugins, not mandatory core. The guide links language-server packages with downloadable tools. Debugger support is excluded from the main Orlo scope.
- **Distinction/limits:** The Lua architecture and small footprint are relevant design references, but claims about performance/size were not benchmarked. “Addons” and base release packages both exist; do not merge every optional plugin into a single built-in feature list.

<a id="peer-36"></a>

### P36. Nextpad++

**Reference, platforms and observed state:** [Nextpad++](https://github.com/nextpad-plus-plus/nextpad-plus-plus-macos); [v1.1.2](https://github.com/nextpad-plus-plus/nextpad-plus-plus-macos/releases/tag/v1.1.2), 2026-10-05; Examined Mac port: macOS 12+ Intel/Apple Silicon; Linux is separate port; Unarchived; pushed 2026-10-24; 4d391f0e8399a47bc96c62f45cb16980c6b6f329.

Official docs: [online help](https://nextpad.org/online-help/) and [plugins](https://nextpad.org/plugins/); source [release README](https://github.com/nextpad-plus-plus/nextpad-plus-plus-macos/blob/4d391f0e8399a47bc96c62f45cb16980c6b6f329/README.md). Examined Mac v1.1.2. Native Objective-C++/Scintilla GPL-3.0 port, separately maintained from upstream Windows Notepad++. Linux exists in a separate repository; Mac source evidence below does not prove Linux parity.

- **Editing/search/views:** Tabbed documents, cloned split views, code folding, document map, function list, bookmarks, column editor, clipboard history, character panel, macro recording, command palette and configurable themes. Literal/regex/incremental and file/project searches are documented; marking/bookmarking results is exposed in the help. The 1.1.2 release specifically fixes cloned-view coordination and fold-by-level.
- **Files/recovery/customization/languages:** Legacy Windows/ISO/CJK encodings plus UTF-8/UTF-16, LF/CRLF/CR conversion, session/backup configuration, native preferences/shortcut mapper, user-defined languages and 80+ syntax languages. An upstream “full parity” marketing claim is not used to assert every Windows feature. Actual encoding load/save and code-fold/print integration live in [inspected editor implementation](https://github.com/nextpad-plus-plus/nextpad-plus-plus-macos/blob/4d391f0e8399a47bc96c62f45cb16980c6b6f329/src/EditorView.mm).
- **Git:** Mac source includes Git panel/helper for repository detection/status, file diff, stage/unstage and committing. [inspected Git helper](https://github.com/nextpad-plus-plus/nextpad-plus-plus-macos/blob/4d391f0e8399a47bc96c62f45cb16980c6b6f329/src/GitHelper.mm). No remote/stash/worktree feature is established by that helper.
- **Terminal/extensions:** A native Plugin Admin loads ported plugins. The release mentions a separate npp terminal editor, which is not evidence of a PTY terminal panel inside the GUI. Integrated GUI terminal functionality was not established.
- **Distinctive candidate:** A dedicated column-number insertion dialog supports initial value, increment, repetition count, zero/space padding and decimal/octal/hex/binary formats across multiple selections. [inspected numeric column editor](https://github.com/nextpad-plus-plus/nextpad-plus-plus-macos/blob/4d391f0e8399a47bc96c62f45cb16980c6b6f329/src/ColumnEditorPanel.mm). Orlo already has column editing and extensive manipulation, so compare this precise sequence-generation UI rather than calling column editing absent. Printing and legacy encodings are independently documented by its online help.
- **Evidence limit:** Release README/macOS minima and website versions conflict as noted above; plugin port counts and projected completion dates in README are stale projections, not current facts.

<a id="peer-37"></a>

### P37. Pulsar

**Reference, platforms and observed state:** [Pulsar](https://github.com/pulsar-edit/pulsar); [v1.132.1](https://github.com/pulsar-edit/pulsar/releases/tag/v1.132.1), 2026-05-20; Windows, macOS Intel/Apple Silicon, Linux x86-64/ARM64; Unarchived; pushed 2026-10-27; 2853be5c34b0f5139fa35192aa765d6dbc2f9eb0.

Official docs: [Using Pulsar](https://docs.pulsar-edit.dev/using-pulsar/), [core packages](https://docs.pulsar-edit.dev/core-packages-and-features/), [IDE builtins](https://docs.pulsar-edit.dev/ide-features/builtin-packages/) and [IDE community packages](https://docs.pulsar-edit.dev/ide-features/community-packages/). Examined v1.132.1, MIT, Electron-based community continuation of Atom, Windows/macOS/Linux.

- **Editing/search/files/views:** Multiple cursors, snippets, semantic selection, code folding, fuzzy finder, project tree, multiple roots, tabs/panes/docks, in-buffer/project regex find-replace, Markdown/image/archive previews, spell checking and line-ending/encoding selectors are bundled packages. Theme, keymap, init script and CSS customization plus package manager are core.
- **Recovery/languages:** Autosave is a bundled optional package; session state storage exists. No crash-losslessness claim was tested. TextMate/Tree-sitter grammar and symbol providers are bundled; language intelligence is service-based and needs language-specific “brain” packages. Diagnostics UI, code actions, formatting, references and renaming can need additional community packages; homepage IDE language should not be treated as all shipped.
- **Git:** Bundled GitHub package supports line/hunk/file staging, discard, expanded commit editor, coauthors, amend/undo, branch checkout, fetch/pull/push and conflict resolution. [Official GitHub package guide](https://docs.pulsar-edit.dev/core-packages-and-features/github-package/). Release [package manifest](https://github.com/pulsar-edit/pulsar/blob/2853be5c34b0f5139fa35192aa765d6dbc2f9eb0/package.json) pins GitHub dependency v0.37.0-pretranspiled; resolved source commit 285ec77acd8ae6e50dda5289c8744e4049dc5773. Its actually read [Git command implementation](https://github.com/pulsar-edit/github/blob/v0.37.0-pretranspiled/lib/git-shell-out-strategy.js) builds amend/coauthor trailers and remote operations.
- **Terminal:** Bundled dependency terminal v0.3.2 is pinned by that manifest even though it lives outside the main packages directory. [Terminal guide](https://docs.pulsar-edit.dev/core-packages-and-features/terminal/) documents ordinary panes/docks, selectable shells, reflow, editor-like search and clickable URLs. Resolved terminal source commit 558dc4c23230cbf0db314cdd1d426df00cb73448; [inspected terminal search palette](https://github.com/pulsar-edit/terminal/blob/v0.3.2/lib/find-palette.js) calls next/previous search and displays match results.
- **Distinctive candidates:** Coauthor entry/expanded commit composition, terminal search, flexible terminal placement and encoding detection. The encoding selector directly applies chosen encodings and can detect with jschardet/iconv-lite. [inspected encoding picker](https://github.com/pulsar-edit/pulsar/blob/2853be5c34b0f5139fa35192aa765d6dbc2f9eb0/packages/encoding-selector/lib/encoding-list-view.js). These sources distinguish bundled dependencies from uninstalled registry packages.

<a id="peer-38"></a>

### P38. Sublime Text

**Reference, platforms and observed state:** [Sublime Text](https://www.sublimetext.com/download); Sublime Text 4, Build 4215, 2026-09-25; Windows 10+, macOS 10.13+ Intel/Apple Silicon, Linux x86-64/ARM64; Current stable download; 4213/4215 updates in September 2026; Proprietary core: no public implementation commit.

Examined [stable Build 4215](https://www.sublimetext.com/download), released 2026-09-25, Windows/macOS/Linux. Proprietary core. Official [documentation index](https://www.sublimetext.com/docs/) covers multiple/column selection, completions, settings, projects, packages, build systems and Git.

- **Editing/search/views:** Multiple selections, column editing, split/tab multi-select, Goto Anything and project symbols, syntax-aware selections/completion, snippets, macros, folding, minimap and distraction-free editing. Regex find/replace and Find in Files are supported; current stable release notes include fixes to find-in-files and join-line behavior.
- **Files/recovery/customization/languages:** Project/workspace pairs preserve open files and modifications; project JSON holds multiple folders, exclusions, settings and build definitions. [Projects docs](https://www.sublimetext.com/docs/projects.html). Settings/keymaps, syntax definitions, color schemes/themes and Python packages are core extensibility. Build 4213 upgraded the plugin host to Python 3.14; the homepage's old “Python 3.8” paragraph is stale. Native syntax-based completion is distinct from separately installed LSP packages.
- **Git:** [Official Git docs](https://www.sublimetext.com/docs/git_integration.html) establish status badges/branch information, incremental inline diffs and hunk reversion. Full history/blame/client actions are handed to the separate Sublime Merge application. Do not attribute all Sublime Merge features to the text editor.
- **Terminal:** Build 4213 added open_terminal for external terminal emulators. A built-in PTY emulator is not established; terminal packages are extension territory.
- **Distinctive reference/limits:** Highly integrated tab multi-selection and syntax completion/snippet UX are relevant; see [completion docs](https://www.sublimetext.com/docs/completions.html). Core implementation is unavailable. Public Python APIs or package sources cannot be cited as the implementation of proprietary editor behavior. No source-backed gap candidate is promoted from Sublime alone.

<a id="peer-39"></a>

### P39. Nova

**Reference, platforms and observed state:** [Nova](https://nova.app/releases/); Nova 14.1, 2026-08-06; macOS 15.6+ on current download page; Current paid product and maintained release history; Proprietary core: no public implementation commit.

Examined [Nova 14.1](https://nova.app/releases/), released 2026-08-06, current [download page](https://nova.app/) requires macOS 15.6+. Proprietary native Mac application.

- **Editing/search/views:** Multiple cursors/columns, folding, minimap, code-structure headers, color wells with Display P3 support, contextual completions, snippets (“Clips”), symbols and Vim mode. [Editor guide](https://help.nova.app/editor/overview/). Local project find/replace has filters, wildcards, saved scopes and results; remote projects can only search open files. [Find sidebar](https://help.nova.app/projects/find-sidebar/).
- **Files/recovery/customization:** Broad encodings include Mac Roman, Latin-1/Windows Latin-1 and Shift JIS; conversion is distinguished from reinterpretation. Text actions provide natural/length sorting, shuffling, numeric-sensitive sorting, unique/reverse/join, output to a tab/clipboard and control-character cleanup. [Text actions](https://help.nova.app/editor/text-actions/). Configurable syntax-colored printing/PDF includes fonts, line numbers, headers/footer and frames. [Printing](https://help.nova.app/editor/printing/). [Files settings](https://help.nova.app/settings/files/) documents file behavior; no additional crash-safety guarantees are inferred.
- **Languages/customization:** Themes, keybindings, project settings, JavaScript extensions and tasks. Nova 14 adds direct language-server detection/install/update/pinning and per-language editor preferences; C/C++/C#/Rust/Swift support is now built in. Extension API access is not source for the core.
- **Git:** Branch/reference/submodule management, changes/history graph, commits/amend/signing through Git configuration and fetch/pull/push. The [Git sidebar guide](https://help.nova.app/git/sidebar/) explicitly excludes remote projects and repository roots outside the project root. Do not claim unrestricted nested-repo support.
- **Terminal:** [Local/SSH terminal tabs](https://help.nova.app/terminals/overview/) can be moved/split like editor tabs; shells and display are configurable. No exact terminal mouse/hyperlink protocol implementation is available.
- **Distinctive candidates/limits:** Printing, legacy encodings, commit signing/amend and integrated LSP management are documented reference behaviors only. The proprietary core has no public file/commit to inspect; not equivalent to Nova's extension API. FTP/publishing/debuggers remain out of the main scope.

<a id="peer-40"></a>

### P40. Rune

**Reference, platforms and observed state:** [Rune](https://github.com/unstablebuild/rune); [v1.3.2](https://github.com/unstablebuild/rune/releases/tag/v1.3.2), 2026-10-26; macOS/Linux x86-64 and ARM64 release artifacts; Unarchived; pushed 2026-10-29; 71537d80dc6b163ddbed0bd69e502a75bdc661d0.

Official [docs](https://docs.rune.build/) and [release README](https://github.com/unstablebuild/rune/blob/71537d80dc6b163ddbed0bd69e502a75bdc661d0/README.md). Examined v1.3.2, GPL-3.0-or-later Go/native graphics application, macOS/Linux releases for x86-64 and ARM64. GUI, TUI and headless operation are documented/referenced by release notes; no Windows release artifact was established.

- **Editing/search/files/views:** Modeless standard, Vim, Emacs and Helix-oriented modes, multiple selections/cursors, line/text operations, macros, code folds and separately hidden lines. Structural Tree-sitter search, semantic symbols and fuzzy file/text navigation. Nine workspace slots, arbitrary tabs/windows/terminals and configurable multiplexed layouts. [release standard-editor manual](https://github.com/unstablebuild/rune/blob/71537d80dc6b163ddbed0bd69e502a75bdc661d0/docs/docs/learn/standard-editor.md); [release search manual](https://github.com/unstablebuild/rune/blob/71537d80dc6b163ddbed0bd69e502a75bdc661d0/docs/docs/learn/search.md).
- **Recovery/customization:** YAML configuration, key maps, aliases, themes and installable packages. Built-in autosave waits after edits, reports errors and avoids forcing writes over disk changes or unwritable files; that is not a tested crash-recovery guarantee. [inspected autosaver](https://github.com/unstablebuild/rune/blob/71537d80dc6b163ddbed0bd69e502a75bdc661d0/internal/ide/autosave.go). Release 1.3.2 adds trust decisions before applying repository config and tightens exact-command approvals; useful context if comparing project commands.
- **Languages:** Integrated LSP commands include completions, hover, diagnostics, definition/references, signature help and file/selection formatting. [release language-intelligence guide](https://github.com/unstablebuild/rune/blob/71537d80dc6b163ddbed0bd69e502a75bdc661d0/docs/docs/languages/intelligence.md). Some language tooling is installed as official packages; no blanket claim that every language server binary ships in the base executable.
- **Git:** Built-in diff visualization and worktree commands. [inspected Git diff popup](https://github.com/unstablebuild/rune/blob/71537d80dc6b163ddbed0bd69e502a75bdc661d0/internal/ide/gitshow.go). Worktrees are intentionally implemented through documented aliases combining Git commands and workspace opening, not a separately established all-purpose commit panel. [released worktree command guide](https://github.com/unstablebuild/rune/blob/71537d80dc6b163ddbed0bd69e502a75bdc661d0/docs/docs/learn/worktrees.md).
- **Terminal:** First-class terminal multiplexer, scrollback searching, normal editor-like find box, terminal layouts and persistent remote/headless sessions. [inspected scrollback-search controller](https://github.com/unstablebuild/rune/blob/71537d80dc6b163ddbed0bd69e502a75bdc661d0/internal/term/vte/search.go).
- **Distinctive candidate:** A terminal search maintains a reader separate from the live streaming view, so inspecting history need not consume the active shell view. Worktree-as-workspace commands are also a concrete scope-appropriate reference. E2E peer networking, agents and debugger features are appendix-level.

<a id="peer-41"></a>

### P41. Fresh

**Reference, platforms and observed state:** [Fresh](https://github.com/sinelaw/fresh); [v0.5.4](https://github.com/sinelaw/fresh/releases/tag/v0.5.4), 2026-10-20; Linux, macOS, Windows, FreeBSD; GUI artifacts also provided; Unarchived; pushed 2026-10-29; d1e266fd119b6860ca47a3ea13eab6c8f138d9cb.

Official [docs](https://getfresh.dev/docs), [release README](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/README.md). Examined v0.5.4, GPL-3.0-or-later, Linux/macOS/Windows/FreeBSD. Primarily a terminal editor; this release also distributes Linux/macOS/Windows GUI artifacts. Their availability is observed, not a claim of equal maturity.

- **Editing/search/files/views:** Multiple cursors/block selection, macros, familiar nonmodal keys with optional Vim mode, tree-sitter indentation, folds, fuzzy file opening, split panes, project tree and live ripgrep/project regex replacement including unsaved buffers. Large-file handling is documented; performance was not benchmarked.
- **Recovery/customization/languages:** Hot Exit preserves dirty/unnamed buffers across restart, with tabs/layout restoration separately configurable; daemon mode allows detach/reattach and live process persistence. [released persistence guide](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/docs/features/session-persistence.md). Themes, searchable settings/keybinding conflict UI, package manager and sandboxed TypeScript plugins. Core LSP supports signature help, range formatting and multiple servers per language with feature routing; installed language servers remain prerequisites. [released LSP guide](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/docs/features/lsp.md); [inspected signature/range-format requests](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor/src/app/lsp_requests.rs).
- **Encodings:** UTF-8/16, Latin-1, Windows-1250/51/52, GBK/GB18030, Shift JIS and EUC-KR with detection, manual reload and status-bar conversion. [released encoding guide](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/docs/features/encoding.md); [inspected encoding implementation](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor-core/src/model/encoding.rs).
- **Git:** Hunk-stage/unstage/discard review buffer, branch/commit/stash-range comparisons, Git log/blame/grep and worktree-oriented workspace dock. [released Git guide](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/docs/features/git.md). Do not infer remote mutation controls from the word Git; this task did not inspect Fresh push/pull implementation.
- **Terminal:** Multiple tabs/splits/dock terminals, scrollback as read-only searchable text, per-split live versus history mode, configurable shell command/args, launch/restart behavior, mouse reporting with Shift bypass, clickable file:line:column paths and OSC7 cwd tracking. [released terminal guide](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/docs/features/terminal.md); [inspected mouse forwarding](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor/src/app/terminal_mouse.rs); [inspected OSC7/activity parsing](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor/src/services/terminal/term.rs); [inspected path detector](https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor/src/services/terminal/path_link.rs).
- **Distinctive candidates/limits:** Strongest direct references for Orlo terminal mouse/search/cwd/profile gaps, legacy encodings, signature help and range formatting. All those docs were fetched from the release commit, not merely the moving website. Fresh's advertised Latin-1 implementation maps through WHATWG Windows-1252; assess exact codec semantics before adopting a similar label.

<a id="peer-42"></a>

### P42. Lite

**Reference, platforms and observed state:** [Lite](https://github.com/rxi/lite); [v1.11](https://github.com/rxi/lite/releases/tag/v1.11), 2020-07-05; Published v1.11 Windows/Linux downloads; Unarchived; pushed 2023-09-20; 91c43dc01ef3b33f526f200054337aacb7e3a1a1.

Official source [release README](https://github.com/rxi/lite/blob/91c43dc01ef3b33f526f200054337aacb7e3a1a1/README.md). Examined rxi/lite v1.11, released 2020-07-05, MIT, Windows/Linux downloads. Repository is not archived but metadata shows no push since 2023-09-20. Do not call it actively maintained or merge Lite XL's features into it.

- **Editing/search/views:** Core command palette and ordinary buffer editing, undo/redo, project/file opening and split-pane structures. Shipped plugins include word completion, autoreload, macro recording, project search, tree view, quote wrapping, reflow, tabularize and whitespace cleanup. [actual release plugins](https://github.com/rxi/lite/tree/91c43dc01ef3b33f526f200054337aacb7e3a1a1/data/plugins); [release core modules](https://github.com/rxi/lite/tree/91c43dc01ef3b33f526f200054337aacb7e3a1a1/data/core).
- **Search semantics:** Literal searches plus Lua-pattern find/replace and project pattern/fuzzy search are actually implemented; Lua patterns are not a full PCRE-compatible regex engine. [inspected find/replace commands](https://github.com/rxi/lite/blob/91c43dc01ef3b33f526f200054337aacb7e3a1a1/data/core/commands/findreplace.lua); [inspected project search](https://github.com/rxi/lite/blob/91c43dc01ef3b33f526f200054337aacb7e3a1a1/data/plugins/projectsearch.lua).
- **Files/recovery/customization/languages:** Lua user init and plugins, additional theme repository, bundled basic language grammars. Autoreload is established; durable dirty-buffer recovery was not established. Multi-cursor behavior, arbitrary encodings, LSP, Git management and an embedded terminal are not asserted from this older release.
- **Git/terminal:** Not established as bundled features in examined v1.11; external plugin ecosystem availability does not establish them.
- **Distinctive candidate:** Small delimiter-alignment command widens each column to its maximum string length and replaces selected lines. [inspected delimiter tabularization](https://github.com/rxi/lite/blob/91c43dc01ef3b33f526f200054337aacb7e3a1a1/data/plugins/tabularize.lua). This is a precise text-manipulation reference, not a claim Orlo's CSV/column tools lack alignment. It also has limitations: its byte-length and simple delimiter splitting are not a CSV parser.

<a id="peer-43"></a>

### P43. COSMIC Text Editor

**Reference, platforms and observed state:** [COSMIC Text Editor](https://github.com/pop-os/cosmic-edit); Development snapshot, 2026-10-29; COSMIC/Linux desktop; Unarchived; pushed 2026-10-29; ec8375c65090ee9547885a2d9bf94a6630f5fbd9.

Official [COSMIC apps page](https://system76.com/cosmic/apps) and [examined snapshot README](https://github.com/pop-os/cosmic-edit/blob/ec8375c65090ee9547885a2d9bf94a6630f5fbd9/README.md). No GitHub latest published Release was resolved; examined development snapshot ec8375c65090ee9547885a2d9bf94a6630f5fbd9 (2026-10-29). GPL-3.0 licensing is in the repository; native Rust COSMIC/Linux editor.

- **Editing/search/files/views:** Tabbed editor, project folder tree, open directories, find/replace and project search, configurable indentation/wrapping/themes and built-in Vim keybindings. The System76 page explicitly documents current status tracking and Revert All Changes. Source menus establish save/save-as, find, replace, project search and Git management; print/encoding entries are still TODO comments in this snapshot. [inspected menu implementation](https://github.com/pop-os/cosmic-edit/blob/ec8375c65090ee9547885a2d9bf94a6630f5fbd9/src/menu.rs).
- **Recovery/customization/languages:** Configuration is a native COSMIC settings model; syntax highlighting uses SyntaxEditor/ViEditor and syntax themes. Snapshot tab loading supports reload and undo-recorded disk replacement. [inspected tab/save/search implementation](https://github.com/pop-os/cosmic-edit/blob/ec8375c65090ee9547885a2d9bf94a6630f5fbd9/src/tab.rs). No durable crash-recovery, plugin ecosystem or LSP integration is established here.
- **Git:** Snapshot implementation provides status, parsed staged/unstaged diffs and file stage/unstage. [inspected Git implementation](https://github.com/pop-os/cosmic-edit/blob/ec8375c65090ee9547885a2d9bf94a6630f5fbd9/src/git.rs). Commit/remotes/stash/worktree management are not established by these methods or the official apps description.
- **Terminal:** COSMIC Terminal is a separate desktop application on that same page. Its splits, GPU rendering, Unicode shaping and profiles must not be attributed to COSMIC Text Editor.
- **Distinctive/limits:** Useful small native editor with basic Git tracking; no stronger Orlo gap candidate emerges here. The snapshot README still says “incomplete pre-alpha” while the public apps page describes delivered desktop capabilities. Preserve this mismatch rather than extrapolating shipping status from either alone.

<a id="peer-44"></a>

### P44. Focus

**Reference, platforms and observed state:** [Focus](https://github.com/focus-editor/focus); [0.3.8](https://github.com/focus-editor/focus/releases/tag/0.3.8), 2025-02-27; 64-bit Windows, macOS, Linux; Unarchived; pushed 2026-10-29; ffc089a35342c4ac08118603a2b781023c62ac77.

Official [focus-editor.dev](https://focus-editor.dev/) and [release README](https://github.com/focus-editor/focus/blob/ffc089a35342c4ac08118603a2b781023c62ac77/README.md). Examined stable 0.3.8 (2025-02-27), GPL-3.0, native Jai editor for 64-bit Windows/macOS/Linux. Main repository pushed in October 2026 and website advertises frequent nightly builds; these are separate from the examined release.

- **Editing/search/views:** Multi-cursor editing, split/duplicated editors, line movement/duplication, case changes, indentation detection/conversion, regex local/project search with include/exclude wildcards, color-preview popup and command palette. [inspected action list](https://github.com/focus-editor/focus/blob/ffc089a35342c4ac08118603a2b781023c62ac77/src/actions.jai); [shipped configuration and keymap](https://github.com/focus-editor/focus/blob/ffc089a35342c4ac08118603a2b781023c62ac77/config/default.focus-config). The release notes give precise new cursor/word-motion and search filtering behaviors.
- **Files/recovery:** External-change handling warns before overwriting and detects file deletion/recreation. Session code writes per-buffer and workspace-state data, but much older loading code is commented; do not infer a particular crash-recovery guarantee without testing active recovery paths. [inspected session persistence code](https://github.com/focus-editor/focus/blob/ffc089a35342c4ac08118603a2b781023c62ac77/src/session.jai).
- **Customization/languages:** Text global/project configurations, user themes/fonts, per-language/per-file overrides. Syntax highlighting and compiler-error regex navigation are lightweight core features. Build/run commands have working directories and output panels. The product explicitly targets users who do not require heavy language support.
- **Git/terminal:** Build-output panel and external run commands are established; a Git-management UI or embedded PTY terminal is not established. LSP support is not established in this stable release.
- **Distinctive/limits:** Color-preview UI and file-specific config may be useful references, but no source-backed new Orlo gap is promoted. Stable README says CRLF is converted to LF, multi-codepoint Unicode is unsupported, and files have a 2 GB hard limit; avoid projecting later main-branch improvements onto 0.3.8.

<a id="peer-45"></a>

### P45. Flow Control

**Reference, platforms and observed state:** [Flow Control](https://github.com/neurocyte/flow); [v0.7.2](https://github.com/neurocyte/flow/releases/tag/v0.7.2), 2026-02-14; Linux, FreeBSD, macOS, Windows, Android/Termux; Unarchived; pushed 2026-10-29; af7c97acb9579f76a237a52e3104b1639dd24fbd.

Official [flow-control.dev](https://flow-control.dev/) and [manual](https://flow-control.dev/docs/); [release README](https://github.com/neurocyte/flow/blob/af7c97acb9579f76a237a52e3104b1639dd24fbd/README.md). Examined v0.7.2 (2026-02-14), MIT, Zig TUI. Linux/FreeBSD/macOS/Windows/Android under Termux; requires an appropriate truecolor terminal and UTF-8 locale.

- **Editing/search/files/views:** Multiple cursors, clipboard history, tabs/palettes/mouse UI, in-memory undo, large-file rope/piece-table buffer and tree-sitter highlighting. Manual documents local and project search, modal/nonmodal operation, command palettes, drag selections and split/tab navigation. [release user manual](https://github.com/neurocyte/flow/blob/af7c97acb9579f76a237a52e3104b1639dd24fbd/help.md).
- **Customization/languages:** Flow/Emacs/Vim/Helix/custom key maps, themes and VS Code theme compatibility project, file-type configurations and integrated LSP transport. State includes per-project recent files and configuration; do not equate in-memory “infinite undo” with persistent undo.
- **Git:** Source implements Git workspace discovery/status, current branch, object content and blame. [inspected Git integration](https://github.com/neurocyte/flow/blob/af7c97acb9579f76a237a52e3104b1639dd24fbd/src/git.zig). Site advertises diff/blame; a full commit/stage/stash/remote-management UI was not established.
- **Terminal:** Flow is a TUI running inside a terminal. The examined README calls multi-terminal sessions a future feature; this is not an integrated emulator. A shell module does not override that evidence.
- **Release/doc mismatch:** The examined release README lists LSP completion, persistent undo/redo and file watcher integration under development. Current homepage advertises inline auto-completion. The source file completion.zig is a generic request-completion helper, not evidence of LSP text completion. Do not promote the current homepage's text into a proven released feature without inspecting editor response/application code.
- **Distinctive candidates/limits:** Clipboard history and flexible keymap modes are precise references; main Orlo gaps were not established from this editor. Performance numbers and very-large-file claims were not benchmarked.

<a id="peer-46"></a>

### P46. Athas

**Reference, platforms and observed state:** [Athas](https://github.com/athasdev/athas); [v0.17.0](https://github.com/athasdev/athas/releases/tag/v0.17.0), 2026-10-15; macOS, Windows, Linux; browser workbench also in release; Unarchived; pushed 2026-10-28; 67a11409221a7d2f6cf2ebdf5071d55a0a210c4e.

Official [docs](https://athas.dev/docs), [release README](https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/README.md). Examined v0.17.0 (2026-10-15), AGPL-3.0, Rust/Tauri/React, macOS/Windows/Linux; source and release notes also include a browser workbench.

- **Editing/search/files/views:** Multi-cursors, line tools, folding by level, Markdown preview of unsaved buffers, minimap, Quick Open, multi-root workspaces and nested editor/terminal splits. [Editing guide](https://athas.dev/docs/editing). Project/in-file regex search and multi-file replacement have scope/include/exclude filters. [Search guide](https://athas.dev/docs/search). Auto Save/reopen-closed-tabs and workspace layout restoration are documented; local-history modules exist but this review does not infer all retention guarantees.
- **Customization/languages:** Rebindable keymaps/presets, Vim mode, themes and language integrations. LSP, outline/diagnostics, file/selection formatting and format/lint-on-save rely on the relevant tooling/integration. [Formatting guide](https://athas.dev/docs/formatting).
- **Git:** Changes/staging/commits, branches/merge, history/diffs/blame, stash and worktrees. [Git guide](https://athas.dev/docs/git-integration). Worktree management opens current/new windows, creates branches/checkouts, distinguishes safe removal from force removal and prunes stale registrations. [Worktree guide](https://athas.dev/docs/git-worktrees). [inspected worktree commands](https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/crates/version-control/src/git/worktree.rs); [inspected workspace/window opening](https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/src/features/git/services/git-worktree-open.ts); [inspected stash operations](https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/crates/version-control/src/git/stash.rs).
- **Terminal:** [Terminal guide](https://athas.dev/docs/terminal) documents tabs/splits, shell selection/custom profiles, search, URL/file/OSC8 hyperlinks, working-directory tracking, OSC133 prompt/exit integration, command navigation/output copying, completion notifications and layout persistence.
- **Actually inspected terminal sources:** [addon loading/link providers](https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/src/features/terminal/hooks/use-terminal-addons.ts) loads SearchAddon/WebLinksAddon and supports file links; [search UI](https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/src/features/terminal/components/terminal-search.tsx) has regex/case/whole-word switches; [OSC133 frontend](https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/src/features/terminal/lib/terminal-shell-integration.ts) keeps prompt/command markers; [shell integration launcher](https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/crates/terminal/src/shell_integration.rs) installs launch-time scripts for supported shells.
- **Distinctive candidates/limits:** Best source-backed reference for terminal regex search, multiple shell/split UI, command-boundary navigation, worktree lifecycle and stash management. AI/collaboration/database/cloud features are outside the main comparison. Website docs are moving; the exact implementation links above are the examined release commit.

<a id="peer-47"></a>

### P47. TTT Editor

**Reference, platforms and observed state:** [TTT Editor](https://github.com/eugenioenko/ttt); [v1.9.0](https://github.com/eugenioenko/ttt/releases/tag/v1.9.0), 2026-10-23; macOS, Linux, Windows, FreeBSD, Android; release includes ARM64 additions; Unarchived; pushed 2026-10-29; cc1a9267a265ff249a98302f702ab2f4146620f0.

Official [tttedit.dev](https://tttedit.dev/) and [release README](https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/README.md). Examined v1.9.0 (2026-10-23), Go TUI, single executable. Git and ripgrep are external prerequisites for source control and workspace search. Linux/macOS/Windows/FreeBSD/Android support is documented; release adds Android/Windows ARM64 artifacts.

- **Editing/search/files/views:** Multi-cursor editing, line/text transforms, folding, clipboard operations, bracket matching, tabbed file explorer, quick open, multi-root workspaces and saved .ttt layouts. TextMate grammars for 124 languages are embedded according to the release editor guide. External changes are watched. [released editor guide](https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/docs-web/src/content/docs/guides/editor.md). Ripgrep workspace search supports glob filtering; source-controlled review is separate from editing.
- **Recovery/customization:** Themes/live picker, JSON settings/keymaps, plugins with permissions and two-way external RPC. v1.9.0 tightens plugin permissions and fixes Save As/replacement/undo behavior. No lossless crash-recovery claim was established.
- **Languages:** Built-in LSP client plus 25 predefined server configurations; users install external language-server executables. Signature help, diagnostics, definition/reference/rename, file/selection formatting and format-on-save/code-actions-on-save are documented. [released LSP guide](https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/docs-web/src/content/docs/guides/lsp.md). [inspected LSP client](https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/internal/lsp/client.go) actually requests signatureHelp and rangeFormatting, but advertises SnippetSupport=false: do not list LSP snippet completions as supported merely because it has completion.
- **Git:** Per-repo multi-root staged/unstaged lists, partial/full-file diffs, stage/discard/commit, new-branch creation, pull/push/sync, history/blame and GitHub PR review. [released Git guide](https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/docs-web/src/content/docs/guides/git.md); [inspected Git commands](https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/internal/git/git.go). Push checks upstream and can set origin/upstream for a new branch; it is not just a toolbar label.
- **Terminal:** Multiple terminal tabs, truecolor VT emulator, dock/fullscreen behavior, configurable shell and scrollback retention. [released terminal guide](https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/docs-web/src/content/docs/guides/terminal.md); [inspected emulator wrapper](https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/internal/terminal/terminal.go). The shell/reporting/search capabilities of the outer host terminal must not be attributed to this inner terminal.
- **Distinctive candidates/limits:** Signature help, format selection/on-save, terminal shell/scrollback controls, and explicit per-repository remote controls. The examined documentation does not establish terminal scrollback search or mouse forwarding; use Fresh/Athas/Rhun evidence for those.

<a id="peer-48"></a>

### P48. Rhun

**Reference, platforms and observed state:** [Rhun](https://github.com/vshvedov/rhun); [v0.19.0](https://github.com/vshvedov/rhun/releases/tag/v0.19.0), 2026-10-27; Linux x86-64, macOS Apple Silicon, Windows x64; Unarchived; pushed 2026-10-27; 3afaafdb19401f9076d79cb290b4d1a711e89e8c.

Official [rhun.app](https://rhun.app/), [release README](https://github.com/vshvedov/rhun/blob/3afaafdb19401f9076d79cb290b4d1a711e89e8c/README.md) and [large release manual](https://github.com/vshvedov/rhun/blob/3afaafdb19401f9076d79cb290b4d1a711e89e8c/docs/guide.md). Examined v0.19.0 (2026-10-27), MIT, native assembly implementation. Linux x86-64, macOS Apple Silicon, Windows x64; source is translated to AArch64 for Mac and uses native platform integration.

- **Editing/search/files/views:** Tabs, project explorer, fuzzy file/project text search, literal/regex find/replace, multiple cursors, line tools, wrapping, folding, syntax highlighting, optional Vim mode and configurable themes/settings. Sessions restore project state and tabs; do not infer all crash-recovery guarantees from reopening a session.
- **Customization/languages:** Native extension processes with declared permissions; first-party language server/formatter/EditorConfig/snippet/task integrations. LSP-backed hover, completion, signature help, definitions/references, rename, formatting, symbols, inlay hints, semantic tokens and code lens are extension-provided integration surfaces in the manual, not proof every tool ships in the base executable.
- **Git:** Core file/gutter status, staging/commit, pull/push, diffs and all-branch history graph. Git worktree registrations are also read for agent-history discovery; that is not evidence of a worktree create/delete UI. AI commit message generation is outside scope.
- **Terminal:** Built-in emulator with truecolor, alternate screen, scrollback, mouse reporting, multiple terminals and clickable web/file links; Windows uses ConPTY. Site and release manual establish these features. [inspected terminal input/view implementation](https://github.com/vshvedov/rhun/blob/3afaafdb19401f9076d79cb290b4d1a711e89e8c/src/app/termview.s#L3451-L3716) checks requested mouse mode and Shift, then routes presses, motion and release through term_mouse.
- **Distinctive candidates/limits:** Requested terminal mouse reporting with Shift-selection escape hatch is confirmed in released implementation, useful corroboration for Fresh. Native extension support is broader than Orlo's light LSP scope; debugger/notebook/agent extensions should not expand the main comparison.


## 8. Verification performed for this report

- Confirmed the worktree starts at the stated full Orlo commit and inspected all ten crate owners, exposed command families and the relevant test sources.
- Ran the existing focused regression test below because the README and implementation disagreed about inline selection folding. **Result: 1 passed, 0 failed, 963 filtered out; exit0.** It verifies the Fold command, rendered placeholder, opening by click, unchanged document text and the remaining multiline behavior.
- Did not run the full application suite, all real language servers, manual native smoke tests or peer binaries. Previous audits' test totals are not presented as current results.
- Validated coverage of all 48 requested names, 37 Orlo inventory groups, unique gap IDs, resolved Markdown references, internal dossier anchors and existing local-source targets. Final whitespace/diff checks apply to this documentation change only.
- Independently checked 211 commit-pinned GitHub source/document targets successfully; a separate review also checked release-tag file links. The non-GitHub URL pass reached 155 of 172 original targets; the three 404 Komodo pages were replaced and 14 SourceForge 403 access restrictions are recorded above. HTTP success verifies reachability, not feature correctness.
- A separate read-only review checked scope, source-level gap claims, source targets and local links. Its pull-diagnostics wording correction is incorporated.
- References are research pointers. No third-party implementation, dependency, product code, API or schema was added.

```sh
cargo test -p orlo-ui --locked fold_on_a_selection_inside_a_line_hides_just_that_text
```

## Reference targets

[lt-workflow]: https://github.com/LightTable/LightTable/blob/53a90bee4c2c6758e4b5dffdffbc32256a05e00d/doc/workflow.md
[lt-config]: https://github.com/LightTable/LightTable/blob/53a90bee4c2c6758e4b5dffdffbc32256a05e00d/doc/behavior-and-keymap-configuration.md
[lt-find]: https://github.com/LightTable/LightTable/blob/53a90bee4c2c6758e4b5dffdffbc32256a05e00d/src/lt/objs/find.cljs
[lt-files]: https://github.com/LightTable/LightTable/blob/53a90bee4c2c6758e4b5dffdffbc32256a05e00d/src/lt/objs/files.cljs
[lt-workspace]: https://github.com/LightTable/LightTable/blob/53a90bee4c2c6758e4b5dffdffbc32256a05e00d/src/lt/objs/workspace.cljs
[mp-news]: https://gitlab.xfce.org/apps/mousepad/-/blob/e9b6bebf91bfcf20a5c3a4d23a476471c98ee456/NEWS
[mp-file]: https://gitlab.xfce.org/apps/mousepad/-/blob/e9b6bebf91bfcf20a5c3a4d23a476471c98ee456/mousepad/mousepad-file.c
[mp-encoding]: https://gitlab.xfce.org/apps/mousepad/-/blob/e9b6bebf91bfcf20a5c3a4d23a476471c98ee456/mousepad/mousepad-encoding.c
[mp-print]: https://gitlab.xfce.org/apps/mousepad/-/blob/e9b6bebf91bfcf20a5c3a4d23a476471c98ee456/mousepad/mousepad-print.c
[ne-release]: https://sourceforge.net/p/nedit/git/ci/5cc0adfcfd8f003f2c4f4af3fcc19e26d62ecfbe/tree/ReleaseNotes
[ne-help]: https://sourceforge.net/p/nedit/git/ci/5cc0adfcfd8f003f2c4f4af3fcc19e26d62ecfbe/tree/doc/help.etx
[ne-readme]: https://sourceforge.net/p/nedit/git/ci/5cc0adfcfd8f003f2c4f4af3fcc19e26d62ecfbe/tree/README
[ne-file]: https://sourceforge.net/p/nedit/git/ci/5cc0adfcfd8f003f2c4f4af3fcc19e26d62ecfbe/tree/source/file.c
[npp-doc-edit]: https://github.com/notepad-plus-plus/npp-usermanual/blob/41d96945ec8d11d4100356293f4f30e60c1418c0/content/docs/editing.md
[npp-doc-search]: https://github.com/notepad-plus-plus/npp-usermanual/blob/41d96945ec8d11d4100356293f4f30e60c1418c0/content/docs/searching.md
[npp-doc-views]: https://github.com/notepad-plus-plus/npp-usermanual/blob/41d96945ec8d11d4100356293f4f30e60c1418c0/content/docs/views.md
[npp-doc-encoding]: https://github.com/notepad-plus-plus/npp-usermanual/blob/41d96945ec8d11d4100356293f4f30e60c1418c0/content/docs/encoding.md
[npp-doc-prefs]: https://github.com/notepad-plus-plus/npp-usermanual/blob/41d96945ec8d11d4100356293f4f30e60c1418c0/content/docs/preferences.md
[npp-doc-session]: https://github.com/notepad-plus-plus/npp-usermanual/blob/41d96945ec8d11d4100356293f4f30e60c1418c0/content/docs/session.md
[npp-doc-plugins]: https://github.com/notepad-plus-plus/npp-usermanual/blob/41d96945ec8d11d4100356293f4f30e60c1418c0/content/docs/plugins.md
[npp-encoding]: https://github.com/notepad-plus-plus/notepad-plus-plus/blob/dd40fe45598791e9aa38f3aeb200e10fb3d6338a/PowerEditor/src/EncodingMapper.cpp
[npp-io]: https://github.com/notepad-plus-plus/notepad-plus-plus/blob/dd40fe45598791e9aa38f3aeb200e10fb3d6338a/PowerEditor/src/NppIO.cpp
[npp-commands]: https://github.com/notepad-plus-plus/notepad-plus-plus/blob/dd40fe45598791e9aa38f3aeb200e10fb3d6338a/PowerEditor/src/NppCommands.cpp
[npp-print]: https://github.com/notepad-plus-plus/notepad-plus-plus/blob/dd40fe45598791e9aa38f3aeb200e10fb3d6338a/PowerEditor/src/ScintillaComponent/Printer.cpp
[pe-readme]: https://github.com/HaikuArchives/Pe/blob/c94b68ddfbefd591a4a890389a386d87b8d70af2/README.md
[pe-menus]: https://github.com/HaikuArchives/Pe/blob/c94b68ddfbefd591a4a890389a386d87b8d70af2/doc/Menus.html
[pe-state]: https://github.com/HaikuArchives/Pe/blob/c94b68ddfbefd591a4a890389a386d87b8d70af2/doc/PrefState.html
[pe-files-doc]: https://github.com/HaikuArchives/Pe/blob/c94b68ddfbefd591a4a890389a386d87b8d70af2/doc/PrefFiles.html
[pe-invisibles]: https://github.com/HaikuArchives/Pe/blob/c94b68ddfbefd591a4a890389a386d87b8d70af2/doc/PrefInvisibles.html
[pe-worksheet]: https://github.com/HaikuArchives/Pe/blob/c94b68ddfbefd591a4a890389a386d87b8d70af2/doc/PrefWorksheet.html
[pe-io]: https://github.com/HaikuArchives/Pe/blob/c94b68ddfbefd591a4a890389a386d87b8d70af2/Sources/CDocIO.cpp
[pe-ftp]: https://github.com/HaikuArchives/Pe/blob/c94b68ddfbefd591a4a890389a386d87b8d70af2/Sources/CFtpStream.cpp
[pl-readme]: https://github.com/mate-desktop/pluma/blob/837036d73ed49a390cae6bbaac9a4f1e6d5b6de9/README.md
[pl-help]: https://github.com/mate-desktop/pluma/blob/837036d73ed49a390cae6bbaac9a4f1e6d5b6de9/help/C/index.docbook
[pl-encoding]: https://github.com/mate-desktop/pluma/blob/837036d73ed49a390cae6bbaac9a4f1e6d5b6de9/pluma/pluma-encodings.c
[pl-loader]: https://github.com/mate-desktop/pluma/blob/837036d73ed49a390cae6bbaac9a4f1e6d5b6de9/pluma/pluma-document-loader.c
[pl-print]: https://github.com/mate-desktop/pluma/blob/837036d73ed49a390cae6bbaac9a4f1e6d5b6de9/pluma/pluma-print-job.c
[tea-readme]: https://github.com/psemiletov/tea-qt/blob/9599b129100e68cff13a5946679e30819997264b/README.md
[tea-manual]: https://github.com/psemiletov/tea-qt/blob/9599b129100e68cff13a5946679e30819997264b/manuals/en.html
[tea-source]: https://github.com/psemiletov/tea-qt/blob/9599b129100e68cff13a5946679e30819997264b/src/tea.cpp
[tea-speech]: https://github.com/psemiletov/tea-qt/blob/9599b129100e68cff13a5946679e30819997264b/src/speech.cpp
[tea-encoding]: https://github.com/psemiletov/tea-qt/blob/9599b129100e68cff13a5946679e30819997264b/src/enc.cpp
[ta-manual]: https://github.com/orbitalquark/textadept/blob/6139db0156b02a56bdf9890c881e0686ae63c3ea/docs/manual.md
[ta-io]: https://github.com/orbitalquark/textadept/blob/6139db0156b02a56bdf9890c881e0686ae63c3ea/core/file_io.lua
[ta-session]: https://github.com/orbitalquark/textadept/blob/6139db0156b02a56bdf9890c881e0686ae63c3ea/modules/textadept/session.lua
[tm-changes]: https://github.com/textmate/textmate/blob/1e85a16eff2478fcae96ccd91137af6c19819752/Applications/TextMate/about/Changes.md
[tm-document]: https://github.com/textmate/textmate/blob/1e85a16eff2478fcae96ccd91137af6c19819752/Frameworks/document/src/OakDocument.mm
[tm-encoding]: https://github.com/textmate/textmate/blob/1e85a16eff2478fcae96ccd91137af6c19819752/Frameworks/encoding/src/encoding.mm
[tm-editor]: https://github.com/textmate/textmate/blob/1e85a16eff2478fcae96ccd91137af6c19819752/Frameworks/document/src/OakDocumentEditor.mm
[xed-readme]: https://github.com/linuxmint/xed/blob/9be0bdcd6f17c119d41543440066da9287c12d0e/README.md
[xed-changelog]: https://github.com/linuxmint/xed/blob/9be0bdcd6f17c119d41543440066da9287c12d0e/debian/changelog
[xed-print]: https://github.com/linuxmint/xed/blob/9be0bdcd6f17c119d41543440066da9287c12d0e/xed/xed-print-job.c
[xed-tab]: https://github.com/linuxmint/xed/blob/9be0bdcd6f17c119d41543440066da9287c12d0e/xed/xed-tab.c
[xed-encoding]: https://github.com/linuxmint/xed/blob/9be0bdcd6f17c119d41543440066da9287c12d0e/xed/xed-encodings-combo-box.c
[zb-changelog]: https://github.com/pkulchenko/ZeroBraneStudio/blob/483ef0dcb593f9599ff27715472425e39a0588e8/CHANGELOG.md
[zb-readme]: https://github.com/pkulchenko/ZeroBraneStudio/blob/483ef0dcb593f9599ff27715472425e39a0588e8/README.md
[zb-commands]: https://github.com/pkulchenko/ZeroBraneStudio/blob/483ef0dcb593f9599ff27715472425e39a0588e8/src/editor/commands.lua
[zb-print]: https://github.com/pkulchenko/ZeroBraneStudio/blob/483ef0dcb593f9599ff27715472425e39a0588e8/src/editor/print.lua
[ch-readme]: https://github.com/ChimeHQ/Chime/blob/84b7afa4153acce27fc9b28ced457b7cb1acb477/README.md
[ch-document]: https://github.com/ChimeHQ/Chime/blob/84b7afa4153acce27fc9b28ced457b7cb1acb477/Modules/Document/TextDocument.swift
[ch-search]: https://github.com/ChimeHQ/Chime/blob/84b7afa4153acce27fc9b28ced457b7cb1acb477/Modules/Search/SearchBar.swift
[ce-readme]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/README.md
[ce-git-doc]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/Documentation.docc/Git/Git.md
[ce-encoding]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/CodeEdit/Features/Documents/CodeFileDocument/FileEncoding.swift
[ce-invisibles]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/CodeEdit/Features/Settings/Pages/TextEditingSettings/InvisiblesSettingsView.swift
[ce-settings]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/CodeEdit/Features/Settings/Pages/TextEditingSettings/Models/TextEditingSettings.swift
[ce-stash]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/CodeEdit/Features/SourceControl/Client/GitClient+Stash.swift
[ce-fetch]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/CodeEdit/Features/SourceControl/Client/GitClient+Fetch.swift
[ce-stash-view]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/CodeEdit/Features/SourceControl/Views/SourceControlStashView.swift
[ce-shell]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/CodeEdit/Features/TerminalEmulator/Model/ShellIntegration.swift
[ce-warning-list]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/CodeEdit/Features/Settings/Views/InvisibleCharacterWarningList.swift
[ce-pull]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/CodeEdit/Features/SourceControl/Client/GitClient+Pull.swift
[ce-push]: https://github.com/CodeEditApp/CodeEdit/blob/05704cb7d2b19892ffcb9742b0ab3cc6116af165/CodeEdit/Features/SourceControl/Client/GitClient+Push.swift
[npp-notify]: https://github.com/notepad-plus-plus/notepad-plus-plus/blob/dd40fe45598791e9aa38f3aeb200e10fb3d6338a/PowerEditor/src/NppNotification.cpp
[npp-main]: https://github.com/notepad-plus-plus/notepad-plus-plus/blob/dd40fe45598791e9aa38f3aeb200e10fb3d6338a/PowerEditor/src/Notepad_plus.cpp
[pl-modelines-readme]: https://github.com/mate-desktop/pluma/blob/837036d73ed49a390cae6bbaac9a4f1e6d5b6de9/plugins/modelines/README.md
[pl-modelines-parser]: https://github.com/mate-desktop/pluma/blob/837036d73ed49a390cae6bbaac9a4f1e6d5b6de9/plugins/modelines/modeline-parser.c
[pl-modelines-plugin]: https://github.com/mate-desktop/pluma/blob/837036d73ed49a390cae6bbaac9a4f1e6d5b6de9/plugins/modelines/pluma-modeline-plugin.c

[orlo-encoding]: ../../crates/project/src/documents/encoding.rs
[orlo-menus]: ../../crates/ui/src/views/workspace/menus.rs
[orlo-disk]: ../../crates/ui/src/views/workspace/disk.rs
[orlo-save]: ../../crates/ui/src/documents/save.rs
[orlo-io]: ../../crates/project/src/documents/io.rs
[orlo-files]: ../../crates/ui/src/views/workspace/files.rs
[orlo-sessions-ui]: ../../crates/ui/src/views/workspace/sessions.rs
[orlo-fold-command]: ../../crates/ui/src/views/editor/commands.rs
[orlo-session]: ../../crates/workspace/src/sessions/mod.rs
[orlo-settings]: ../../crates/config/src/settings/mod.rs
[orlo-stats]: ../../crates/editor/src/editing/stats.rs
[orlo-actions]: ../../crates/ui/src/views/editor/actions.rs
[orlo-typing]: ../../crates/editor/src/commands/typing.rs
[orlo-selection]: ../../crates/editor/src/selection/mod.rs
[orlo-columns]: ../../crates/editor/src/editing/columns.rs
[orlo-clipboard]: ../../crates/editor/src/editing/clipboard_history.rs
[orlo-viewport]: ../../crates/editor/src/viewport/mod.rs
[orlo-lsp-ui]: ../../crates/ui/src/views/workspace/language.rs
[orlo-lsp]: ../../crates/syntax/src/languages/lsp/mod.rs
[orlo-language-detect]: ../../crates/syntax/src/detection/language.rs
[orlo-manifest]: ../../crates/syntax/src/languages/manifest.rs
[orlo-git]: ../../crates/git/src/lib.rs
[orlo-git-ui]: ../../crates/ui/src/panels/git/mod.rs
[orlo-git-repo]: ../../crates/git/src/repository/mod.rs
[orlo-branches]: ../../crates/git/src/branches/mod.rs
[orlo-commit]: ../../crates/git/src/commits/mod.rs
[orlo-git-history]: ../../crates/git/src/history/mod.rs
[orlo-git-history-ui]: ../../crates/ui/src/panels/git/history.rs
[orlo-term-state]: ../../crates/terminal/src/sessions/mod.rs
[orlo-term-ui]: ../../crates/ui/src/panels/terminal/emulator.rs
[orlo-term-emulation]: ../../crates/terminal/src/emulation/mod.rs
[orlo-term-input]: ../../crates/terminal/src/input/mod.rs
[orlo-term-panel]: ../../crates/ui/src/panels/terminal/mod.rs
[orlo-shell]: ../../crates/terminal/src/shell/mod.rs
[orlo-term-config]: ../../crates/terminal/src/configuration/mod.rs
[geany-manual]: https://github.com/geany/geany/blob/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace/doc/geany.txt
[geany-encoding]: https://github.com/geany/geany/blob/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace/src/encodings.c
[geany-vte]: https://github.com/geany/geany/blob/cb5ff3cf0b63971010e60ea2feedd1a0181c1ace/src/vte.c
[cot-encoding]: https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Resources/CotEditor.help/Contents/Resources/en.lproj/pgs/howto_change_encoding.html
[cot-document]: https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Sources/Document/Document.swift
[cot-print-doc]: https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Resources/CotEditor.help/Contents/Resources/en.lproj/pgs/howto_print.html
[cot-print-src]: https://github.com/coteditor/CotEditor/blob/e188605143e61255699c4b1a49e993f38576641a/CotEditor/Sources/Document/Print/PrintTextView.swift
[rune-editor]: https://github.com/unstablebuild/rune/blob/71537d80dc6b163ddbed0bd69e502a75bdc661d0/docs/docs/learn/standard-editor.md
[rune-autosave]: https://github.com/unstablebuild/rune/blob/71537d80dc6b163ddbed0bd69e502a75bdc661d0/internal/ide/autosave.go
[jedit-gzip]: https://github.com/jEdit-editor/jEdit/blob/388517e3b685d723db004d295d255a200c7784a3/org/gjt/sp/jedit/bufferio/BufferLoadRequest.java
[jedit-registers]: https://github.com/jEdit-editor/jEdit/blob/388517e3b685d723db004d295d255a200c7784a3/org/gjt/sp/jedit/Registers.java
[jedit-elastic]: https://github.com/jEdit-editor/jEdit/blob/388517e3b685d723db004d295d255a200c7784a3/org/gjt/sp/jedit/textarea/ElasticTabstopsTabExpander.java
[atom-fold-doc]: https://github.com/atom/flight-manual.atom.io/blob/678ba0247a61b836a6691521810cb5417b052eef/content/using-atom/sections/folding.md
[brackets-fold]: https://github.com/brackets-cont/brackets/blob/e41ba100d1955f027abf9223cc2cd38e3b7c9adb/src/extensions/default/CodeFolding/main.js
[gnome-readme]: https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/README.md
[gnome-vim]: https://gitlab.gnome.org/GNOME/gnome-text-editor/-/blob/0eda8f023bc3a60256c1c1f064beced1805a3847/src/editor-page-vim.c
[fresh-lsp-doc]: https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/docs/features/lsp.md
[fresh-lsp-src]: https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor/src/app/lsp_requests.rs
[fresh-save-src]: https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor/src/app/on_save_actions.rs
[fresh-term-doc]: https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/docs/features/terminal.md
[fresh-path-src]: https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor/src/services/terminal/path_link.rs
[fresh-term-src]: https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor/src/services/terminal/term.rs
[fresh-mouse-src]: https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor/src/app/terminal_mouse.rs
[fresh-session-doc]: https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/docs/features/session-persistence.md
[lapce-readme]: https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/README.md
[lapce-editor]: https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/lapce-app/src/editor.rs
[lapce-snippet]: https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/lapce-app/src/snippet.rs
[lapce-term]: https://github.com/lapce/lapce/blob/b012cef466741528f338879fa708355217fc40bd/lapce-app/src/config/terminal.rs
[ttt-lsp-doc]: https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/docs-web/src/content/docs/guides/lsp.md
[ttt-lsp-src]: https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/internal/lsp/client.go
[ttt-git-doc]: https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/docs-web/src/content/docs/guides/git.md
[ttt-git-src]: https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/internal/git/git.go
[ttt-term-doc]: https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/docs-web/src/content/docs/guides/terminal.md
[ttt-term-src]: https://github.com/eugenioenko/ttt/blob/cc1a9267a265ff249a98302f702ab2f4146620f0/internal/terminal/terminal.go
[pulsar-git-doc]: https://docs.pulsar-edit.dev/core-packages-and-features/github-package/
[pulsar-git-src]: https://github.com/pulsar-edit/github/blob/285ec77acd8ae6e50dda5289c8744e4049dc5773/lib/git-shell-out-strategy.js
[pulsar-term-doc]: https://docs.pulsar-edit.dev/core-packages-and-features/terminal/
[pulsar-term-find]: https://github.com/pulsar-edit/terminal/blob/558dc4c23230cbf0db314cdd1d426df00cb73448/lib/find-palette.js
[kate-stash]: https://invent.kde.org/utilities/kate/-/blob/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3/addons/project/stashdialog.cpp
[kate-terminal]: https://invent.kde.org/utilities/kate/-/blob/2b6465a886b9d3bfc6eab25065c14ffe025e6ac3/addons/konsole/kateconsole.cpp
[kate-accessible]: https://invent.kde.org/frameworks/ktexteditor/-/blob/ea14509c132f2c016d55b38f7a1ec24cc6f9e7d2/src/view/kateviewaccessible.h
[kate-renderer]: https://invent.kde.org/frameworks/ktexteditor/-/blob/ea14509c132f2c016d55b38f7a1ec24cc6f9e7d2/src/render/katerenderer.cpp
[athas-stash]: https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/crates/version-control/src/git/stash.rs
[athas-worktree]: https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/crates/version-control/src/git/worktree.rs
[athas-worktree-open]: https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/src/features/git/services/git-worktree-open.ts
[athas-term-search]: https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/src/features/terminal/components/terminal-search.tsx
[athas-term-addons]: https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/src/features/terminal/hooks/use-terminal-addons.ts
[athas-term-splits]: https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/src/features/terminal/components/terminal-split-view.tsx
[athas-shell-src]: https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/src/features/terminal/lib/terminal-shell-integration.ts
[athas-shell-launch]: https://github.com/athasdev/athas/blob/67a11409221a7d2f6cf2ebdf5071d55a0a210c4e/crates/terminal/src/shell_integration.rs
[rhun-guide]: https://github.com/vshvedov/rhun/blob/3afaafdb19401f9076d79cb290b4d1a711e89e8c/docs/guide.md
[rhun-term-src]: https://github.com/vshvedov/rhun/blob/3afaafdb19401f9076d79cb290b4d1a711e89e8c/src/app/termview.s

[fresh-config-src]: https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor-core/src/config.rs
[fresh-lsp-manager]: https://github.com/sinelaw/fresh/blob/d1e266fd119b6860ca47a3ea13eab6c8f138d9cb/crates/fresh-editor/src/services/lsp/manager.rs
