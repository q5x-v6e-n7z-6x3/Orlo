# Expanded Editor Features Implementation Plan

> Completion is tracked only in [the canonical feature checklist](editor-comparison-2026-10-29.md). The step checkboxes below are historical planning notes; verification now follows focused feature checks, periodic workspace checkpoints, and the required final gates.

> For agentic workers: use subagent-driven-development to implement and independently review one selected entry at a time. User instructions override skill commit/approval steps.

**Goal:** Complete every Recommended and Optional entry in the updated requirements while preserving Orlo as a focused macOS text editor with Git, a terminal and lightweight language tools.

**Architecture:** Extend the existing ten-crate owners, document/view split, action catalog, settings, widgets and background tasks. Reuse the verified original 27-feature implementation; recheck each entry against its updated criteria and implement only missing behavior. No replacement editing, terminal, Git or language engine.

**Tech Stack:** Rust, GPUI, existing Tree-sitter/syntect/LSP services, native macOS APIs and Git/PTY backends.

**Spec:** [editor-comparison-2026-10-29.md](editor-comparison-2026-10-29.md). The updated source has 15 Recommended and 49 Optional entries (64 selected), despite the follow-up request saying 63. All selected tiers are binding; no arbitrary entry is dropped to fit the stale count.

## Global Constraints

- Preserve the current worktree and history; the original audit baseline 31c83cbedfaf33322ab92b5b6bd3d8ed54b81e8b is historical. On 2026-10-31 the user authorized root commits of verified, accepted features; workers never commit. Pushes, PRs, publishing and releases remain unauthorized. Context reset never means Git reset.
- Do not use GPT-6 Astra. Every worker/reviewer must use another allowed model, with no subagents of its own.
- Execute one feature at a time. Complete listed prerequisites first; N39 consumes accepted T02/T06. N26/N27 are Deferred and unselected, so their special order does not apply.
- Execute all 64 current Recommended/Optional entries; all 19 Deferred entries and all Excluded categories remain outside scope. Already implemented behavior is verified rather than rebuilt.
- Read applicable instructions, docs/architecture/overview.md, docs/development/verification.md, the exact selected entry and current owners/callers before editing. No AGENTS.md was found in this checkout or ancestors at setup.
- Before product edits, add a short behavior contract (defaults, entry points, failure/stale behavior and exact acceptance cases) to the expanded-phase section of editor-features-progress.md.
- Complete model/backend, reachable UI/commands, applicable settings/persistence, meaningful regressions and user docs. The active checkboxes and completion evidence live beside each entry in the requirements file; do not check before verification/review.
- Preserve ordinary editing, inline folds, local snippets, document formatting, clipboard history, recovery/history, partial Git staging, shared views and PTY behavior. New explicit specifications override older routine design choices only within their selected feature; record those changes.
- Async results/edits pin identity/version/selection, validate all targets before application and reject stale work. Text edits are atomic/undoable and respect read-only/Follow/IME protections.
- Reuse dependencies; explain any genuinely necessary addition. Reference peer docs/source without copying implementations. Preserve the original historical audit at editor-comparison-2026-10-29-original.md and the imported research reference.
- Every Cargo invocation uses CARGO_TARGET_DIR=<checkout>/target. Focused checks must select actual tests. Keep command/result logs; do not weaken gates, suppress failures or delete coverage.
- Git verification uses disposable repositories/local remotes only, never this product checkout or real remotes. Test-specific configuration changes use subprocess-local environments, never process-wide environment mutation.
- Root owns serial final workspace fmt, strict all-target Clippy, tests, release build, diff checks and the documented serial release performance suite. Feature workers run relevant checks before advancing; native behavior is exercised where applicable without physical print jobs.
- rg is unavailable on this machine; use grep/find/Python fallback without installing tools. No new checkout or target cleanup.

## Review Focus

- IME, virtual-space/overtype and shared-view edits must remain coherent through new transforms, viewer flags, save-copy and project lifecycle.
- New save/line-ending metadata must preserve exact bytes, undo/saved identity, queued captures, recovery and external-change protections.
- Parser-backed selection/indent/search must reject stale trees and keep ordinary lexical/unsupported-language behavior explicit.
- Terminal search/profile/prompt state must retain correct output provenance, isolation, input bytes and process lifetimes.
- Shared command catalogs, settings layering and result pages must stay reachable and source-coordinate based; optional overlays never alter text/geometry.

## Shared interfaces and sequencing

Use existing Editing/Document/ViewModel and change mapping for editor behavior, shared catalog actions for menu/palette/keymap reachability, existing save/write leases for storage, and captured source contexts for background work. Public additions stay with their responsible crate. Each accepted task report records any exact new API consumed later; consumers inspect that accepted implementation rather than invent parallel helpers. Start links below are ownership anchors, not a prohibition on necessary connected callers/tests.

Accepted E05 interfaces: `Selection::rectangular` is transient provenance for an existing editing
target, separate from real offsets and virtual columns. Preserve it when reconstructing those
targets; ordinary new source-range selection remains ordinary. Snippet placement uses
`Session::with_origins` and `Session::selections()`; range-only `Session::selection()` and
ordinary `select_ranges` retain their roles. No virtual/provenance field is persisted in sessions.

Requirements/acceptance are copied verbatim into each task below so its isolated brief carries the complete behavior contract. Full canonical entry content remains authoritative. Step trackers describe implementation/review workflow; only the canonical requirements checkboxes report feature completion.

## Task 1: N01 — URI-percent encode/decode

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Convert selected text to/from a URI component.
  - Specification: Encode UTF-8 bytes using uppercase %HH; leave only ASCII letters, digits and -._~ unescaped. Decode %HH strictly as UTF-8 and keep + literal; form-url-encoding is not implied. Transform nonempty selections independently and atomically; invalid escapes/UTF-8 leave every target unchanged.
  - Acceptance: Verify `a b/é` → `a%20b%2F%C3%A9` and exact round-trip, literal +, emoji, multiline and multiple selections. Reject %, %GG and invalid UTF-8 without partial edits; one Undo restores all targets.
  - Start: [crates/editor/src/editing/convert.rs](../../crates/editor/src/editing/convert.rs), [crates/ui/src/views/workspace/command_entries.rs](../../crates/ui/src/views/workspace/command_entries.rs), [crates/editor/src/buffer/tests/transforms.rs](../../crates/editor/src/buffer/tests/transforms.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 2: N02 — HTML entity escape/unescape

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Escape selected text for an HTML text/attribute context.
  - Specification: Escape `&`, `<`, `>`, double quote and apostrophe to `&amp;`, `&lt;`, `&gt;`, `&quot;` and `&#39;`. Unescape those names plus valid decimal/hex numeric Unicode entities in one pass; unknown named entities remain literal. Reject malformed numeric entities/lone surrogates atomically. Styled Copy as HTML stays independent.
  - Acceptance: Test all five characters, Unicode numeric references, unknown names and multiple selections. `&amp;lt;` decodes once to `&lt;`. Invalid numeric input changes no target; round-trip and one-step undo pass.
  - Start: [crates/editor/src/editing/convert.rs](../../crates/editor/src/editing/convert.rs), [crates/ui/src/views/workspace/command_entries.rs](../../crates/ui/src/views/workspace/command_entries.rs), [crates/editor/src/buffer/tests/transforms.rs](../../crates/editor/src/buffer/tests/transforms.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 3: N04 — Subword movement, selection and deletion

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Navigate/edit identifier components without consuming a whole word.
  - Specification: Add previous/next subword actions and selection/deletion variants. Split lower-to-upper case, acronym-to-title boundaries, letter/digit transitions and underscore-separated parts; preserve grapheme boundaries and punctuation runs. Keep existing WordLeft/WordRight behavior and bindings. Expose actions in the existing command catalog/keymap system.
  - Acceptance: Pin boundaries for camelCase, HTTPServer, snake_case, item42Value and Unicode/combining text. Test forward/backward symmetry, selection, multicaret deletion, document ends and undo; ordinary word-motion tests remain unchanged.
  - Start: [crates/editor/src/cursor/mod.rs](../../crates/editor/src/cursor/mod.rs), [crates/editor/src/selection/mod.rs](../../crates/editor/src/selection/mod.rs), [crates/ui/src/views/editor/actions.rs](../../crates/ui/src/views/editor/actions.rs), [crates/editor/src/buffer/tests/motion.rs](../../crates/editor/src/buffer/tests/motion.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 4: N22 — Save a Copy As

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Write current buffer content elsewhere without changing its identity or saved state.
  - Specification: Add File > Save a Copy As and a palette action. Capture current text/encoding/BOM/line endings and use protected writes/overwrite checks. Keep original path, dirty state, saved point, read-only flags, undo, views and project association. Reject the original path as a copy destination; another live document ownership check still applies.
  - Acceptance: Copy a dirty named document and an untitled document, including Unicode and every currently supported encoding and an edit during async save. Verify copied bytes represent the captured snapshot and original document state remains unchanged. Explorer Duplicate still copies disk content.
  - Start: [crates/editor/src/buffer/save.rs](../../crates/editor/src/buffer/save.rs), [crates/ui/src/documents/save.rs](../../crates/ui/src/documents/save.rs), [crates/ui/src/views/workspace/files.rs](../../crates/ui/src/views/workspace/files.rs), [crates/ui/src/documents/save_tests.rs](../../crates/ui/src/documents/save_tests.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 5: N16 — Expand/shrink syntax-node selection

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Select parsed nodes through focused nonmodal commands.
  - Specification: Add Expand Syntax Selection and Shrink Syntax Selection using the current bundled Tree-sitter tree. Expand to the smallest enclosing named range strictly larger than the selection; shrink restores prior ranges. Invalidate the range stack on incompatible edits/caret changes and reject stale parse results. Keep Smart Expand lexical behavior and report unavailable parser support.
  - Acceptance: Cover identifier -> expression -> statement nesting, shrink reversal, multicarets, malformed source, Unicode and edits during parsing. Selection changes never rewrite code; broad workspace AST querying is outside the feature.
  - Start: [crates/syntax/src/grammars/treesitter.rs](../../crates/syntax/src/grammars/treesitter.rs), [crates/ui/src/languages/index.rs](../../crates/ui/src/languages/index.rs), [crates/ui/src/views/editor/commands.rs](../../crates/ui/src/views/editor/commands.rs), [crates/editor/src/selection/mod.rs](../../crates/editor/src/selection/mod.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

Scoped N16 implementation and verification are recorded in
`.superpowers/sdd/editor-expanded-features-plan/task-5-report.md`: 709 editor / 169 syntax /
1212 UI tests, 22 focused N16 cases, existing fold/Smart Expand compatibility, strict affected
Clippy/fmt/diff and exact serial release input/highlight gates passed. Canonical checkboxes
remain unchecked pending fresh root-owned independent scoped review.

## Task 6: N17 — Language-sensitive indentation

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Indent newlines and reindent selected lines from language rules.
  - Specification: Initial coverage is Python, Rust, JavaScript and TypeScript. Respect configured tabs/indent width, Python colon/dedent keywords and brace/continuation context while excluding strings/comments. Add Reindent Selection; calculate a validated batch in one transaction. Unsupported languages keep current indentation behavior. Manual formatting remains independent.
  - Acceptance: Test Python blocks/else, nested braces/continuations, strings/comments, tabs, multiline selections and incomplete code. Newline and reindent produce expected whitespace and undo normally without altering nonwhitespace text.
  - Start: [crates/editor/src/commands/typing.rs](../../crates/editor/src/commands/typing.rs), [crates/editor/src/commands/line_editing.rs](../../crates/editor/src/commands/line_editing.rs), [crates/syntax/src/grammars/treesitter.rs](../../crates/syntax/src/grammars/treesitter.rs), [crates/ui/src/views/editor/commands.rs](../../crates/ui/src/views/editor/commands.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 7: E01 — Syntax-driven fold regions

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Discover foldable braces, comments and markup using the existing language parser.
  - Specification: For bundled Tree-sitter languages, compute versioned fold ranges off the UI thread. Use language-aware ranges for fold commands/gutter discovery; retain indentation discovery when no parser/rule is available. Preserve exact selection folding, nested folds and edit mapping. Coalesce duplicate/crossing ranges deterministically.
  - Acceptance: Cover brace code, multiline comments, markup, nested ranges, malformed/incomplete code and an unsupported language. A stale parse result cannot hide newer text. Existing inline-selection folding and indentation tests still pass.
  - Start: [crates/syntax/src/grammars/treesitter.rs](../../crates/syntax/src/grammars/treesitter.rs), [crates/ui/src/views/editor/commands.rs](../../crates/ui/src/views/editor/commands.rs), [crates/editor/src/rendering/fold.rs](../../crates/editor/src/rendering/fold.rs), [crates/ui/src/views/editor/fold_tests.rs](../../crates/ui/src/views/editor/fold_tests.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

Scoped verification complete; production behavior already meets E01. Task7 adds malformed/
incomplete parser coverage, extends the existing normalization fixture for duplicates/crossings/
input order, and clarifies recovery in the user guide. Final checks pass: 726 editor, 170 syntax
(3 existing ignored), 1,221 UI (25 existing ignored), 35 focused UI fold checks, strict affected
all-target Clippy, fmt and diff. Tests/documentation only; no fresh performance run is needed
because runtime paths are unchanged. Exact coverage, actual commands/logs, limits and frozen
task delta are in `.superpowers/sdd/editor-expanded-features-plan/task-7-report.md`.
E01 remains unchecked pending fresh controller-owned independent review.

## Task 8: E02 — Restore collapsed folds

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Retain collapsed regions across reopen and session restoration.
  - Specification: Persist per-view collapsed ranges with document identity/content validation. Restore only valid ranges against the matching recovered/saved text; ignore invalid or stale metadata. Old session files load without fold fields. Shared text views retain independent collapsed states.
  - Acceptance: Close/reopen and restore two views with different folds, including an inline fold. Verify old/corrupt session data and changed disk content do not hide unrelated text or prevent recovery.
  - Start: [crates/workspace/src/sessions/mod.rs](../../crates/workspace/src/sessions/mod.rs), [crates/ui/src/views/workspace/sessions.rs](../../crates/ui/src/views/workspace/sessions.rs), [crates/editor/src/rendering/fold.rs](../../crates/editor/src/rendering/fold.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

E02 verification: existing production behavior satisfies the entry. Added actual project
close/reopen of dirty shared nested/inline folds with changed disk text, missing/malformed/
unknown/crossing optional metadata preserving recovered shared text and valid sibling folds,
and read-only/Follow plus native composition recovery coverage. Fresh full suites pass
726 editor, 111 workspace and 1,223 UI tests (25 existing UI ignored); focused persistence
13, native recovery 2 and fold geometry/input 35 pass, as do strict affected all-target Clippy,
fmt and diff. Production source is unchanged; no new timing run is required. Exact APIs,
commands/logs, limits and frozen task delta are in
`.superpowers/sdd/editor-expanded-features-plan/task-8-report.md`. E02 is accepted after fresh independent Spec Compliance PASS/Task Quality APPROVE review,
with no findings; the canonical requirement is checked. Global final gates remain outstanding.

## Task 9: E03 — Configurable unusual-character warnings

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Make configured problematic codepoints visible without changing text.
  - Specification: Add an opt-in warning-codepoint/range setting and an independent warning style/glyph. Support zero-width and direction-control characters as well as user entries. Show codepoint/name on inspection. Existing NBSP/en/em/ideographic-space marks and whitespace toggles continue working; rendering must preserve caret/selection geometry.
  - Acceptance: Exercise U+00A0, U+200B, U+202E, combining marks and a user-defined codepoint with warnings enabled/disabled. Copy/save bytes and IME positions remain unchanged; invalid settings report errors without breaking rendering.
  - Start: [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs), [crates/ui/src/views/editor/element.rs](../../crates/ui/src/views/editor/element.rs), [crates/ui/src/views/workspace/tools.rs](../../crates/ui/src/views/workspace/tools.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

E03 scoped evidence (2026-10-30): opt-in/default-off warnings and validated singleton/range
preferences now share scalar-first lookup with Character Info. Settings-time disjoint interval
compilation keeps enabled painting logarithmic in configured intervals and bounded to clipped,
non-folded source scalars. Existing glyph/color/whitespace/IME behavior remains covered.
Causal config/Character Info REDs pass; full post-fix config 69/editor 726/UI 1,228 pass (25 existing
UI ignored), with 9 selected config and 12 UI warning checks plus strict affected all-target
Clippy/fmt. Six exact serial release checks each select one passing test; workloads and
budgets are unchanged. Source-coordinate/geometry/native-input/shared-view/copy/save/undo
and malformed settings evidence, actual logs, limits and task-only frozen SHA manifests are in
`.superpowers/sdd/editor-expanded-features-plan/task-9-report.md`. E03 is accepted after independent full review plus scoped approval of the sole fix;
global controller gates remain outstanding.

E03 review fix: reject distinct key spellings for the same numeric interval as a recoverable
preferences error. Causal2-test RED/GREEN and post-fix full69config/726editor/1228UI(25ignored),
strict affected Clippy/fmt pass. Initial review/report remain preserved; fresh scoped review found P2 addressed and no
new breakage, and final hash/diff readback preceded canonical acceptance. Render/index/input sources and
six previously verified exact release-gate workloads remain unchanged.

## Task 10: F01 — Legacy encodings and UTF-32

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Open, reinterpret and save common legacy text without silent character loss.
  - Specification: Retain UTF-8/UTF-16 behavior. Add UTF-32 LE/BE, Windows-1250/1251/1252, ISO-8859-1, Shift JIS, EUC-JP/EUC-KR, GBK/GB18030 and Big5 to explicit encoding choices. Honor Unicode BOMs; keep encoding/BOM per document. Reject unrepresentable saves with the offending character/location and leave disk unchanged. Search, diff and history must use a known encoding or report unsupported input rather than substitute replacement characters.
  - Acceptance: Round-trip representative non-ASCII fixtures and UTF-32 BOM variants; a legacy save containing an unrepresentable emoji fails without changing bytes. Reopen-with-encoding preserves existing dirty-buffer decisions. Existing UTF-8/UTF-16 and metadata-preserving save tests pass.
  - Start: [crates/project/src/documents/encoding.rs](../../crates/project/src/documents/encoding.rs), [crates/project/src/documents/io.rs](../../crates/project/src/documents/io.rs), [crates/editor/src/buffer/save.rs](../../crates/editor/src/buffer/save.rs), [crates/ui/src/documents/save.rs](../../crates/ui/src/documents/save.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.


F01 implementation evidence (independent review pending): existing exact 26 codecs, Unicode
BOM/no-BOM and disk-hint behavior retained; missing scalar/U+ and one-based scalar-location
errors and strict disk-Find malformed-input admission added at their owners. Save/copy/writer,
reopen/Follow/recovery/lease/read-ticket behavior is preserved. Causal real Save/Find and codec
REDs pass; final full project 88/editor 728/UI 1,230 tests pass (25 established UI ignores),
plus five focused expanded regressions, strict affected all-target Clippy/fmt/diff and all six
unchanged serial release filesystem/search/index gates. Exact commands, final-source timings,
covering tests, fixture corrections and frozen diff/log/source hashes are recorded in
`.superpowers/sdd/editor-expanded-features-plan/task-10-report.md`. Checkboxes remain open
for independent review; final controller-owned global/native gates remain outstanding.

F01 review fix1: Go to Reference closed-file search now shares strict admission and known
closed disk hints, while open text/cancellation/generation flows remain. Actual one-test
behavior RED/GREEN plus full1231UI/25ignores and strict UI Clippy/fmt pass; core88project/
728editor sources remain unchanged. Repeated six filesystem gates passed unchanged budgets; fresh scoped review approved P1 addressed with no new breakage;
canonical F01 is accepted.

F01 accepted after full review and fix1 scoped approval; post-fix1231UI and repeated six
release gates pass, current-core88project/728editor results retained on unchanged source.
Root final hash readback precedes metadata-only acceptance; global gates remain pending.

## Task 11: L01 — Editable LSP snippet completions

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Accept server completions containing snippet-format insertions.
  - Specification: Route supported insertTextFormat=2 content through the existing snippet model with fields/mirrors/transforms and normal field navigation. Apply completion ranges/additional edits transactionally after version/capability checks. Advertise only implemented support. Unsupported syntax reports a clear failure without partial edits; N31 adds local choice-popup behavior separately.
  - Acceptance: Use fixture servers for insert/replace ranges, nested fields, mirrors, final tabstop, Unicode protocol coordinates and additional edits. Reject stale/overlapping edits atomically; snippet completion is reachable from the real completion UI and undoes as one insertion.
  - Start: [crates/ui/src/views/workspace/language.rs](../../crates/ui/src/views/workspace/language.rs), [crates/syntax/src/languages/lsp/mod.rs](../../crates/syntax/src/languages/lsp/mod.rs), [crates/editor/src/editing/snippet.rs](../../crates/editor/src/editing/snippet.rs), [crates/ui/src/views/workspace/language_tests.rs](../../crates/ui/src/views/workspace/language_tests.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

L01 worker evidence (independent scoped review pending; checkboxes stay open): existing
runtime satisfies the contract without a product-code rewrite. Four added GPUI fixture tests
prove six UTF-8/16/32 TextEdit/InsertReplaceEdit combinations with nested fields, live mirrors/
transforms, final-tabstop navigation, Unicode additional edits and shared undo/redo; preserved
typed-prefix mapping; atomic unsupported-syntax/overlap failure; delayed read-only/Follow/IME/
version guards. Existing cancellation, caret/server/source/virtual-space guards and local
snippets remain covered. Full affected 728 editor / 170 syntax / 1,235 UI pass (3 syntax + 25 UI established
ignored tests); focused snippet 5 / completion 27 / local-snippet 64 and nonzero protocol checks, strict
affected all-target Clippy/fmt/diff are recorded in task-11-report.md and final frozen logs.
Tests/docs-only: runtime, capability advertisement, settings, workloads and performance budgets
are unchanged; root owns final global/native/release/performance gates. No real-server or
physical OS smoke is claimed.

L01 accepted after independent Spec Compliance APPROVE/Task Quality APPROVE, no findings.
Root six-file hash readback and unchanged production proof precede metadata-only acceptance;
final global/native gates remain pending.

## Task 12: L02 — Signature help

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Show callable signatures and the active parameter while editing calls.
  - Specification: Add capability-gated signatureHelp requests, an explicit command and server-declared trigger/retrigger handling. Display signature text/documentation and active-parameter emphasis; support multiple signatures. Cancel/hide on context exit and reject stale version/caret/server responses. Coordinate with completion/hover focus.
  - Acceptance: Fixture tests cover nested calls, comma triggers, Unicode parameter offsets, empty results, server failure and delayed responses after a caret move. Keyboard dismissal and actual popup rendering are verified.
  - Start: [crates/syntax/src/languages/lsp/mod.rs](../../crates/syntax/src/languages/lsp/mod.rs), [crates/ui/src/views/workspace/language.rs](../../crates/ui/src/views/workspace/language.rs), [crates/ui/src/views/workspace/language_tests.rs](../../crates/ui/src/views/workspace/language_tests.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

Verified implementation (independent review complete): initial review's pending-comma,
retained-payload and pending-lifetime findings are repaired, including the residual explicit
restart that could repin old raw help to a new selection. Explicit requests start fresh;
automatic retriggers preserve their newly prepared context. Current 30 signature UI tests
and 1,261 full UI tests pass (25 existing ignores); unchanged syntax fix1 proof is 6 focused /
174 full passed (3 ignores). Strict affected checks pass. Four exact serial release gates
passed on fix1; fix2's cold explicit setup change does not alter render or ordinary editing
paths. See task-12-report.md for causal RED→GREEN, actual commands, limits and frozen evidence.
Fresh scoped fix2 review approved spec and quality with no remaining findings. Root hash
readback precedes metadata-only acceptance; final global/native gates remain pending.

## Task 13: T01 — Search terminal scrollback

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Find text in retained terminal output without rerunning commands.
  - Specification: Provide terminal-local Find, next/previous, case and regex options over retained logical output lines, including soft-wrapped lines. Highlight matches and scroll to the current one. Handle scrollback eviction/live output with generation checks. Closing Find preserves selection/scroll; searches send no input to the PTY.
  - Acceptance: Test split wraps, Unicode, empty/invalid regex, alternate screen, live output and evicted matches. Verify keyboard routing and visible navigation in the terminal UI.
  - Start: [crates/terminal/src/sessions/mod.rs](../../crates/terminal/src/sessions/mod.rs), [crates/terminal/src/output/mod.rs](../../crates/terminal/src/output/mod.rs), [crates/ui/src/panels/terminal/emulator.rs](../../crates/ui/src/panels/terminal/emulator.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

Verified implementation: literal/case-sensitive defaults with local Case/Regex, Unicode
logical-line/cell matching, exact match copy and cancellable independent snapshots. Query/
options/errors retain the displayed source. Navigation/scroll/selection pin reviewed output;
Paused click or field-local Cmd-Alt-U updates from fresh live output without old selection
coordinates. Close preserves displayed selection/scroll; explicit live input returns to the
existing backend. Compact controls/status fit a 206px body, keeping one fixed28px local row.

Final affected checks:37 terminal /1,272 UI passed (1/25 existing ignores),12 focused UI,
7 focused model cases; strict all-target Clippy/fmt/diff and seven unchanged exact serial
release gates passed. Fresh fix1 review approved spec and quality with all3initial findings
addressed, no new findings. Root verified full10/scoped7files and327inputs before metadata-only
acceptance; raw evidence and limits in task-13-report.md. Final native/global gates pending.

## Task 14: T02 — Terminal file and web links

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Activate file:line:column and HTTP(S) output links with correct working-directory context.
  - Specification: Track validated local OSC7 directory updates and retain the relevant cwd for output lines. Parse quoted/spaced paths and one-based locations; use existing editor navigation. Resolve relative paths against their output cwd. Explicit link activation opens HTTP(S) externally; selection dragging stays available and unsupported URI schemes are not executed.
  - Acceptance: Test paths emitted before/after cd, Unicode/spaces, wrapped output, malformed OSC7, missing files and line/column clamping. Clicking/selecting sends no accidental shell input.
  - Start: [crates/terminal/src/emulation/mod.rs](../../crates/terminal/src/emulation/mod.rs), [crates/terminal/src/sessions/mod.rs](../../crates/terminal/src/sessions/mod.rs), [crates/ui/src/panels/terminal/emulator.rs](../../crates/ui/src/panels/terminal/emulator.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

Verified implementation: existing OSC7/per-cell provenance, displayed-source Cmd-click,
selection and workspace navigation are preserved. File-token normalization now accepts
compiler separators, fully quoted locations and extensionless files with explicit positive
locations. Causal model tests and actual GPUI/PTY proofs cover historical cwd through paused/
retained/command output, Unicode/wrap/clamping, document reuse, HTTP(S), refusals and exact
input isolation. Final42 terminal/1,274 UI passed (1/25 existing ignores),13 model and24
terminal UI focused tests, strict affected Clippy/fmt/diff passed. Fresh independent review
approved spec and quality, no findings. Root nine-file/325-input readback precedes metadata-only
acceptance. Cold activation-only changes leave hot owners/benchmarks unchanged; no unrelated
performance repeat. Native OS handler and final global checks remain pending.

## Task 15: T03 — Terminal application mouse reporting

**Tier:** Recommended. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Forward mouse events when an application requests them while keeping local selection available.
  - Specification: Track terminal mouse modes, including supported X10/button/drag/any-motion and SGR encoding. Send press/release/motion/wheel coordinates relative to terminal cells only when requested. Use Shift as the explicit local-selection override. Reset modes on terminal reset/exit; inactive mouse mode keeps existing selection/scroll behavior.
  - Acceptance: Verify encoded sequences and cell coordinates, drag/release, scrolling, modifier override, alternate screen and reset. A real mouse-aware terminal application receives events while Shift-drag still selects output.
  - Start: [crates/terminal/src/emulation/mod.rs](../../crates/terminal/src/emulation/mod.rs), [crates/terminal/src/input/mod.rs](../../crates/terminal/src/input/mod.rs), [crates/ui/src/panels/terminal/emulator.rs](../../crates/ui/src/panels/terminal/emulator.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 16: F02 — Print and Save as PDF

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Print a document or selected text through the native macOS print dialog.
  - Specification: Provide File > Print with page setup, selection/document scope, font size, wrapping, line-number and syntax-color options. Use a snapshot so printing does not modify the buffer. Support the system PDF destination; paginate Unicode, tabs, long lines and page breaks consistently.
  - Acceptance: Verify pagination and selection scope with deterministic layout tests. Manually print-preview a multi-page Unicode fixture and save a PDF; text, clipping and page order are correct, and path/dirty/undo state stays unchanged.
  - Start: [crates/ui/src/platform/macos/mod.rs](../../crates/ui/src/platform/macos/mod.rs), [crates/ui/src/views/workspace/menus.rs](../../crates/ui/src/views/workspace/menus.rs), [crates/ui/src/views/editor/element.rs](../../crates/ui/src/views/editor/element.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 17: F03 — Read-only log follow

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Follow file growth in a dedicated read-only mode.
  - Specification: Expose Start/Stop Following. Append decoded new bytes, retaining partial multibyte sequences between reads. Follow the bottom only while the user is already there; scrolling upward pauses automatic scrolling. Handle truncation and file replacement explicitly. Refuse to enter with unsaved edits until the ordinary save/discard decision is resolved.
  - Acceptance: Exercise append, split UTF-8 sequences, truncation, rotation and stop/restart with temporary files. Scrolling back remains stable while output arrives; following never writes the source file or spawns a replacement command.
  - Start: [crates/project/src/watchers/mod.rs](../../crates/project/src/watchers/mod.rs), [crates/ui/src/views/workspace/disk.rs](../../crates/ui/src/views/workspace/disk.rs), [crates/ui/src/views/editor/mod.rs](../../crates/ui/src/views/editor/mod.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

Worker implementation/verification evidence: retained the current Follow engine, added four
actual-file GPUI acceptance tests and narrowed user-scroll resumption to the remembered
selection generation. Arbitrary, selected, multiple, virtual and no-frame review stay stable.
Full affected 728 editor/88 project/1287 UI pass (25 established ignored); focused/strict and
exact serial hot-gate evidence, commands, limits and frozen snapshot17 inputs are in
`.superpowers/sdd/editor-expanded-features-plan/task-17-report.md`. All three boxes remain
unchecked pending independent scoped review.

## Task 18: F04 — Opt-in autosave to disk

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Save eligible dirty files automatically, separately from recovery snapshots.
  - Specification: Default off. Allow user/project opt-in and a positive idle-delay setting; default delay when enabled is 2 seconds. Save only named writable documents through the existing protected save path. Defer during IME composition, save conflicts or an in-flight save; do not choose paths for untitled buffers. Keep manual save and recovery behavior.
  - Acceptance: Use controlled timers to prove rapid edits coalesce into one write, errors remain visible, external modifications are not overwritten and untitled/read-only files are skipped. Failed autosave leaves the document dirty.
  - Start: [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs), [crates/ui/src/documents/save.rs](../../crates/ui/src/documents/save.rs), [crates/ui/src/views/workspace/disk.rs](../../crates/ui/src/views/workspace/disk.rs), [crates/ui/src/views/workspace/files.rs](../../crates/ui/src/views/workspace/files.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

Worker evidence (review pending): existing protected/shared autosave retained; added validated
`file_autosave_delay_ms` (whole 1..=4294967295ms, default 2000) through existing user/project/
scoped settings. Real-file controlled-clock default/override/coalescing/owner tests and retained
IME/in-flight-save/conflict/dirty/close/formatting guards pass. Full70 config/728 editor/89 project/
111 workspace/1,290 UI pass (25 established UI ignores), focused settings+28 autosave+73 save tests,
strict affected all-target Clippy/fmt/diff and four unchanged exact serial release gates pass.
Exact commands, causal failures, self-review, native/global limits and snapshot18 freeze are in
`.superpowers/sdd/editor-expanded-features-plan/task-18-report.md`. All boxes remain unchecked
for independent scoped review; no save-engine/L04-policy/N22/F01/F03 interface changes.

## Task 19: F07 — Discarded untitled-document rescue

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Recover an untitled buffer after an explicit close/discard.
  - Specification: Before discarding nonempty untitled text, store a private rescue snapshot distinct from hot-exit state. Add a searchable Rescue picker with preview, restore and permanent-delete actions. Restore into a new dirty untitled document. Bound retention to the latest 50 entries and 30 days; provide Clear Rescue and honor private storage permissions.
  - Acceptance: Discard, relaunch, preview and restore an untitled Unicode buffer. Verify retention pruning, corrupt-entry isolation and permanent deletion; closing an empty draft creates no rescue entry.
  - Start: [crates/workspace/src/sessions/mod.rs](../../crates/workspace/src/sessions/mod.rs), [crates/ui/src/views/workspace/files.rs](../../crates/ui/src/views/workspace/files.rs), [crates/ui/src/views/workspace/sessions.rs](../../crates/ui/src/views/workspace/sessions.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.


Task19 implementation evidence (independent scoped review pending): extended existing private
Store to latest50/30days within whole1MiB per draft/8MiB aggregate; worker-loaded1KiB content
keys feed the existing Choices filter. Selected full read-only scratch dock preview offers
independent dirty-untitled restore, confirmed exact-record permanent Delete, Browse and Clear.
Surface/generation/membership rechecks reject pending/stale Delete/Clear results and actions;
preview editors never enter session/hot-exit recovery. Actual disposable Unicode close, fresh
Store,51-entry retention,30-day boundary, corrupt siblings, private guards, escaped50-row bounds,
shared/IME and cross-window stale behavior are covered. Exact checks, causal evidence, limits
and frozen snapshot19 delta are recorded in `.superpowers/sdd/editor-expanded-features-plan/task-19-report.md`.
All Task19/canonical boxes remain unchecked for root review.

Task19 fix1 review corrections: safe oversized artifacts cannot evict usable siblings; worker
catalog/selected reads return atomic generation/revision/expiry admission stamps, with no
foreground filesystem work or storage-lock wait. Root's compact toolbar retains its real220px/
440px/narrow-bottom pointer/bounds regression. Delayed Clear preserves replacement/new-generation
surfaces and focus. Terminal loading failures keep focused visible Browse/Return errors even
without the status bar. Causal REDs and final checks are appended to task19 report; original
v1 release measurements are historical, not reruns on this fix. All boxes remain unchecked
for fresh scoped independent review of all five findings.

## Task 20: E04 — Overtype mode

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Offer explicit insert/overwrite editing with a visible mode indicator.
  - Specification: Default to insertion. Add a toggle action and status indicator. In overwrite mode replace one following grapheme per typed grapheme without consuming line breaks; at EOL insert normally. Honor read-only state, multicarets and normal undo; define IME commits as committed text rather than destructive composition updates.
  - Acceptance: Overwrite ASCII, emoji/combining graphemes and multicaret text; test EOL, selection replacement, undo/redo and IME commit. Switching documents/modes cannot leave an invisible overwrite state.
  - Start: [crates/editor/src/commands/typing.rs](../../crates/editor/src/commands/typing.rs), [crates/ui/src/views/editor/input.rs](../../crates/ui/src/views/editor/input.rs), [crates/ui/src/views/editor/actions.rs](../../crates/ui/src/views/editor/actions.rs), [crates/ui/src/views/editor/mod.rs](../../crates/ui/src/views/editor/mod.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

E04 scoped verification is accepted after independent Spec/Quality approval: existing runtime retained; three
GPUI acceptance tests and strengthened immutable input proof added. Full suites pass: 728 editor,
89 project, 119 workspace and 1,310 UI tests (25 established ignores). Focused checks pass:
20 UI / 13 model overtype, 49 undo, 14 virtual-space, 20 snippet, 13 recovery, 22 Save Copy,
6 Print and 1 exact Follow test. Strict
affected all-target Clippy/fmt/diff pass. No source hot path or performance owner changes;
no redundant release gates claimed. Snapshot20 delta, exact commands/individual exits/UTC
ledger, final input/artifact hashes and readback are recorded in task-20-report.md. All
three boxes and canonical E04 are checked after root corroboration of the frozen evidence. An optional indicator-geometry test hardening suggestion is tracked for final triage; no required E04 gap remains.

## Task 21: E05 — Virtual space beyond line ends

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Place/edit carets beyond EOL without immediately adding padding.
  - Specification: Provide off, rectangular-only and all-carets policies; default off. Track virtual display columns separately from document offsets. Materialize spaces/tabs according to indentation policy only on an edit. Preserve existing explicit padding and define copy/delete/navigation for virtual positions without invalid offsets.
  - Acceptance: Rectangularly type across short/long lines, cancel without changes and undo the first materialized padding. Test tabs, wide characters, soft wraps, multiple carets and deletion at virtual EOL.
  - Start: [crates/editor/src/selection/mod.rs](../../crates/editor/src/selection/mod.rs), [crates/editor/src/viewport/mod.rs](../../crates/editor/src/viewport/mod.rs), [crates/editor/src/commands/column_editing.rs](../../crates/editor/src/commands/column_editing.rs), [crates/ui/src/views/editor/mouse.rs](../../crates/ui/src/views/editor/mouse.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

E05 accepted after fresh scoped Spec/Quality PASS: I1 native, I2 snippet/completion, I3 macro and
M1 geometry-cardinality fixes are addressed. Final71 config/742 editor/1326 UI/119 workspace and
doctests pass (25 established UI ignores); focused preservation/strict/fmt/diff and12 exact serial
release gates pass. Root2203hashes/381source+scripts/both deltas/868V1archive corroborated, source
unchanged through metadata acceptance. Exact logs, failures, interrupted compile, custody and
limits remain in task-21-report.md. The live M1 report label is corrected with its reviewed
precorrection copy preserved. Final whole-workspace/native/release gates remain pending.

## Task 22: E06 — Declarative file modelines

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Read a small safe set of file-local editing preferences.
  - Specification: Default off. Scan only the first/last 5 physical lines for documented Vim/Emacs/Kate-style declarations of tab width, indentation width/spaces, wrap and language. Validate values; ignore commands, expressions and unknown keys. Explicit current-view overrides win, then enabled modelines, EditorConfig, scoped settings and global defaults.
  - Acceptance: Verify recognized forms, precedence, malformed/oversized values and an attempted executable modeline. Loading a declaration performs no process execution, filesystem write or network operation.
  - Start: [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs), [crates/config/src/settings/editorconfig.rs](../../crates/config/src/settings/editorconfig.rs), [crates/ui/src/views/workspace/customization.rs](../../crates/ui/src/views/workspace/customization.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 23: L03 — Format selected ranges

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Request language-server formatting for selected text.
  - Specification: Expose Format Selection only when rangeFormatting is supported. Convert each nonempty selection to negotiated protocol positions; reject overlapping requests/edits. Validate the complete returned edit batch against the original version and writable documents, then apply it in one undo transaction. Keep full-document formatting.
  - Acceptance: Fixture tests prove selected ranges, tabs/Unicode offsets, empty selections, unsupported servers and stale/overlapping replies behave deterministically without partial changes.
  - Start: [crates/ui/src/views/workspace/language.rs](../../crates/ui/src/views/workspace/language.rs), [crates/syntax/src/languages/lsp/mod.rs](../../crates/syntax/src/languages/lsp/mod.rs), [crates/ui/src/views/workspace/language_tests.rs](../../crates/ui/src/views/workspace/language_tests.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 24: L04 — Formatter on save

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Optionally format a document before the normal protected save.
  - Specification: Default off per language/project. Run one configured document formatter for an explicit save (and autosave when F04 is available), with cancellation and a 5-second default timeout. Save the validated formatted snapshot; reject stale formatter edits and avoid recursive save hooks. On failure retain dirty edits and report it; offer an explicit Save Without Formatting action.
  - Acceptance: Verify a successful format/save, unsupported formatter, timeout, formatting error, edit during formatting and repeated saves. No partial format or silent unformatted write occurs on failure; manual document formatting still works.
  - Start: [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs), [crates/ui/src/views/workspace/language.rs](../../crates/ui/src/views/workspace/language.rs), [crates/ui/src/documents/save.rs](../../crates/ui/src/documents/save.rs), [crates/ui/src/views/workspace/files.rs](../../crates/ui/src/views/workspace/files.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 25: L05 — Runtime language-server configuration

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Configure another language/filetype without recompiling Orlo.
  - Specification: Extend existing language manifests/settings with stable language IDs, file/glob/shebang matches, server executable/arguments and root markers. Use existing fallback highlighting when no bundled parser exists. Reload validates configuration and restarts only affected sessions; project-local executable settings run only under the existing explicit configuration/trust model.
  - Acceptance: A fixture language not in the compiled enum gets detection, completion/diagnostics/navigation through a fake server. Invalid config, missing executables and reloads preserve unrelated servers and present actionable errors.
  - Start: [crates/syntax/src/languages/manifest.rs](../../crates/syntax/src/languages/manifest.rs), [crates/syntax/src/detection/language.rs](../../crates/syntax/src/detection/language.rs), [crates/ui/src/views/workspace/language.rs](../../crates/ui/src/views/workspace/language.rs), [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 26: G01 — Explicit remote Git operations

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Provide deliberate fetch/pull/push/upstream actions without background network contact.
  - Specification: Use asynchronous Git subprocesses and the user credential helpers/SSH configuration. Show remote/ref/upstream targets before dispatch. Default pull to fast-forward-only and push to ordinary non-force push; errors remain actionable. Status/opening a repository never contacts remotes. Update the documented local-only policy only when this selected feature is implemented.
  - Acceptance: Use local bare repositories/remotes to verify fetch, fast-forward pull, push, missing upstream, non-fast-forward rejection, authentication cancellation and command failure. No automatic fetch or force push is introduced.
  - Start: [crates/git/src/lib.rs](../../crates/git/src/lib.rs), [crates/git/src/operations/mod.rs](../../crates/git/src/operations/mod.rs), [crates/ui/src/panels/git/mod.rs](../../crates/ui/src/panels/git/mod.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 27: G02 — Stash controls

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: List, preview, create, apply, pop and drop local stashes.
  - Specification: Identify stash entries by object identity as well as display index so refreshes cannot target another stash. Creation includes staged/unstaged changes with an explicit include-untracked choice. Preview file changes; apply/pop use Git conflict behavior. Conflicted pop retains its stash and opens existing conflict tools.
  - Acceptance: Test staged/unstaged/untracked files, multiple entries, stale indexes, conflicting apply/pop and drop. Operations do not discard unsaved editor buffers or report success after Git failure.
  - Start: [crates/git/src/lib.rs](../../crates/git/src/lib.rs), [crates/git/src/operations/mod.rs](../../crates/git/src/operations/mod.rs), [crates/ui/src/panels/git/mod.rs](../../crates/ui/src/panels/git/mod.rs), [crates/git/src/tests.rs](../../crates/git/src/tests.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 28: G03 — Worktree lifecycle

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Create/open/remove/prune worktrees through the Git UI.
  - Specification: List paths, branches and locked/prunable states. Create using an existing or explicitly new branch, then open through the normal workspace/window route. Refuse ordinary removal of dirty/locked worktrees; pruning affects only eligible metadata. Reuse current branch-in-use safeguards; do not add implicit force removal.
  - Acceptance: Temporary-repository tests cover linked discovery, create/open, branch already checked out, dirty/locked removal rejection and stale-directory pruning. Existing worktree-aware branch deletion protections pass.
  - Start: [crates/git/src/repository/mod.rs](../../crates/git/src/repository/mod.rs), [crates/git/src/branches/mod.rs](../../crates/git/src/branches/mod.rs), [crates/ui/src/panels/git/mod.rs](../../crates/ui/src/panels/git/mod.rs), [crates/ui/src/views/workspace/projects.rs](../../crates/ui/src/views/workspace/projects.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 29: G04 — Amend and commit composition

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Amend the current commit and compose coauthor trailers explicitly.
  - Specification: Add an explicit amend mode; keep ordinary staged-only commit as the default. Preserve the previous author unless the user changes it. Validate/edit the complete message and coauthor name/email fields, deduplicating identical trailers. Honor existing Git signing configuration and report signing/commit failures.
  - Acceptance: Verify ordinary commits, amend with/without staged changes, unborn HEAD, Unicode message/trailers, duplicate coauthors and signing failure. A failed commit leaves index/worktree untouched.
  - Start: [crates/git/src/commits/mod.rs](../../crates/git/src/commits/mod.rs), [crates/ui/src/panels/git/mod.rs](../../crates/ui/src/panels/git/mod.rs), [crates/git/src/tests.rs](../../crates/git/src/tests.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 30: T04 — Multiple terminal sessions

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Keep several independent PTYs per window.
  - Specification: Introduce terminal tabs with create/rename/switch/close and active-session identity. Each session has separate output, cwd, search state and process lifetime. Hiding the panel preserves sessions; closing a running session uses existing process-close conventions. This task requires tabs; terminal splits and process resurrection are outside its acceptance criteria.
  - Acceptance: Prove two sessions do not share input/output, resize/focus targets the active viewport and closing one leaves others alive. Verify hide/show and clean shutdown without leaked PTYs.
  - Start: [crates/ui/src/panels/terminal/mod.rs](../../crates/ui/src/panels/terminal/mod.rs), [crates/terminal/src/sessions/mod.rs](../../crates/terminal/src/sessions/mod.rs), [crates/terminal/src/pty/mod.rs](../../crates/terminal/src/pty/mod.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 31: T05 — Terminal profiles and scrollback settings

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Choose shell executable/arguments and retained output size.
  - Specification: Add named profiles with executable, argument array, optional startup directory and a nonnegative scrollback-line limit; keep the current login-shell/10,000-line default. Spawn arguments directly rather than concatenate an implicit shell string. Validate paths/options, apply a profile to new sessions and define trimming when retention is reduced.
  - Acceptance: Use fake shells/PTY fixtures to verify arguments containing spaces, missing executables, cwd fallback and independent session profiles. Test zero/small limits and trimming; existing default startup remains compatible.
  - Start: [crates/terminal/src/configuration/mod.rs](../../crates/terminal/src/configuration/mod.rs), [crates/terminal/src/shell/mod.rs](../../crates/terminal/src/shell/mod.rs), [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs), [crates/ui/src/panels/terminal/mod.rs](../../crates/ui/src/panels/terminal/mod.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 32: T06 — Terminal prompt and command boundaries

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Recognize commands/output/status for navigation and copy-last-output.
  - Specification: Parse supported OSC133 prompt/input/output/end markers into bounded ranges tied to retained output. Add previous/next command and Copy Last Output actions, with optional exit-status markers. Shell integration is explicitly enabled and shell-specific; terminals without markers continue working. Do not infer completed commands by scraping prompt text.
  - Acceptance: Replay success/failure, interrupted commands, partial/malformed markers, clear/reset and evicted output. Command navigation/copy excludes prompts and never executes output; a supported shell is smoke-tested.
  - Start: [crates/terminal/src/emulation/mod.rs](../../crates/terminal/src/emulation/mod.rs), [crates/terminal/src/sessions/mod.rs](../../crates/terminal/src/sessions/mod.rs), [crates/terminal/src/shell/mod.rs](../../crates/terminal/src/shell/mod.rs), [crates/ui/src/panels/terminal/emulator.rs](../../crates/ui/src/panels/terminal/emulator.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 33: T07 — Send editor selection to terminal

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Paste selected editor text into the existing interactive terminal.
  - Specification: Expose separate Send Selection and Send and Run actions. Plain Send writes text using terminal paste/bracketed-paste rules without appending a newline; Run explicitly appends one. Multiple nonempty selections join in document order with newlines. Target the chosen active session; create/show one when needed and report a dead PTY.
  - Acceptance: Verify exact bytes/newline policy, multiline Unicode, multicarets, bracketed paste and exited sessions. The action uses the existing PTY rather than a separate shell-filter process.
  - Start: [crates/ui/src/views/editor/commands.rs](../../crates/ui/src/views/editor/commands.rs), [crates/ui/src/panels/terminal/mod.rs](../../crates/ui/src/panels/terminal/mod.rs), [crates/terminal/src/input/mod.rs](../../crates/terminal/src/input/mod.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 34: N03 — Regex and string-literal escaping

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Provide explicit literal-escaping operations with named dialects.
  - Specification: Add Escape Regex for Orlo regex metacharacters, Quote/Unquote JSON String using the existing JSON implementation, and Decode Text Escapes for backslash/quote, n/r/t/b/f, xHH, uHHHH and UHHHHHHHH forms. For text escapes xHH denotes U+00HH; combine valid surrogate pairs and reject isolated surrogates, unknown escapes and trailing backslashes. Keep URI/HTML operations separate.
  - Acceptance: Literal regex output matches the original selected text. JSON quoting round-trips quotes/newlines/emoji. Text-escape fixtures cover Unicode pairs, control forms and malformed input; every multi-selection failure is atomic.
  - Start: [crates/editor/src/editing/convert.rs](../../crates/editor/src/editing/convert.rs), [crates/editor/src/editing/json.rs](../../crates/editor/src/editing/json.rs), [crates/ui/src/views/workspace/command_entries.rs](../../crates/ui/src/views/workspace/command_entries.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 35: N05 — Paragraph and sentence navigation

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Move/select/delete at paragraph or sentence boundaries.
  - Specification: Add previous/next paragraph and sentence motions, selection variants, Select Sentence and Delete Sentence. Paragraphs are separated by blank lines; sentences end at .?! followed by whitespace/end, including closing quotes/brackets. Specify this heuristic in the user guide; retain existing paragraph expansion.
  - Acceptance: Use prose fixtures with blank lines, quoted punctuation, Unicode and no final punctuation. Check both directions, boundaries, multiple carets and undo; abbreviation fixtures follow the documented punctuation heuristic.
  - Start: [crates/editor/src/cursor/mod.rs](../../crates/editor/src/cursor/mod.rs), [crates/editor/src/selection/mod.rs](../../crates/editor/src/selection/mod.rs), [crates/ui/src/views/editor/actions.rs](../../crates/ui/src/views/editor/actions.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 36: N06 — Transpose adjacent words

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Swap adjacent words without manually selecting them first.
  - Specification: Add a distinct Transpose Words action. At a caret inside a word, swap that word with the following word; in separating whitespace choose the words on either side, and at document end choose the final two. Use existing word classification, preserve intervening punctuation/space and existing grapheme/selection transpose actions. Reject overlapping multicaret targets atomically.
  - Acceptance: Verify alpha, beta -> beta, alpha with separators preserved, end-of-document behavior, Unicode and multicarets. A missing partner is a no-op; one Undo restores the original text.
  - Start: [crates/editor/src/commands/transform.rs](../../crates/editor/src/commands/transform.rs), [crates/editor/src/cursor/mod.rs](../../crates/editor/src/cursor/mod.rs), [crates/ui/src/views/editor/actions.rs](../../crates/ui/src/views/editor/actions.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 37: N07 — Replace/remove existing surrounds

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Change or delete an existing enclosing pair around text.
  - Specification: Expose Change Surround and Delete Surround with a bracket/quote target and positive outer-pair count. Reuse matching rules and retain interior text/selections. Handle escaping and malformed pairs; unmatched targets are no-ops. Preserve current automatic wrapping and empty-pair backspace.
  - Acceptance: Change (value) to [value], remove the outer pair from nested pairs and test quoted/escaped strings, unmatched pairs and multiple nonoverlapping targets. Undo restores each operation atomically.
  - Start: [crates/editor/src/commands/typing.rs](../../crates/editor/src/commands/typing.rs), [crates/editor/src/commands/transform.rs](../../crates/editor/src/commands/transform.rs), [crates/ui/src/views/editor/actions.rs](../../crates/ui/src/views/editor/actions.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 38: N08 — Selection complement and regex set operations

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Manipulate selected ranges without modifying document text.
  - Specification: Add Invert Selections, Split Selections by Regex, Keep Selections Matching Regex and Remove Matching Selections. Complement is relative to the document; split excludes delimiter matches; predicates inspect each complete selected range. Use existing regex dialect/limits and normalize overlap. If a result set is empty, retain one valid caret rather than an invalid selection state.
  - Acceptance: For abcd with b selected, inversion selects a and cd. Splitting a,b,c on comma selects a/b/c; keep/remove filters preserve range identity. Test reversed ranges, empty/zero-width matches, Unicode and selection undo; text stays byte-for-byte unchanged.
  - Start: [crates/editor/src/selection/mod.rs](../../crates/editor/src/selection/mod.rs), [crates/editor/src/search/model.rs](../../crates/editor/src/search/model.rs), [crates/ui/src/views/editor/actions.rs](../../crates/ui/src/views/editor/actions.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 39: N09 — Increment/decrement existing values

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Adjust selected numbers, dates or times rather than replace them with a generated sequence.
  - Specification: Add positive/negative signed-step actions. Support whole signed decimal/hex/binary/octal integers with checked signed-128-bit arithmetic, retaining radix prefix, case, minimum zero padding and underscore grouping. Also support ISO YYYY-MM-DD by days and HH:MM[:SS]/ISO date-times by minutes. Validate every selected value first and reject unsupported formats/overflow atomically.
  - Acceptance: Verify 009 + 1 -> 010, 0x0f + 1 -> 0x10, negatives, underscore grouping, month/year rollover and minute changes retaining seconds. Existing column numbering is unchanged; multiple target edits undo together.
  - Start: [crates/editor/src/editing/columns.rs](../../crates/editor/src/editing/columns.rs), [crates/editor/src/commands/transform.rs](../../crates/editor/src/commands/transform.rs), [crates/ui/src/views/workspace/command_entries.rs](../../crates/ui/src/views/workspace/command_entries.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 40: N10 — Align regex fields and selected columns

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Align matching fields/selection widths using display columns.
  - Specification: Provide Align on Pattern with a validated capture index and Align Selected Fields for same-line ranges. Compute padding from tab-expanded/grapheme display widths; keep delimiters/content and unmatched lines unchanged. Allow several field columns across selected lines. Existing caret-only padding and CSV operations remain separate.
  - Acceptance: Test key=value rows, a selected capture, two field columns, tabs/wide/combining characters and lines with no match. Alignment is stable on a second run and one Undo restores the original.
  - Start: [crates/editor/src/commands/column_editing.rs](../../crates/editor/src/commands/column_editing.rs), [crates/editor/src/editing/columns.rs](../../crates/editor/src/editing/columns.rs), [crates/ui/src/views/workspace/transforms.rs](../../crates/ui/src/views/workspace/transforms.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 41: N11 — Search by syntax class

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Limit find/replace to comments, strings or their complements.
  - Specification: Add an Any/Comments/Strings/Comments-or-Strings mask with include/exclude choices. Classify a match by its starting position using current parser/grammar scopes; document this for multiline/cross-scope matches. Keep selection and regex/case/word options composable. Unsupported language classification is shown as unavailable rather than silently broadening the search.
  - Acceptance: Fixtures distinguish code, comments, normal/raw strings and interpolation. Verify replacements only touch eligible matches, version changes invalidate stale classification and unsupported syntax reports the limit.
  - Start: [crates/editor/src/search/model.rs](../../crates/editor/src/search/model.rs), [crates/ui/src/panels/search/find.rs](../../crates/ui/src/panels/search/find.rs), [crates/syntax/src/highlighting/highlight.rs](../../crates/syntax/src/highlighting/highlight.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 42: N12 — Search within previous matching lines

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Restrict a new query to prior source hit lines, rather than all matching files.
  - Specification: Retain file/buffer identity, source version and merged hit-line intervals for the chosen result set. Add Previous Matching Lines as an explicit scope. The complete new match must fit within an allowed contiguous interval. Invalidate changed source intervals and request refresh; retain the existing previous-files scope and substring browser filter.
  - Acceptance: Two queries distinguish hit-line scope from previous-file and path filtering. Cover multiline matches, open unsaved buffers, Unicode, missing files and a source edit after the first query; stale intervals never produce unsafe replacements.
  - Start: [crates/editor/src/search/results.rs](../../crates/editor/src/search/results.rs), [crates/ui/src/panels/search/find.rs](../../crates/ui/src/panels/search/find.rs), [crates/ui/src/panels/search/results.rs](../../crates/ui/src/panels/search/results.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 43: N13 — Independent structured search-result pages

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Revisit several queries with independent options, results and replacement review state.
  - Specification: Maintain up to 8 per-window structured sets with switch/close/refresh actions. Each stores scope/query/options/status/checkboxes and source versions. Refresh replaces only that set; replacement still uses existing stale-result checks. A ninth set retires the oldest inactive non-running set; if none is eligible, reject creating the new set with an explanatory status. Preserve appended textual Find Results; simultaneous background searches are not required.
  - Acceptance: Switch/refresh two different queries without overwriting each other, apply replacements to the selected set and reject stale results. Verify bounded retention, cancellation and closing pages while text-result navigation still works.
  - Start: [crates/ui/src/views/workspace/mod.rs](../../crates/ui/src/views/workspace/mod.rs), [crates/ui/src/panels/search/find.rs](../../crates/ui/src/panels/search/find.rs), [crates/ui/src/panels/search/results.rs](../../crates/ui/src/panels/search/results.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 44: N14 — Reversible narrowing and matching-line views

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Hide text outside a focus region or query without deleting it.
  - Specification: Add Narrow to Selection/Fold, Show Matching Lines with a nonnegative context-line count (default 0), and Clear Filter. Keep the previous fold state so clearing restores it. Track focus ranges through edits; recompute regex visibility from current versions. All edits/copy/navigation use source coordinates, not projected row numbers.
  - Acceptance: Filter a document and clear it with identical text, folds and undo history. Test editing inside the focus, matches appearing/disappearing, nested folds, context and no matches; hidden lines are never deleted.
  - Start: [crates/editor/src/rendering/fold.rs](../../crates/editor/src/rendering/fold.rs), [crates/editor/src/viewport/mod.rs](../../crates/editor/src/viewport/mod.rs), [crates/ui/src/views/editor/commands.rs](../../crates/ui/src/views/editor/commands.rs), [crates/ui/src/views/editor/fold_tests.rs](../../crates/ui/src/views/editor/fold_tests.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 45: N15 — Fold traversal commands

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Jump to next/previous/parent fold headers and select a current fold.
  - Specification: Use the current valid fold ranges, including indentation/selection ranges, without requiring syntax folding. Define parent as the nearest strictly enclosing range; next/previous follow source order without wrap. Provide selection variants and reveal destinations with existing view navigation.
  - Acceptance: Test nested/sibling/inline folds, no parent, document boundaries and folded destinations. Traversal does not change text or fold state except the reveal needed to show the target.
  - Start: [crates/editor/src/rendering/fold.rs](../../crates/editor/src/rendering/fold.rs), [crates/ui/src/views/editor/commands.rs](../../crates/ui/src/views/editor/commands.rs), [crates/ui/src/views/editor/actions.rs](../../crates/ui/src/views/editor/actions.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 46: N20 — All-open-buffer bookmark picker

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Find a bookmark by filename, line and text preview across open documents.
  - Specification: Build a picker from existing view bookmarks, deduplicating shared buffer/line targets while retaining a valid owning view. Filter by filename/text; choosing a row reveals and jumps to that bookmark. Refresh/validate when text or tabs change. Keep next/previous local bookmark commands and existing persistence.
  - Acceptance: Create bookmarks across two files/shared views, filter/select them and handle deleted lines/closed tabs while the picker is open. No duplicate shared-buffer rows or stale jumps remain.
  - Start: [crates/editor/src/commands/bookmarks.rs](../../crates/editor/src/commands/bookmarks.rs), [crates/ui/src/views/editor/commands.rs](../../crates/ui/src/views/editor/commands.rs), [crates/ui/src/overlays/quick_panel.rs](../../crates/ui/src/overlays/quick_panel.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 47: N21 — Activate document file/address text

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Open a selected path:line:column from an ordinary text document.
  - Specification: Add Open Selected Path/Address and explicit Open Selected URL actions. Parse optional matching quotes and one-based numeric location suffixes; resolve relative paths from the current file parent, then project root for untitled buffers. Preserve spaces/Unicode, reuse location clamping and report missing/ambiguous targets. HTTP(S) URLs launch only on the explicit URL action.
  - Acceptance: Verify relative/absolute paths with spaces, quoted targets, line/column suffixes, untitled context, missing files and ordinary URLs. Text selection/copy and existing result-editor links remain unchanged.
  - Start: [crates/ui/src/views/editor/mouse.rs](../../crates/ui/src/views/editor/mouse.rs), [crates/ui/src/views/workspace/goto.rs](../../crates/ui/src/views/workspace/goto.rs), [crates/ui/src/views/workspace/command_entries.rs](../../crates/ui/src/views/workspace/command_entries.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 48: N23 — Toggle viewer/read-only mode

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Protect an ordinary writable document temporarily.
  - Specification: Add a document-wide user viewer flag separate from OS permissions, malformed-input protection and snapshot read-only state. Show the mode and disable all editing entry points for every shared view, including transforms/replace/filters. Toggling off removes only the user flag; existing dirty text/undo remain. Default off for newly opened documents.
  - Acceptance: Toggle a dirty buffer shared across views; typing, paste, commands and replacement cannot mutate it while protected. Toggling off restores normal editing only when underlying protections permit it; saves/close decisions remain correct.
  - Start: [crates/editor/src/buffer/mod.rs](../../crates/editor/src/buffer/mod.rs), [crates/ui/src/views/editor/input.rs](../../crates/ui/src/views/editor/input.rs), [crates/ui/src/views/editor/commands.rs](../../crates/ui/src/views/editor/commands.rs), [crates/ui/src/views/workspace/menus.rs](../../crates/ui/src/views/workspace/menus.rs).

- [x] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [x] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [x] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 49: N25 — Mixed-line-ending inspection and preservation

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Inspect mixed endings and round-trip untouched line boundaries losslessly.
  - Specification: Detect LF/CRLF/CR boundaries before normalization and show counts/location navigation in the document inspector. Preserve each existing boundary until an explicit Convert Line Endings action; inserted boundaries use the preferred document ending. Track the boundary map through edits, undo/recovery and encoding changes. Old documents/session formats retain their existing single-ending defaults.
  - Acceptance: A mixed-ending fixture opens/saves with identical bytes; editing one line preserves other endings. Explicit conversion changes every boundary and is undoable. Test insert/delete/join/split, shared views, save-copy when N22 is available, recovery and no-final-newline cases. Detection alone does not complete this task.
  - Start: [crates/project/src/documents/io.rs](../../crates/project/src/documents/io.rs), [crates/editor/src/buffer/save.rs](../../crates/editor/src/buffer/save.rs), [crates/editor/src/buffer/changes.rs](../../crates/editor/src/buffer/changes.rs), [crates/workspace/src/sessions/mod.rs](../../crates/workspace/src/sessions/mod.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 50: N28 — Explorer location history and path breadcrumbs

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Navigate folder locations and document ancestors independently of editor jump history.
  - Specification: Add explorer Back/Forward/Parent actions with a 32-location per-window history, preserving expanded project roots. Add a lightweight active-file path breadcrumb menu for ancestors/children, reusing ordinary open/reveal actions. Validate moved/deleted locations; an ancestor operation must not silently replace the multi-root project.
  - Acceptance: Browse two nested directories and roots, go back/forward/parent, delete a recorded folder and open an ancestor child. Existing document jump history and expanded-tree state remain independent.
  - Start: [crates/project/src/explorer/mod.rs](../../crates/project/src/explorer/mod.rs), [crates/ui/src/panels/explorer/mod.rs](../../crates/ui/src/panels/explorer/mod.rs), [crates/ui/src/views/workspace/goto.rs](../../crates/ui/src/views/workspace/goto.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 51: N29 — Natural explorer order and sticky ancestor rows

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Offer natural filename order and optionally keep ancestor context visible.
  - Specification: Add lexical/natural explorer sorting with current lexical order as default. Natural order compares ASCII digit runs numerically without numeric overflow, preserving folders-first and deterministic case/leading-zero ties. Add a separate off-by-default sticky-ancestors view setting; pinned rows use real tree identity and the same open/context/focus actions.
  - Acceptance: Verify chapter-2 before chapter-10, huge digit runs, case/leading zeros and stable folder grouping. Scroll a deep tree and interact with pinned rows; keyboard focus, selection/reveal and screen coordinates stay consistent. Both subfeatures must be complete to close N29.
  - Start: [crates/project/src/explorer/mod.rs](../../crates/project/src/explorer/mod.rs), [crates/ui/src/panels/explorer/mod.rs](../../crates/ui/src/panels/explorer/mod.rs), [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 52: N30 — Natural/locale/length and capture-key sorting

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Extend existing sorting with precise additional comparators and keys.
  - Specification: Add natural integer-chunk, current-macOS-locale and grapheme-length modes; retain existing lexical/numeric behavior. Permit a validated regex capture index and Keep First Selected Line as Header. Use stable ordering for equal keys, keep unmatched keys after matched ones in source order and preserve line endings. Process nonoverlapping selected line blocks independently.
  - Acceptance: Sort v2-part9/v2-part10, locale fixtures, combining-text lengths and capture groups. Test headers, reverse order with stable ties, invalid capture, unmatched lines and undo. Existing pattern/f64 numeric sorting remains compatible.
  - Start: [crates/editor/src/editing/transform.rs](../../crates/editor/src/editing/transform.rs), [crates/editor/src/commands/transform.rs](../../crates/editor/src/commands/transform.rs), [crates/ui/src/views/workspace/transforms.rs](../../crates/ui/src/views/workspace/transforms.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 53: N31 — Snippet choice placeholders

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Choose values inside local snippet fields.
  - Specification: Parse TextMate-style choice fields, including escaped commas/pipes/backslashes. Insert the first choice by default and show an owned keyboard-accessible choice list when the field is active; confirm/cancel preserves normal field navigation. Choice edits update mirrors across active insertion carets and survive provider shutdown. Keep existing fields/transforms compatible.
  - Acceptance: Insert a multi-choice snippet, choose/cancel/Tab/Shift-Tab, edit a chosen value and undo. Test escaped delimiters, nested surrounding fields, mirrors, multicarets and UI closure on incompatible edits.
  - Start: [crates/editor/src/editing/snippet.rs](../../crates/editor/src/editing/snippet.rs), [crates/editor/src/commands/snippets.rs](../../crates/editor/src/commands/snippets.rs), [crates/ui/src/views/editor/snippets.rs](../../crates/ui/src/views/editor/snippets.rs), [crates/editor/src/buffer/tests/snippet_tests.rs](../../crates/editor/src/buffer/tests/snippet_tests.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 54: N32 — Date/time insertion and safe snippet variables

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Insert timestamps and expand time/clipboard/project values without shell evaluation.
  - Specification: Add Insert Date/Time with local or UTC selection and a validated strftime-style format supporting %Y, %m, %d, %H, %M, %S, %z, %:z and %% (unknown directives fail); default ISO date-time with numeric offset. Capture one instant for every caret in an expansion. Add CURRENT_YEAR, CURRENT_MONTH, CURRENT_DATE, CURRENT_HOUR, CURRENT_MINUTE, CURRENT_SECOND, CURRENT_SECONDS_UNIX, CLIPBOARD, WORKSPACE_NAME, WORKSPACE_FOLDER and RELATIVE_FILEPATH; use two-digit month/date/hour/minute/second, four-digit year and decimal Unix seconds. Resolve project context from the file’s containing root (first root for untitled); absent context/clipboard is empty and an outside-root relative filepath falls back to the filename. Reuse current snippet defaults/transforms; random/UUID and command evaluation are outside this task.
  - Acceptance: Use an injected clock/clipboard to prove deterministic local/UTC output, identical multicaret timestamps, escaping and missing-project/empty-clipboard behavior. Plain insertion and snippets undo normally; old variables stay compatible.
  - Start: [crates/editor/src/commands/snippets.rs](../../crates/editor/src/commands/snippets.rs), [crates/ui/src/views/editor/snippets.rs](../../crates/ui/src/views/editor/snippets.rs), [crates/ui/src/views/workspace/command_entries.rs](../../crates/ui/src/views/workspace/command_entries.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 55: N33 — Passive selection-occurrence highlighting

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Show other occurrences as a user selects text.
  - Specification: Add an off-by-default setting for a single nonempty single-line selection. Match literal case-sensitive occurrences within existing bounded search limits, on a cancellable/versioned worker. Keep this overlay separate from Find ranges and five manual highlight styles. Clear on empty/multiline selection or disabled state.
  - Acceptance: Select text with repeated Unicode matches, change/clear selection and deliver a delayed result after an edit. Only current occurrences render; Find state, text, selections and manual highlights are unchanged.
  - Start: [crates/ui/src/views/editor/mod.rs](../../crates/ui/src/views/editor/mod.rs), [crates/ui/src/views/editor/element.rs](../../crates/ui/src/views/editor/element.rs), [crates/editor/src/search/model.rs](../../crates/editor/src/search/model.rs), [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 56: N38 — Per-selection shell filters and predicates

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Process several selected ranges through explicit shell commands.
  - Specification: Extend shell filtering to independent nonempty inputs captured with document/version/range identity. Gather successful outputs before one atomic replacement; cancel/failure/stale state leaves text unchanged. A predicate mode retains ranges on exit 0, drops them on exit 1 and reports other execution failures. Preserve the existing one-caret whole-document filter.
  - Acceptance: Test ordered multicaret outputs, Unicode, stderr/exit failure, cancellation, edit during execution and predicate retention. No partial replacement or shell injection through input text occurs; user-authored commands remain explicit.
  - Start: [crates/editor/src/commands/filter.rs](../../crates/editor/src/commands/filter.rs), [crates/ui/src/views/workspace/customization.rs](../../crates/ui/src/views/workspace/customization.rs), [crates/ui/src/views/workspace/automation.rs](../../crates/ui/src/views/workspace/automation.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 57: N39 — Sync active terminal directory

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Change an existing terminal to the active file/project directory safely.
  - Specification: Expose explicit Sync Terminal Folder. Choose the active file parent, otherwise active project root; quote the supported shell directory command. Enable sending only when reliable prompt-ready state confirms the shell is awaiting a command with an empty input line; disable after any unconfirmed input and for foreground jobs/unsupported shells. Wait for cwd reporting rather than assuming success. No automatic sync on document switches.
  - Depends on: T02, T06.
  - Acceptance: With shell integration, verify spaces/quotes in paths, file/project fallback, partly typed input and foreground editor/long-running job rejection and PTY exit. A successful action produces a real directory change without executing in an unrelated application.
  - Start: [crates/ui/src/panels/terminal/mod.rs](../../crates/ui/src/panels/terminal/mod.rs), [crates/terminal/src/sessions/mod.rs](../../crates/terminal/src/sessions/mod.rs), [crates/terminal/src/shell/mod.rs](../../crates/terminal/src/shell/mod.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 58: N40 — Go to type definition/implementation

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Add the two lightweight LSP destinations missing from definition/reference navigation.
  - Specification: Expose capability-gated typeDefinition and implementation actions. Support Location/LocationLink arrays with negotiated position encoding; use existing single-target open/multi-target picker and side-open conventions. Reject stale server/document responses and report no result/unsupported method without moving the caret.
  - Acceptance: Fixture tests return one/multiple/link targets, Unicode positions, stale responses and missing capabilities. Verify actual commands and picker navigation; ordinary definition/references still work.
  - Start: [crates/syntax/src/languages/lsp/mod.rs](../../crates/syntax/src/languages/lsp/mod.rs), [crates/ui/src/views/workspace/language.rs](../../crates/ui/src/views/workspace/language.rs), [crates/ui/src/views/workspace/language_tests.rs](../../crates/ui/src/views/workspace/language_tests.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 59: N41 — Opt-in inline diagnostic messages

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Read the highest-priority diagnostic beside its source line.
  - Specification: Add an off-by-default setting with severity threshold. Render at most one current-version message per line, choosing severity then source order; elide long text and expose full detail through existing diagnostic interaction. Use an overlay, not document text, and keep the all-open-tab picker/underlines/gutter.
  - Acceptance: Verify error/warning priorities, multiple messages, long lines, soft wraps/folds, stale diagnostics and disabled mode. Copy/save/text coordinates and caret geometry remain unchanged.
  - Start: [crates/ui/src/views/editor/element.rs](../../crates/ui/src/views/editor/element.rs), [crates/ui/src/views/workspace/language.rs](../../crates/ui/src/views/workspace/language.rs), [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 60: N43 — Local Git tags

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: List, create and delete local tags.
  - Specification: Show tag name/type/target/message; create lightweight tags without a message or annotated tags with a message at HEAD/chosen commit. Validate names/revision identity, reject duplicate names without force, and delete only explicitly selected local tags. Reuse async Git/error handling; remote tag pushing and signed-tag UI are outside this task.
  - Acceptance: Temporary repositories verify lightweight/annotated creation, target resolution, duplicate/invalid names, unborn HEAD, stale selection and deletion. No operation contacts remotes or changes the checked-out branch/worktree.
  - Start: [crates/git/src/lib.rs](../../crates/git/src/lib.rs), [crates/git/src/operations/mod.rs](../../crates/git/src/operations/mod.rs), [crates/ui/src/panels/git/mod.rs](../../crates/ui/src/panels/git/mod.rs), [crates/git/src/tests.rs](../../crates/git/src/tests.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 61: U01 — Unicode/codepoint and literal-key insertion

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Expose dedicated character insertion without replacing macOS input services.
  - Specification: Provide Insert Unicode Character with a validated U+hex/scalar input and a command opening the native Character Viewer when available. Add Insert Literal Next Key, canceled by Escape, that bypasses ordinary editor keybinding interpretation for one text-producing input. Reuse working OS Services/IME paths; never insert surrogate codepoints or raw control shortcuts accidentally.
  - Acceptance: Verify U+1F600, combining characters, invalid/lone-surrogate values, literal-key cancellation and IME compatibility. Native Character Viewer is manually checked before duplicating any existing route.
  - Start: [crates/ui/src/platform/macos/mod.rs](../../crates/ui/src/platform/macos/mod.rs), [crates/ui/src/views/editor/input.rs](../../crates/ui/src/views/editor/input.rs), [crates/ui/src/views/workspace/tools.rs](../../crates/ui/src/views/workspace/tools.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 62: U02 — New file from template

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Create an untitled file prefilled by an existing local snippet.
  - Specification: Add New From Template with a searchable picker over local snippet definitions. Choose the snippet/language explicitly, create one dirty untitled document, expand it using normal variables/fields and use the selected language for syntax. Saving later uses normal Save As; do not invent a second template language or execute template code.
  - Acceptance: A template creates a new untitled document with the expected language/content/active fields. Cancel creates nothing; invalid snippets report an error without a partially initialized document. Existing snippet insertion continues working.
  - Start: [crates/ui/src/views/editor/snippets.rs](../../crates/ui/src/views/editor/snippets.rs), [crates/ui/src/views/workspace/files.rs](../../crates/ui/src/views/workspace/files.rs), [crates/ui/src/views/workspace/command_entries.rs](../../crates/ui/src/views/workspace/command_entries.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 63: U06 — Relative gutter line numbers

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Offer distance-from-caret numbering without a modal editor.
  - Specification: Add absolute/relative gutter numbering with absolute as default. In relative mode display the primary caret line as its absolute number and other physical lines by nonnegative distance; wrapped continuation rows do not gain fake line numbers. Keep current gutter visibility/distraction settings and independent view positions.
  - Acceptance: Verify lines above/below the primary caret, document ends, wraps/folds, multicarets and shared views. Switching mode changes only displayed numbers, not text, caret position or line-navigation arguments.
  - Start: [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs), [crates/ui/src/views/editor/element.rs](../../crates/ui/src/views/editor/element.rs), [crates/ui/src/views/workspace/menus.rs](../../crates/ui/src/views/workspace/menus.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.

## Task 64: U07 — Cursor-column highlighting

**Tier:** Optional. **Files:** Start owners/tests listed below, plus their connected callers and user documentation. **Interface:** extend the existing owner/API/catalog; preserve accepted earlier task interfaces and all global constraints.

- Requirement: Optionally emphasize caret display columns across visible rows.
  - Specification: Add an off-by-default view setting for a low-emphasis background band at each distinct caret display column, measured relative to each visible text row. Reuse existing column measurement for tabs/wide/combining characters. Preserve rulers, selections, diagnostics and horizontal scrolling; copy/export ignores the decoration.
  - Acceptance: Verify multiple columns, tabs, wide/combining text, wraps/folds, resizing and disabled mode. Mouse hit-testing and saved/copied bytes stay identical.
  - Start: [crates/config/src/settings/mod.rs](../../crates/config/src/settings/mod.rs), [crates/ui/src/views/editor/element.rs](../../crates/ui/src/views/editor/element.rs), [crates/editor/src/viewport/mod.rs](../../crates/editor/src/viewport/mod.rs).

- [ ] Inspect current behavior and write the short implementation contract; if all criteria already hold, identify the exact covering verification instead of rewriting code.
- [ ] Add a meaningful failing behavior regression for any missing branch, implement the smallest complete vertical slice, and update user documentation.
- [ ] Run covering tests and strict affected checks, self-review, record actual commands/results, freeze the task delta and obtain independent scoped review before checking the canonical entry.
