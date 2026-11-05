# Editor feature implementation record

Final acceptance, 2026-11-02: all 64 Recommended/Optional entries are verified in the
[single canonical checklist](editor-comparison-2026-10-29.md#final-selected-feature-verification--2026-11-02).
Final workspace 3,292 passed / zero failed / 29 established ignored, all 27 serial performance gates, strict workspace
Clippy, fmt, release build, both real-server smokes and native PDF checks passed. The final
integration review approves Spec and Quality with no required findings. Earlier dated pending
notes below are historical, superseded by the canonical command/results and native-limit
section. The Task19 rescue query and Task20 indicator assertions are resolved; native terminal
quit cleanup and multiline tag-row repairs are verified. Failure logs remain intact. No new
archives, hashes or duplicate checklist were produced; Deferred entries stay outside scope.

## U07 behavior contract

`cursor_column_highlight` is an off-by-default boolean file preference, resolved through
existing global/scoped/project settings for each view. View > Highlight Cursor Columns and
its palette/bindable toggle use the existing transient window display override policy;
Preferences publication clears the override. Invalid values retain ordinary settings warnings
and inherited defaults. Single-line fields do not acquire this editor decoration.

Each current selection head (including supported virtual carets) contributes its measured
horizontal display position relative to the text-row origin. Reuse current shaped rows,
tab expansion, wrap indent, folds and long-line clipping; measure offscreen caret rows with
the same bounded row path. Deduplicate columns before crossing only visible text rows and
clip bands to the text area, excluding filler rows. A faint one-space-wide background sits
below rulers, selections, Find, manual/passive highlights and diagnostics. No retained column
cache or worker can publish stale geometry; resize/font/wrap/scroll/edit frames derive it anew.
No document-wide paint scan, coordinate engine, source edit or hit target is introduced.

Acceptance: causal settings/command/paint regressions; actual shaped/painted bounds for
multiple and duplicate columns, selection heads, tabs/CJK/combining/emoji, virtual carets,
wrap/inline and line folds/fillers, resize/horizontal and vertical scroll, disabled fields,
shared views with independent carets; identical source bytes, copy/save/print input, hit tests,
selection/version/Undo and existing decoration layers. Worker runs focused affected config/
editor/UI tests, strict affected all-target Clippy, fmt/diff. ROOT owns independent scoped
review, canonical acceptance and ONE final workspace/release/global serial performance pass
covering all unchanged hot-path workloads/assertions/budgets, plus native/final checks.

U07 worker verification: causal missing-setting, unregistered-command and missing-paint REDs
retained. Final focused config1/editor1/UI9 pass; affected config88/editor859/UI1741 pass
(25 existing UI ignored), strict affected all-target Clippy, fmt/diff pass. Enabled 1,004-carets
frame shapes 3,959 chars across 981 offscreen rows and paints 63 deduplicated visible bands;
scrolled long Unicode rows reuse the visible shape and measure the other row once. Exact
commands, setup/lint failures and self-review are in
`.superpowers/sdd/editor-expanded-features-plan/task-64-report.md`. At worker handoff U07 was
unchecked. ROOT owned independent scoped review, canonical acceptance and the ONE final
workspace/release/global serial performance invocation with every original workload/budget,
plus final native/global checks; the worker ran no release/performance subset.

## U06 behavior contract

`line_number_mode` is a file preference with `absolute` (default) and `relative` values.
Existing global/scoped/project resolution feeds each view; View > Line Numbers and the
shared palette commands choose Absolute or Relative for the current window until settings
reload, following existing window display overrides. This changes only the gutter label.
The primary caret's physical source line keeps its one-based absolute number; every other
numbered physical line shows its unsigned source-line distance from that caret. Secondary
carets never become additional origins. Each shared view uses its own primary caret.

Use the existing first-row/fold/filler mapping: wrap continuations and filler rows have no
number, folded display lines retain their first physical source line, and distances count
hidden physical lines. Even a primary caret in the trailing source piece of an inline fold
uses its physical line, not the displayed header. Gutter visibility, distraction-free mode,
source/navigation/export coordinates, selections, scroll anchors, hit mapping and Undo stay
unchanged. Invalid settings use the existing visible settings warning/default behavior;
labels derive synchronously from the admitted document/view, with no cached origin or job.

Acceptance: causal settings/command regressions, actual shaped gutter labels above/below
the caret and at BOF/EOF, wrap/line-fold/inline-fold/filler/scroll cases, primary multicaret
and shared-view independence, menu/palette reachability, setting layering/reload, visibility,
unchanged text/selection/version/Undo and actual click/source-line navigation. Run affected
config/UI/editor tests, strict affected all-target Clippy, fmt/diff and existing exact serial
render/scroll/typing/fold gates without changing workloads/assertions/budgets. ROOT owns
independent scoped review, canonical acceptance, checkpoint63 and later final gates.

U06 worker verification: final focused config1/UI8 pass; affected config87/editor858/UI1728
pass (25 existing UI ignored). Strict affected all-target Clippy, fmt and diff checks pass.
All eight existing exact serial release typing/find/highlight/frame/restart/comparison/fold/
UI-keystroke gates pass unchanged: comparison2.571/10ms, folded scroll3.281/8ms and UI10M
keystroke44.283/300ms. Actual literal commands, retained causal/setup failures and scope limits
are in `.superpowers/sdd/editor-expanded-features-plan/task-63-report.md`. Independent scoped
review and canonical acceptance remain pending with ROOT; U07/final gates remain outstanding.

## N40 behavior contract

Add separate Go to Type Definition and Go to Implementation commands, each with a To Side
variant, to the existing language-server menu, palette and user-action registry. Hide these
entries until the active route advertises its corresponding capability; direct bound actions
still report unsupported methods. Use the current transport, negotiated UTF-8/16/32 positions,
Location/LocationLink parser, single-target opener and multiple-target Choices picker. These
semantic-only destinations never fall back to lexical definitions or references.

No result, invalid/error/unsupported replies and stale source/document/selection/server/window
contexts leave the document and caret alone. Preserve source guards through picker confirmation
and asynchronous target reads, including saved-read tickets and side-view creation. Explicit
server restart cancels pending new navigation before waiting for the Manager lock. Keep existing
definition/reference behavior and all assistance deadlines.

Acceptance uses real fixture subprocesses for both methods, capabilities, one/multiple/link
targets, Unicode paths and all position encodings, empty/error replies and delayed responses;
GPUI exercises the actual menu/palette actions, picker keyboard navigation, side opening,
edits/selection/focus/restart invalidation and ordinary definition/references. Run focused
syntax/UI/editor checks, strict affected Clippy, formatting and diff checks. This is a cold
explicit-request path; no rendering or keystroke performance workload changes.

The [original 2026-10-29 comparison](editor-comparison-2026-10-29-original.md) is historical evidence at
`31c83cb`. This worktree starts at that revision. Its missing audit was copied unchanged from
the sibling audit worktree. No `AGENTS.md` exists in either checkout or applicable ancestors.
Read ownership in [the architecture map](../architecture/overview.md) and run
[the verification guide](verification.md). Extend existing owners and widgets; no new runtime or
platform; add dependencies only when existing code cannot reasonably provide the capability,
with an explanation. No commits, publication, or real-remote Git testing.

The original 27-feature phase below is complete under its original acceptance criteria.
The follow-up request selects every Recommended and Optional entry in the expanded
[requirements](editor-comparison-2026-10-29.md), imported from the audit worktree on 2026-10-30.
That file currently contains 15 Recommended and 49 Optional entries (64 total), rather than
the request's count of 63. Its checkboxes are the active completion checklist. Prior completion
does not automatically satisfy changed specifications; each selected entry is rechecked.
The [50-editor research](editor-comparison-2026-10-29-research.md) is reference material only.

## Original phase progress (historical execution order)

- [x] T01 — Retained terminal search
- [x] T02 — OSC7 and output links (historical directory provenance)
- [x] T03 — Requested mouse reporting
- [x] T05 — Shell, arguments and retention settings
- [x] T04 — Terminal sessions per window
- [x] T07 — Send selection / execute selection
- [x] T06 — Launch-time shell integration and command boundaries
- [x] L01 — LSP snippet completions
- [x] L02 — Signature help
- [x] L03 — Range formatting
- [x] L04 — Format on save
- [x] L05 — Runtime language registry routing
- [x] E01 — Syntax folding with indentation fallback
- [x] E02 — Persist folds
- [x] E03 — Unicode warnings
- [x] E04 — Overtype mode
- [x] E05 — Virtual space
- [x] E06 — Declarative modelines
- [x] F01 — Legacy encodings / UTF-32
- [x] F02 — Printing / PDF
- [x] F03 — Log following
- [x] F04 — File autosave
- [x] F07 — Discarded untitled rescue
- [x] G04 — Amend / commit assistance
- [x] G02 — Stashes
- [x] G03 — Worktrees
- [x] G01 — Explicit remote operations

For each ID: specify behavior here before editing, add regression tests, implement the backend
and reachable UI, update the user guide, verify, then advance. Preserve all existing features.
F05, F06, E07–E09 and G05 remain excluded.

Checked IDs passed their implementation, focused verification and scoped review. Historical
handoff notes below may describe earlier pending gates; later acceptance entries supersede
them. All 27 feature gates and the final correction review are complete. The full workspace,
release build and all 27 release performance gates passed. Final verification evidence and
documented limitations appear at the end of this record. Work remains uncommitted.

## T01 behavior specification

Default: search closed; existing PTY, retention and key behavior unchanged. Terminal Find
(button and Cmd-F) opens a standard editor field. Search is literal and case sensitive,
including soft-wrapped rows and Unicode; Previous/Next wrap over retained matches. Enter,
Shift-Enter and Cmd-G / Cmd-Shift-G navigate. Escape/Close clears highlights and returns to
live output. No terminal input is emitted by search controls.

Search runs against snapshots off the UI thread; output continues draining. Refresh on output
is throttled. Replaced queries and closed searches cancel work; late results must not overwrite
a newer query, terminal size, or primary/alternate screen. While output changes rapidly, a
completed snapshot remains searchable until the next refresh. Bounds are the existing retained
screen. Match locations use terminal cells, including wide and combining characters.

Acceptance: real PTY and GPUI input tests; matching retained/soft-wrapped Unicode output,
navigation/wrap/no matches, query replacement/closing and ongoing output; bounded release
search performance plus existing rendering/input gates. Full final checks as in verification.md.

## T02 behavior specification

Cmd-click a web URL or a local path in terminal output. Ordinary dragging still selects text.
Accept http/https URLs, absolute/relative paths, quoted paths with spaces, file URLs and optional
`:line[:column]` / `(line,column)` locations. The UI uses the existing file-open/jump path so
shared documents and asynchronous large-file opens work. Missing paths report an error.
No output link is executed as a shell command. OSC7 accepts only absolute local file URLs,
decodes percent escapes strictly and rejects NUL/remote hosts; malformed sequences leave the
last valid cwd intact. Before the first valid OSC7 use the launch directory. No user shell
configuration is modified. Search snapshots retain their cwd for relative link resolution.

Acceptance: split OSC7 sequences and both terminators, encoded Unicode/spaces, malformed and
remote URLs, changing cwd, path suffixes, quoted paths, URL punctuation, soft wraps, wide cells,
and UI file opening at a location. Shell integration that emits OSC7 by default belongs to T06.

## Verification evidence

- T01 backend: 3 regression tests passed; GPUI terminal: 3 tests passed, including a real PTY
  with active search, query replacement and dismissal. Full workspace/performance gates running.

Terminal checks so far: strict workspace Clippy passed before the review fixes; 9 focused GPUI
tests and 21 terminal tests passed before the last control-string/startup regression additions.
The retained search release gate passed again at 11.9 ms (250 ms budget).

The first full workspace run passed. The full release performance run had 22 UI gates pass
and two fail: `bench_find` (symbol lookup 3152 ms / 3000 ms budget, 3049 ms when rerun idle),
and `bench_scroll_a_100k_line_file_with_1000_folds` (fixture generated 941 folds / minimum 950).
The editor/parser/benchmark source is unchanged by the terminal features; these failures are
not yet resolved. No thresholds or coverage were weakened. Final checks must be repeated.

## T03 behavior specification

Default remains text selection and scrollback. When a child requests X10, VT200, button-motion
or any-motion reporting, forward the events that mode asks for, using default, UTF-8 or SGR
coordinates. Report 1-based cells with floor hit testing, clamp drags to screen cells and
suppress duplicate motion in one cell. Shift-drag always selects text for its whole gesture;
search is also a selection surface. Mouse press focuses the terminal. Do not leak events from
the search field, outside the terminal, or across focus loss. Wheel reporting takes precedence
over scrollback only while requested; default-byte coordinates that cannot be encoded are
refused rather than wrapped. Keep existing alternate-screen and key behavior.

Acceptance: protocol bytes for each mode/encoding, modifiers, press/release/motion/wheel,
disabled mode and overflow; UI focus, floor coordinates, Shift gesture and search override.

## T05 behavior specification

Global `terminal` preferences contain `shell` (null uses SHELL, then /bin/zsh), `arguments`
(null uses ["-l", "-i"]) and `scrollback` (default 10000, valid 0–100000). Arguments are
passed literally, without shell splitting or substitution. Settings affect newly created
sessions only. Invalid values warn through the existing preferences parser and retain defaults;
an unavailable executable produces the existing terminal startup error. No dependency is added.
Acceptance: old preferences/default compatibility, explicit arguments, zero/bounded retention,
NUL/invalid settings, literal argument construction and actual configured PTY launch.

## T04 behavior specification

One terminal tab is created on first show, as before. New Terminal creates and focuses another
independent PTY, using current launch preferences and the active terminal cwd (otherwise the
existing editor/project/home fallback). A tab strip and New/Next/Previous/Close commands select
sessions. Reuse workspace Groups ordering/selection/removal; hiding or changing panels retains
all sessions. Close ends only the active session, selecting its neighbour; close the last hides
the panel. Switching projects/window teardown ends every session, and quitting restores none.
No terminal splits, process persistence or multiplexer is added.
Acceptance: two sessions keep independent text/cwd, switching and hiding preserve them, closing
one releases it and keeps its neighbour, closing/project teardown releases all; commands and tabs
remain reachable with no editor document.

## T07 behavior specification

Two explicit Tools/palette commands send nonempty editor selections, joined by newlines, to
the selected existing terminal tab (create one if needed). Send Selection pastes without an
extra Enter; Execute Selection appends one Enter outside the paste framing. Neither edits the
document, so read-only views work. Reveal/focus the destination. No selection is a no-op with
a status message. Preserve paste normalization and size/backpressure/error reporting. For Send,
multiline text is refused if the application has not enabled bracketed paste, because the raw
newlines could execute commands; Execute is the explicit alternative. Submit paste and Enter
in one queue item so a failed send cannot enqueue a lone Enter.
Acceptance: exact bytes for single/multiline and bracketed paste, distinct newline behavior,
size bounds and real PTY delivery; read-only/multiple selections and existing session reuse.

## T06 behavior specification

Recognize OSC133 A/B/C/D prompt, input, execution and completion markers across arbitrary PTY
chunks. Keep a bounded in-memory list of retained command text/output and exit status. Previous/
Next Command review retained output while the PTY runs; Copy Command Output copies that record;
Live returns to the current screen. Output records are rendered through the existing emulator,
with at most 32 records and 1 MiB total text, in addition to current screen retention. Explicitly
label this retained output, since scrollback limits may have removed earlier output.

For interactive zsh, install hooks using a private temporary ZDOTDIR wrapper, forwarding the
user's startup files and restoring ZDOTDIR. No user shell config is written. Custom shells can
emit OSC133 themselves; unsupported shells remain ordinary terminals. Preserve -l/-i and shell
configuration/signals. Drop temporary files with the session. Invalid/oversized OSC payloads,
missing markers and resets must not grow memory without bound or invent successful exits.
Acceptance: split markers, bounded retention, Unicode output/status, real zsh startup/config
forwarding, cwd changes via OSC7, command review/copy/live, malformed sequences and cleanup.

## L01 behavior specification

Explicit LSP completion accepts snippet-format items and completion-list defaults. Parse before
mutating, expand at the primary completion range through the editor's existing variable/indent/
tabstop/mirror machinery, and apply additional edits plus expansion in one undo step. Fields
follow earlier additional edits. Other views share the text and map their independent carets.
Keep existing primary-caret-only server completion policy; local multicaret snippets stay as
before. LSP choices insert/select their first choice for ordinary field editing; local snippet
syntax remains unchanged. Invalid/overlapping snippets or read-only documents refuse all edits.
Existing request epochs/version/caret checks reject cancelled/stale resolve responses. Advertise
snippet support only after backend, UI and fixture tests pass. No new dependency.
Acceptance: Unicode fields/mirrors, tab navigation, additional edits before/after main insertion,
one-step undo/redo, shared views, read-only/stale/cancellation and actual fixture-server results.

## L02 behavior specification

Goto > Language Server / palette Signature Help explicitly requests the signature at the caret.
Show the active signature and emphasize the active parameter in a caret popup; accept label
strings and protocol UTF-16 offset pairs without splitting Unicode. No automatic requests are
added. Empty/unsupported responses report status. Close on Escape, edits, caret moves, scroll,
resize, focus loss or a superseding navigation; late replies use the existing generation and
document/caret protections. Do not change document text or undo state. Acceptance: parameter
selection and Unicode ranges, invalid responses, fixture request/capabilities, popup and dismissal.

## L03 behavior specification

Format Selection is a separate Goto > Language Server / palette command. It formats the primary
nonempty selection using textDocument/rangeFormatting when advertised by the server; empty
selections and unsupported servers report status. Keep Format Document and its shortcut.
Use the existing document options, protocol position conversions, atomic edit validation,
read-only refusal, stale source checks and undo path. Formatting edits remain unsaved.
Acceptance: fixture request/range/options for Unicode selection, one-step undo, unchanged
document formatting, unsupported/read-only documents and stale response protection.

## Completed language slice verification

L01–L03 received an independent spec/quality review. L02 caret movement, pending resize and
superseding-navigation findings were fixed; a scoped review approved the fixes. Resize protection
compares the request and response viewport sizes (returning to exactly the same size is equivalent
context). Four signature-related GPUI tests passed, including new lifecycle regressions. The
language UI suite passed 58 tests before the final two range-formatting assertions/refusal cases.
Strict workspace Clippy passed after the L02 fixes. Final workspace gates remain pending.

Remaining features execute sequentially under [the implementation plan](editor-features-plan.md).
The no-commit instruction takes precedence over skill commit steps; task review uses snapshots
of uncommitted files. The original audit remains unchanged.

## L04 behavior specification

`format_on_save` is a boolean file preference, off by default, with the existing global,
project and path/language override resolution. Save, Save All and save-before-close use it;
Save As uses the destination's preference and formats a named source with its current server.
An untitled source has no LSP identity yet, so its first Save As only runs ordinary cleanup.
Save preference choices are captured when requested, matching cleanup; queued source snapshots
are taken at their turn. Manual document/selection formatting remains independent of this setting.

Reserve the document's save slot before asynchronous formatting. Queued saves prepare the
current document only when their turn starts; the document stays saving through preparation,
cleanup and the existing leased write, so shared views and close/quit use the same waiters.
Validate edits with the existing LSP staging/conversion rules, then apply them through the
normal undoable shared-document edit path only if source identity/version, read-only state and
server generation still match. Caret changes or switching tabs do not cancel a document save.
A source edit, server restart/disable, unavailable/unsupported server, invalid response or
request failure skips formatting and reports status; it still saves the latest document text.
No automatic retry. Existing LSP timeout bounds apply. Read-only sources are never formatted;
ordinary save refusal and Save As copying remain unchanged. Cleanup runs after formatting.

Acceptance: fixture-server Unicode formatting and undo/redo/shared views; default-off and
scoped-settings compatibility; source edits and server invalidation while formatting waits;
queued saves/Save As; close and quit waiting through preparation and a held disk write;
read-only, unsupported/error/malformed responses and cleanup ordering. No dependencies added.

## L04 verification

Implemented with no dependencies. Twelve fixture-driven save-format tests passed, including
stale edits from another view, latest-text fallback, queued saves, Save All, Save As/read-only
copies, undo/redo, active-tab changes, cancellation, formatter/disk errors, cleanup order and
close/quit waiting across preparation and disk writing. The existing save suite passed 27 tests;
the language suite passed 70 tests before the final latest-text regression was added. Config
passed all 53 tests. Focused strict Clippy passed for UI/config and their dependencies.

GPUI's deterministic test executor cannot interleave UI edits with a blocking fixture-server
wait. A test-only response pause reuses the save gate utility after real response staging and
before application; production behavior is unchanged. Controller review and final workspace,
release and performance gates remain pending. Detailed commands and design handoff are in the
local `.superpowers/sdd/editor-features-plan/task-1-report.md`.

## L05 behavior specification (before implementation)

- Resolve LSP identity from the current validated language registry before the bundled fallback.
  User manifests may add file names/extensions/content or first-line patterns, protocol IDs,
  server families, commands, initialization options and root markers without recompiling.
  A detected manifest without `lsp` has no server. A same-ID manifest replaces that bundled
  language's routing whole; unrelated bundled IDs and shared server families keep their defaults.
- Existing `lsp_enabled`, `language_servers` overrides (including an empty command), restart,
  polling, all LSP commands, Goto Definition and format-on-save use the same resolved identity.
  Preferences override manifest server defaults. For shared families, the first manifest by ID supplying each default wins;
  use Preferences to override that family explicitly. Explicit project roots still win;
  otherwise search ancestors using exact/leading-wildcard root markers and `.git`.
- Reload Customizations rereads manifests using the existing loader/error alert. New routing
  applies to already open files, invalidates old requests/decorations/formatting and restarts
  affected server state. Malformed folders are isolated; bundled fallbacks remain available.
  Parser/highlighter choices and installed grammar overrides stay independent of LSP routing.
- Acceptance: fixture subprocess sees a new language ID, Unicode source and correct root;
  completion/definition and save formatting reach it; settings override/disable and reload work;
  old replies are discarded; existing ID/family/root/parser/grammar regressions stay green.
  No new dependencies, general plugin runtime or WASM execution are introduced.

L05 preference/explicit syntax precedence clarification (before these edits): matching manifests
supply `name`/`scope` for scoped file settings, including format-on-save. An explicit View >
Syntax choice instead supplies its chosen grammar name/scope and named server identity; reload
retains that explicit choice. Language choices remain per-view. Shared-document LSP snapshots
prioritize the requesting view (the active view for polls); only views with that same language
choice receive decorations. Save preparation prioritizes its source view. Installed grammar
rendering and the existing path-change redetection behavior stay intact.


## L05 implementation and focused verification

Implemented registry-first runtime protocol IDs, owned server-family keys, manifest command/
initialization defaults below Preferences, exact/wildcard root markers, all LSP commands/Goto
Definition and format-on-save routing. Matching manifest name/scope drive file overrides;
explicit Syntax choices drive their own preferences and named server identity. Reload clears
old decorations and invalidates old request/save-format snapshots; unchanged command/options
still rebuild sessions when the language generation changes. Syntax/parser installation and
bundled family IDs remain intact. No dependencies or runtime were added.

Verified: `cargo test -p orlo-ui --locked views::workspace::language::tests -- --nocapture`
passed 75 tests; preferences 9, customization 11 and goto 3 passed. `cargo test -p orlo-syntax
--locked` passed 161 tests, 3 existing ignored; includes registry validation/reload and bundled
parser/installed-grammar regressions. The initial syntax run exposed six transport tests whose
fixture still required `snippetSupport=false` after L01; changing its exact assertion to `true`
restored all six. Focused strict Clippy, touched-file rustfmt check and diff check passed.
Full final workspace/release/performance gates and controller review remain pending.

Review concern: one document still has one LSP route. Conflicting per-view Syntax choices
prioritize the requester/active polling view; other windows may switch that route on polling.
Source identity/choice/registry/version/epoch guards discard stale results, and save preparation
prioritizes its own source. This limitation is documented; no transport-ownership redesign was
introduced. Test-support builds isolate registry reloads per GPUI test thread, while production
retains the shared process registry. No live native/real-server smoke was performed for L05.
See `.superpowers/sdd/editor-features-plan/task-2-report.md` for exact commands and decisions.

### L05 review round 1 behavior amendment (before fixes)

Manual LSP edit application must validate the requesting view's registry/language freshness,
then validate every affected shared view by document ID/path/version/read-only state; differing
Syntax choices on true shared views do not invalidate a formatting/rename/action edit. Changed
sources and independent duplicate buffers at one path still reject the complete edit batch.

A same-family protocol-ID transition must isolate all prior-client diagnostics, including
unversioned publications. Restart the affected existing family/root session before syncing its
new identity; ordinary text edits and unchanged identities retain their session. Regressions
must observe nonempty manual formatting in conflicting shared views with undo, changed/duplicate
rejection, and delayed old-identity versioned/unversioned fixture diagnostics after a switch.

L05 review-round polling consequence (before its fix): restarting conflicting identities must
not spawn a server on every alternating window poll. Ordinary polls choose the lowest-ID open
view of each shared document across all windows, independent of active tabs/calling window.
Explicit requests and source-save preparation still prioritize their own view; the next poll
may restore the stable owner once. Closing the owner selects the next open view. A focused
two-window C/C++ fixture test will check process identity across repeated polls and verify
explicit and format-on-save requests retain their source choice.


L05 review round 1: separated source registry/choice freshness from affected shared-buffer
identity; nonempty manual formatting now reaches differently styled shared views and undo,
while changed/duplicate/read-only and changed-source rejection remain covered. Same-family
protocol changes retire the affected client/session, isolating delayed versioned/unversioned
old diagnostics. Stable lowest-editor-ID polling ownership avoids repeated process restarts
across conflicting windows; explicit commands and saves keep source priority. Five added
regressions cover these boundaries. Final focused evidence is recorded in task-2-report.md;
controller re-review and full final gates remain pending.

Round-1 final evidence: all 9 runtime tests passed, five selected existing edit/session regressions
passed, strict UI Clippy passed, and touched Rust formatting/Python AST/diff checks passed.
Stable polling ownership supersedes the initial active-view polling policy and resolves the
previous cross-window polling concern above. Explicit commands/saves can still change the one
shared route, after which a poll can restore the stable owner once. Both Important findings are
ready for controller re-review; no broader gates are claimed by this focused fix round.

### T06 launch-argument follow-up specification

Automatic zsh wrappers run only with the supported `-i`, `-l`, `-li`, and `-il` startup flags, including an interactive flag. Other configured argument forms run unchanged without automatic hooks, so startup-disabling options cannot leave private `ZDOTDIR` state in the shell. OSC7/133 emitted by any shell continues to work. Default arguments retain automatic integration. Acceptance: real interactive zsh launches with named/combined no-RCS options preserve the original `ZDOTDIR`; default launch/hooks and cleanup regressions remain passing.

T06 follow-up verified: the real-PTY no-RCS launch test failed before the fix with the private directory in `ZDOTDIR`, then passed for named and combined flags. `cargo test -p orlo-terminal --locked`: 25 passed, 1 existing ignored release gate. Strict terminal all-target Clippy and touched-file rustfmt passed. Configured arguments remain literal; default launch/hook/cleanup regressions passed.

## E01 behavior specification (before implementation)

Existing Fold, Fold Level and gutter controls use a shared index from the current incremental
Tree-sitter highlight snapshot. Multiline brace/bracket containers, block comments, JSX/TSX
markup and supported keyword/body blocks are discovered without relying on indentation.
Keep the header line visible and hide through the syntax node end; suffix text after the node
stays visible. One outermost region per header is offered; nested headers remain independent.
Fold Level uses syntax nesting (outermost = 1) when that index exists. Exact inline selections
and existing multiline selected-line behavior remain unchanged, as does the view-owned fold
model, including edits/shared views/read-only and undo/redo behavior.

No new parser, dependency, setting or persistence format. Unsupported/over-limit languages,
pending/cancelled parsing and stale snapshots use existing indentation discovery. A current
empty syntax index is authoritative. Build discovery once on the existing highlighting worker,
never during paint; validate rope identity before exposing it and retain existing job revision
checks. Commands do not wait for a parse. Acceptance: unindented braces/comments/JSX, Unicode,
nesting, suffix preservation, incremental edits and stale/cancelled jobs, fallback, exact manual
folds, read-only/shared views and existing rendering/fold release gates. Correct the known
1000-fold benchmark fixture with nonempty inline ranges without changing budgets/minimums.

## E01 implementation and verification

Implemented syntax folding through existing worker snapshots and the view-owned fold model.
Commands and gutter share the current candidate index; exact manual folds remain unchanged.
Header text stays visible, the closing delimiter is hidden, and following code stays visible.
Syntax levels use structural nesting. Unsupported/installed grammars, standalone HTML, pending
or stale parsing and over-limit files retain indentation. JSX/TSX markup is supported. No
setting, dependency or persistence format was added; E02 remains separate.

Verified: full syntax suite passed 163 tests (3 existing ignored) before final test expansion;
final focused syntax/editor run passed 7 syntax tests and the new index-boundary test. Editor
rendering passed 19 existing tests; all 160 editor UI tests passed after final changes, including
three new syntax-fold tests. Strict all-target Clippy passed for UI/editor/syntax. Touched-file
rustfmt and diff check passed. Parser coverage includes all 17 compiled language variants,
Unicode/CRLF, comments/markup, nested regions, cancellation, stale jobs and incremental edits.
The HCL delimiter test failed first, then passed with its explicit grammar seam.

All 10 release rendering gates passed serially on final source. Scrolling 100k lines with 1000
folds took 2.874 ms (8 ms budget); comparison scrolling took 1.441 ms (10 ms); background
highlighting took 26.467 ms (120 ms). All existing timing budgets and the fold minimum of 950
remain unchanged. The earlier fold-fixture failure is resolved by selecting nonempty inline
ranges in each block; the current fixture now produces all 1000 intended folds. The historical
`bench_find` failure above is outside these rendering gates and is not claimed resolved here.

The index is rebuilt on the worker after a changed parse and reused across viewports; no
parsing/discovery runs during paint. Standalone HTML still uses indentation, and recognized
syntax constructs are intentionally limited to the documented structural forms. No native
manual smoke was run. Controller review and final workspace/release gates remain pending.
Exact commands, red/green evidence and limits: `.superpowers/sdd/editor-features-plan/task-3-report.md`.

## E02 behavior specification (before implementation)

Default: remember active collapsed ranges automatically, per view, through the existing private
session/project-workspace serializer. No new setting or command. Persist exact character offsets,
including nested and inline selection folds; syntax candidate indexes are never persisted. Fold
changes schedule the existing session autosave. Restoring folds never edits text or undo history.

Validation: optional version-1 fold records contain the decoded rope's UTF-8 byte length and
streaming SHA-256 fingerprint, plus canonical (start, end) char pairs. Compare the entire current
rope before applying; reject the whole fold record on unknown version, stale fingerprint/length,
empty/reversed/out-of-range range, duplicate, noncanonical ordering or partial overlap. Nested
and touching ranges are valid. Missing/malformed records restore expanded views while other
view/session fields remain usable. Encoding/BOM/line-ending-only changes with identical decoded
text retain valid offsets. Saved selections inside hidden text move to the placeholder boundary
through the existing fold application path.

Bounds: at most 4096 ranges per view and 16 MiB of decoded UTF-8 text for persistence; exceeding
either drops persistence for that view, without limiting live folding. The text cap matches the
syntax parser cap and bounds synchronous restore/capture hashing latency. Hash rope chunks without
flattening; cache the four most recent immutable rope identities (at most 64 MiB retained text),
so shared/unchanged captures do not rescan and paint never hashes. Recompute after text changes.
No digest is trusted from a background read: validate against the actual document attached at
restore, including a shared document edited while the read waited. Existing restore generation,
write leases and open cancellation continue to guard asynchronous loads.

Close/reopen: retain the latest 32 closed named views in each window's existing session, newest
last, with paths and fold metadata but no unsaved text. Ordinary Open/Recent and Reopen Closed File restore
folds from the newest closed record for that path only when text still matches; selecting an
already-open tab does not alter its folds. Reopen Closed File consumes that history entry;
ordinary Open retains it until a newer close. A close after Unfold All records expanded state.
Independent open views restore their own explicit state, overriding closed history. Untitled
closed buffers are excluded; discarded dirty text is never recovered by this feature.

Acceptance: old/malformed schemas, unknown versions, stale same-size edits, invalid/crossing/
oversized ranges and text cap; Unicode, nesting, exact inline and node-ended folds; independent
shared views, unchanged bytes/undo and read-only views; close/reopen and serialized restart;
late restore into edited shared text, cancellation and existing ordered-save race regressions.

E02 close-history clarification before verification: only the newest 32 closed records retain
fold metadata and persist; older in-memory entries retain their original path-only reopen
behavior. Reopen Closed consumes one record and creates a distinct view even when another view
of the file is open, preserving two independently closed views. Ordinary Open selects an existing
tab unchanged, or uses newest-per-path history when creating one. Closed records contain only
paths and fold metadata, never recovery text or rope snapshots.

E02 cache refinement: use one fingerprint cache per live editor view, keyed by immutable rope
identity, rather than the initially proposed four-entry global LRU. This prevents repeated
whole-text scans when more than four folded files are open. Each cache retains at most one
16 MiB rope snapshot, sharing its unchanged storage with the document; an empty/over-limit fold
capture releases it, and closing the editor drops it. Shared views each hash once after text
changes; unchanged captures and changes to fold ranges alone reuse their cached text digest.

E02 close-record policy (corrected during review round 1): closed records contain only paths
and fold metadata in a default-filled `ViewState`; they do not use `ViewState::capture` or
retain document/view preferences. Explicit reopen must apply only validated folds, preserving
fresh file preparation, current EditorConfig and already-shared document overrides/decoding.
Small files use ordinary decoding; deferred closed reads do not take stale encoding hints from
closed metadata. Full session restoration still applies its complete saved `ViewState`.
Synchronous small-file reopen, missing-file handling and large-file prompts remain available;
leased/large reads reuse the generation-guarded restore path. Capture omits folds while a shared
view's revision is behind the document, preventing old offsets receiving new-text fingerprints.
Acceptance for review round 1: failing-before-fix/passing-after-fix last-view close/reopen with
current EditorConfig indent_size=2 and tab_width=8, both immediate and write-lease-deferred,
with and without folds; preserve a shared sibling's explicit document settings and folds.



## E02 implementation and verification

Implemented versioned optional active-fold state in the existing private session and project
workspace serializers, with exact canonical character ranges and streaming SHA-256 validation.
All fold commands remain reachable; their view-change events schedule the existing autosave.
Per-editor rope-identity caches avoid hashing unchanged captures and reject capture before a
shared view's fold offsets have caught up. Restore validates the actual attached document and
uses the existing refold/selection path, preserving nested/inline/node-ended ranges without
editing bytes or adding undo entries. No syntax candidate cache is serialized.

Closed-view metadata preserves paths and folds for the latest 32 named views, with no recovery text. Older
in-memory history remains path-only. Explicit Reopen creates an independent shared view;
ordinary Open leaves existing views alone and uses newest matching history only for a new view.
Newest expanded history suppresses older collapsed records. Background reopen also completes
when a normal Open is already reading the path; current-generation, file-write and shared-text
guards remain in force. The 4096-range/16 MiB limits affect persistence only.

Initial implementation verification: all 103 workspace-crate tests passed. The broader session selector passed 50 UI
and 47 workspace tests before the final concurrent-open refinement; the final UI sessions owner
suite passed 14 tests, including all 8 new UI regressions. The final fold selector passed 88 UI
tests (1 existing release benchmark ignored), and editor rendering passed 20 tests. Strict
all-target Clippy passed for UI/workspace/editor. Touched-file rustfmt and diff checks passed.
A negative control disabling fingerprint comparison failed the same-size stale-text regression
as expected; restored validation passed the later session suite. Bounds coverage includes the
100k-line/1000-fold workload, the 16 MiB boundary, excess ranges and cache reuse.

`sha2` was already a locked workspace dependency; this slice adds it to the workspace crate
for stable full-text fingerprints, introducing no new package. User guide, README and architecture
map describe restoration and its limits. No E03+ behavior, commits, publication or agents.
Full workspace/release/performance gates remain with the controller. Detailed commands, resolved
iteration failures and limits: `.superpowers/sdd/editor-features-plan/task-4-report.md`.


### E02 review round 1 verification

Corrected explicit Reopen to apply only validated folds through a dedicated closed-view seam,
retaining current file preparation/EditorConfig or the attached shared document's overrides.
Closed records do not supply decoding hints. Full session restoration is unchanged. Corrected
the earlier inaccurate full-`ViewState::capture` claim in place; actual closed records store
path/folds/defaults and never document/view preferences or recovery text.

Before the fix, new synchronous and write-lease-deferred last-view reopen tests both failed:
indent_size became None/effective 8 instead of current EditorConfig Some(2)/effective 2 with
tab_width 8. The shared-override baseline passed. After the fix all three passed; final extended
coverage also rejects stale closed-record encoding/indent/wrap fields, checks both empty and
nonempty folds, and preserves shared encoding/BOM/line-ending/indent overrides and sibling folds.
Final owner suite: 17 UI session tests passed, including all 11 fold-persistence regressions;
9 preference tests and 3 encoding tests passed. Strict UI all-target Clippy, touched rustfmt and
diff checks passed. Commands and actual results are appended to `task-4-report.md`.


## E03 behavior specification (before implementation)

`unicode_warnings` defaults to true. `unicode_warning_characters` is a replacement map from
single Unicode scalar strings (JSON escapes accepted) to `{ "glyph": "!", "description": "…" }`.
Both are file preferences: global, matching global language/path/scope overrides, project layer,
then matching project overrides; later supplied maps replace the whole list, not individual
entries. An empty map disables all marks. Missing keys retain defaults; invalid fields use the
existing preferences warning/default fallback. No dependency or document/session schema change.

The default list is U+0000 (NUL), U+00A0 (NBSP), U+00AD (soft hyphen), U+061C (Arabic letter
mark), U+200B (zero width space), U+200E/F (direction marks), U+2028/29 (line/paragraph
separators), U+202A–E (direction embeddings/overrides/pop), U+202F (narrow NBSP), U+2060 (word
joiner), U+2066–9 (direction isolates/pop), and U+FEFF (embedded BOM/zero width NBSP).
Every default uses `!` and a specific description. Ordinary spaces/tabs/newlines, combining
marks, ZWJ/ZWNJ, variation selectors, emoji tags and other language text are excluded by default;
explicit list overrides may opt any scalar in. This is a visibility aid, not BiDi validation.

Draw a small amber glyph overlay at the existing shaped scalar position (minimum one-space
marker area for zero-width scalars); never replace text or add advance width. Only visit the
already clipped visible RowText pieces; hidden folded text and filler rows do not warn. Suppress
marks overlapping current IME composition until committed. Single-line input widgets do not
warn. Character Info (Edit menu and palette) appends configured descriptions for the cluster
at the primary caret even when marks are disabled, labeling that state. Preferences remains
the reachable persistent configuration editor. No asynchronous warning jobs or stale caches:
recompute from current visible text/configuration each frame, and inspect current text on demand.

Acceptance: defaults/invalid settings/serialization and scoped/project replacement; warning and
legitimate sequences; actual GPUI marked glyph placement with unchanged shaped rows, selection,
caret/wrap/mouse geometry and document bytes; marked-text suppression and IME commit; folded,
clipped huge-line, read-only and shared-view behavior; undo/redo and live settings changes;
Character Info descriptions through its existing action. Run focused strict checks and the
serial release rendering gates; controller retains final workspace/all-performance verification.

E03 validation clarification: a configured glyph must be exactly one scalar and cannot be a
control or whitespace character. Keys may be any scalar, but line terminators omitted from
shaped rows receive descriptions in Character Info only. Custom invisible/combining glyphs may
not be visibly distinct; use punctuation such as `!` or `?`. No general Unicode-name database
or confusable detection is introduced.


## E03 implementation and verification

Implemented two file preferences (`unicode_warnings`, default true, and the replacement map
`unicode_warning_characters`) with 21 conservative defaults, per-scalar glyphs/descriptions,
normal global/project/path/language/scope resolution and existing preferences persistence.
Character Info in Edit and the palette reports configured descriptions even with marks disabled.
No new dependency, action system, document/session schema or measurement change.

Warnings paint over existing shaped positions using clipped non-fold RowText pieces. Ordinary
Unicode/emoji/combining sequences are excluded by default; explicit overrides still preserve
shaping. IME-marked ranges, hidden folds, filler rows and single-line widgets are not annotated.
A new regression exposed the missing redraw after `unmark_text`; notifying that existing input
boundary fixes stale marks and composition underlines without changing the input/edit model.
The unmark test failed before the fix, then passed.

Verified: 2 config and 6 GPUI E03 tests; all 55 config tests; all 164 editor UI tests; 10 workspace
preference tests, 2 Character Info tests, 7 composition tests and the existing multicaret dead-key
test. Geometry tests compare shaped rows/positions, wrapping, caret/selection state and hit testing
before and after enabling warnings. Huge-line/fold tests bound visited characters; shared/read-only,
live settings, undo/redo, composition commit/cancel/unmark and serialization are covered.
Strict config/UI all-target Clippy, touched Rust formatting and diff checks passed.

All 10 existing release rendering gates passed serially on frozen source. Scrolling 100k lines
with 1000 folds took 2.880 ms (8 ms budget); a frame over a 10 MB line took 2.165 ms (8 ms);
comparison scrolling took 1.356 ms (10 ms). All thresholds and coverage remain unchanged.
No native/manual font/IME smoke, VoiceOver or general BiDi security validation is claimed;
GPUI tests use the deterministic test text backend. Adjacent zero-width marks may overlap,
and configured line terminators omitted from shaped rows are described in Character Info only.
Controller review and final workspace/all-performance gates remain pending. Exact commands,
failures resolved during iteration, source hashes and limits: `.superpowers/sdd/editor-features-plan/task-5-report.md`.

### E03 review round 1 behavior amendment (before fixes)

Warning colors must not use the customizable caret key. Use literal amber #F9AE58 only where
it has at least 4.5:1 contrast against the editor surface; otherwise use whichever of opaque
black/white contrasts more. Compute separate colors for the normal editor background, selection
background composited over it, and the current Find match background composited over it.
Cache glyphs by both glyph and surface variant so one `!` on different surfaces cannot reuse
the wrong color. Text geometry, rules, descriptions and bounded scanning remain unchanged.
Add a rendered custom-scheme regression with white editor/black selection/white selected text
and caret matching either background, plus U+2028/U+2029 glyph coverage. Run the palette-mutating
regression in its own test subprocess to avoid affecting other parallel tests' global theme.


E03 review round 1 verified: warning ink now bypasses the caret palette key and chooses amber
where contrast is at least 4.5:1, otherwise black/white, independently on normal, selected and
current-Find surfaces. Translucent selection/find colors are composited over the editor;
glyph caches distinguish surface variants. The isolated custom-scheme GPUI regression failed
first with black-on-black selected warnings, then passed with both black/white caret colors,
translucent selection and the same glyph on different surfaces. It also verifies rendered
U+2028/U+2029 warnings; the documented LF/CRLF policy is unchanged.

Final checks passed: 7 E03 UI tests, 2 config Unicode tests, all 165 editor tests, 10 preferences
tests, 2 Character Info tests, strict config/UI all-target Clippy, touched rustfmt and diff checks.
All 10 serial release rendering gates passed on frozen final source: fold scrolling 2.941 ms
(8 ms budget), 10 MB-line frame 2.179 ms (8 ms), comparison scrolling 1.372 ms (10 ms).
No dependency, input/geometry, settings-schema or deferred-feature change. Full command/results
and remaining native-test limits are appended to `task-5-report.md`.

## E04 behavior specification — before implementation (2026-10-29)

Ordinary insertion remains the default. Overtype is transient, per ViewModel: new, duplicated
and restored views start in insertion mode; changing it neither dirties the document nor creates
an undo step or changes another view. No settings/session schema or dependency is needed.
The editor action `editor::ToggleOvertype`, Edit menu/command palette and a persistent clickable
status-bar `INS` / `OVR` indicator provide entry points (including switching back to insertion).
Single-line fields and immutable snapshot viewers cannot enable it.

In overtype, keyboard text (native input and InsertText actions) replaces one existing grapheme
per new incoming grapheme in the preceding cluster’s context, stopping before LF, CRLF or lone
CR and extending at end of line/file. A combining mark, ZWJ/emoji continuation or regional
indicator that extends the preceding cluster inserts without deleting unrelated following text.
Selections are replaced exactly, without deleting following text. Auto-pair insertion, wrapping
and step-over are disabled in this mode. Paste, Tab/indent, newline, snippets and explicit native
replacement ranges keep their existing insertion/replacement semantics. Undo/redo uses the
existing shared-document history and mode changes close typing groups.

Overtype IME composition inserts a temporary marked preview before the original target: updates
replace only that preview, retaining all underlying source (including selected source). Empty
composition cancels/removes the preview and restores the target selections. Commit (including
unmark) removes the preview and applies overtype once, with one composition undo step; non-BMP
UTF-16 ranges and multiple carets remain supported. Selection/motion/other-view changes invalidate
composition targets, leaving the existing preview as inserted text as current composition-ending
commands do; stale target metadata must never delete newly edited source. Read-only/save and
immutable-view input protections remain at their existing boundaries. A native range outside the
current marked preview uses ordinary explicit replacement rather than stale overtype metadata.

Acceptance: backend tests for default/mode locality, Unicode graphemes, line endings, selections,
multiple/overlapping carets, insert/paste/newline semantics and undo/redo; GPUI tests for actual
keyboard/native/action paths, menu/palette and clickable rendered indication, composition update,
commit/cancel/unmark, selected source, shared-view invalidation and immutable viewers. Focused
existing IME/fold/Unicode-warning regressions, strict touched checks and serial release typing /
input gates precede handoff; controller owns final workspace/all-performance verification.


## E04 result (2026-10-29)

Implemented transient per-view overtype with `editor::ToggleOvertype`, Edit menu/Command Palette
entry and persistent clickable INS/OVR status. New, duplicated, restored and reopened views use
insertion; no schema/dependency changes. Keyboard replacements preserve graphemes, line endings,
selected ranges and per-caret left context; combining/emoji continuations retain following source.
Existing insertion, paste, Tab/newline, snippet, undo and shared-document machinery is reused.
IME previews retain original source until commit; both cancellation paths and stale targets are
covered. E03 warning repaint and E01/E02 folds remain intact.

Verification passed: 651 editor tests; 11 E04 GPUI tests; focused existing input (8), dead-key (1),
composition (9), fold (28), Unicode-warning (7) and fold-persistence (11) tests. Strict editor/UI
all-target Clippy, touched rustfmt and diff checks passed. Frozen-source serial release typing gate
passed at 0.003 ms / 1 ms (one caret) and 1.412 ms / 20 ms (1,000 carets); undo/redo gates passed
at 0.096 ms / 3 ms and 14.751 ms / 30 ms. The real UI keystroke with 10M find matches passed at
48.544 ms / 300 ms. No physical native IME/accessibility smoke is claimed. Controller review and
final workspace/all-performance gates remain pending; exact commands, decisions and limits are
in `.superpowers/sdd/editor-features-plan/task-6-report.md`.


### E04 review correction — before product fixes (2026-10-29)

The controller demonstrated that capped preceding-cluster context can delete following source
when a ZWJ continuation follows an emoji with 600 combining marks. E04 is reopened until repaired.
The amended contract requires complete preceding grapheme context without flattening an ordinary
long line/document. Both IME cancellation entry points must also restore original backward/forward
selection orientation and primary identity at every caret. Permanent backend/native-input and
both-cancel-path regressions will run red before fixes; existing expanded forward segmentation
must remain intact. Focused regression/strict checks and frozen-source serial typing/input gates
will be repeated after the fixes.

The same correction includes snapping programmatic carets inside long clusters: for `👩` + 600
U+0301 + `z`, a caret at char 600 must snap to char 0, and overtyping `a` must produce `az`.
The controller identified the still-capped snap helper during the first release compile; permanent
backend/native regressions are being run red before this additional fix. The superseded binary
will not be used for the final performance claim.


### E04 review correction verified (2026-10-30)

Reproduced and fixed both the long preceding-emoji context loss and the interior-caret snap
failure with permanent backend/native regressions before their fixes. Backward segmentation and
snapping now share complete Unicode `GraphemeCursor` traversal over borrowed rope chunks; the
capped helper is removed and forward-cluster expansion retained. No ordinary line/document is
flattened per keystroke. Both IME cancellation paths now restore full original Selection snapshots,
including backward orientation and primary identity, with separate red/green multi-caret tests.

Final checks passed: 654 editor tests, all 176 editor-view GPUI tests, 15 overtype UI tests and
existing dead-key/composition/Unicode-warning/fold-persistence regressions; strict editor/UI
all-target Clippy, touched rustfmt and final diff check. On frozen final source, serial release
typing passed at 0.001 ms / 1 ms (one caret), 1.262 ms / 20 ms (1,000 carets); undo/redo passed
at 0.051 ms / 3 ms and 15.060 ms / 30 ms. The UI 10M-find-match keystroke gate passed at
44.823 ms / 300 ms. E04 is verified again; controller re-review/final workspace gates and native
manual smoke limits remain as recorded in `task-6-report.md`, which includes exact red/green
commands, failures and final results.

## E05 behavior specification (2026-10-30, before product edits)

`virtual_space` is a boolean setting, false by default, accepted by the existing global,
project and scoped preference pipeline. It enables end-of-line virtual navigation in editable
file views; fields and immutable viewers stay clamped. Disabling it clears virtual endpoints
without editing text. No shortcut or transient toggle is added: Preferences is the entry point.
Existing session selection tuples remain compatible and store real offsets only; virtual columns
are transient, including reopen/restore. Undo and selection history retain them within a live view.

Extend `Selection` with independent anchor/head counts of spaces beyond their real line-end
char offsets. Order/merge uses `(offset, virtual spaces)` endpoints; real document offsets remain
valid, and nonzero virtual counts are valid only at line ends. Display uses the existing Rows,
Measure, folded/wrapped and column-selection machinery. Tabs and Unicode retain existing shaped
positions and grapheme boundaries; beyond the shaped end each virtual column is one space glyph.
Right extends into virtual space; Left retreats through it before normal grapheme movement.
Up/Down/Page preserve the measured x, including short-line virtual endpoints. Home/End, word and
document navigation return to real text (WordLeft from a gap reaches its line end first).
Shift movement and plain/rectangular mouse drag retain virtual selection endpoints. Virtual space
is available only after a logical/display line's final row, never inside a soft-wrap continuation,
fold placeholder or filler row. Existing folded text navigation stays with Rows.

Typing, paste, Tab, newline and explicit insertion materialize only the spaces needed to reach
each replacement's starting endpoint, then use existing edits; multiple carets on one line share
padding. Padding and text share one undo step with the original virtual selections. Overtype counts
incoming graphemes after the padding has become their actual left context. Backspace through a
virtual gap moves left without deleting source; Delete there does nothing; word-backspace reaches
the real line end without touching the preceding word. Deleting a selection removes real selected
text only, collapsing a wholly virtual selection without adding spaces. Copy exports real selected
source only; a wholly virtual selection/caret does not invoke implicit whole-line copy/cut. Whole-line
clipboard paste retains its existing above-line behavior. Existing structural line commands target
the real lines. Completion and trigger snippets cannot consume the source word behind a gap;
explicit triggerless snippets insert at the virtual position.

IME previews share existing composition/history machinery. Original full selections and primary
are retained; provisional virtual padding is removed on cancellation along with the preview, and
commit applies insertion/overtype once. Revision, selection and primary mismatches invalidate old
composition targets; stale data never removes new source. Other-view changes map ordinary offsets
and conservatively discard virtual endpoints touched at their line end, retaining unaffected gaps.
Read-only input/save boundaries and save snapshots are unchanged: navigation never dirties or changes
source bytes. No dependency, replacement editor engine or session schema migration is needed.

Acceptance: opt-in/default and scoped settings compatibility; navigation/selection/copy/delete;
tabs, long graphemes, mixed Unicode and CRLF; multiple carets on same/different lines; typing,
paste, Tab, newline, overtype, snippets, undo/redo, shared views, clean saves and stale/cancel IME.
Actual GPUI input and shaped geometry tests cover clicks/drags/carets/selections, wrap/folds,
settings reload, immutable viewers and session persistence. Focused backend/config/UI regressions,
touched strict checks and serial frozen-source release typing, large-find keystroke and rendering
gates precede handoff; controller owns final workspace/build/all-performance gates.


## E05 result (2026-10-30)

Implemented opt-in `virtual_space` through global/project/scoped settings and existing file-view
preferences. Real char offsets plus independent virtual endpoint counts retain per-view selection,
primary and history ownership. Navigation, shaped clicks/drags/rectangles, folds/wrap/fillers,
painting, native bounds and long-line scrolling share the existing Rows/Measure pipeline. No source
padding appears before an insertion needs it; multiple carets on one line share padding and undo
restores virtual endpoints with the text. Copy/delete/no-op/Tab/newline/clipboard/snippet/overtype
paths and native composition commit/cancel/stale guards are integrated. Sessions retain their real
offset tuple schema; save snapshots omit unmaterialized spaces. No new dependency or E06+ work.

Verification: **670 editor tests, 56 config tests, 1,023 UI tests (24 release tests ignored)** pass.
Strict config/editor/UI all-target Clippy, touched rustfmt and final diff checks pass. Permanent
red/green regressions cover far-beyond-long-line scrolling, whole-line clipboard selection handling,
empty paste, default shared-view vertical-goal reset and explicit native composition targets.
The complete review covered constructor updates and input/no-op/clipboard branches before freeze.

A prior T04 preservation issue found by the broad UI suite was repaired at the controller's direction:
restore the original comparison-cancellation call when closing the terminal panel. An E01 save/undo
regression's indentation setup is now explicit Plain Text, preserving all exact assertions while
removing the race with syntax-worker boundaries; dedicated syntax coverage is unchanged.

On the final frozen build (5m 28s compile; all 269 crate/bench Rust fingerprints unchanged afterward),
serial release typing passed at **0.003 ms / 1 ms** (one caret), **1.550 ms / 20 ms** (1,000 carets);
undo/redo passed at **0.096 ms / 3 ms** and **14.885 ms / 30 ms**. The real UI keystroke with 10M find
matches passed at **44.715 ms / 300 ms**. All **10 rendering gates** passed, including 100k-line /
1,000-fold scrolling at **2.851 ms / 8 ms** and comparison scrolling at **2.858 ms / 10 ms**.
No build or test competed with measurement. Earlier interrupted compiles supply no performance claim.

No physical native IME/accessibility smoke is claimed. Session virtual positions are intentionally
transient; ending a composition through movement/settings/another-view edits retains the inserted
preview/padding as ordinary text, matching the existing composition policy. Controller review and
final workspace/build/all-performance gates remain. Exact commands, results, failed iterations,
changed files and limits: `.superpowers/sdd/editor-features-plan/task-7-report.md`.

## E05 review correction contract (2026-10-30, before fixes)

Review round 1 reopened E05 for three permanent regressions. Distributed paste must decide each
target's replacement before requesting padding; an empty assigned string contributes no pad,
while an existing pad needed by another same-line target maps the no-op endpoint normally.
Option-drag retains horizontal pixel goals relative to the text origin and resolves both through
the existing Rows/Measure for every row; normal-mode rectangle column behavior remains intact.
Pending server completion commits must match the complete logical selection and primary captured
at commit, including virtual endpoints. This guard is completion-specific: formatting/save
freshness remains independent of caret movement. Exact bytes/endpoints/undo, real GPUI shaped
rectangle paint in both directions, and a gated real-fixture completion response are the red/green
proof before affected checks, self-review, source freeze, and serial release gates.

Review correction debug proof: both distributed-paste regressions failed before the fix; the
real GPUI run failed both rectangle assertions and the pending completion primary/additional edit
assertion. All pass after correction. Full affected checks now pass: **672 editor tests** and
**1,025 UI tests / 24 existing ignored**, strict editor/UI Clippy, touched rustfmt and diff whitespace.
Amended seams were self-reviewed against `task-107-before`. Product source (269 crate/bench Rust
files) is frozen for scoped controller review and serial release typing/input/rendering gates;
these final gates and review are still pending. Full evidence is appended to `task-7-report.md`.

Correction frozen-source release proof is now green: all four typing/undo budgets passed; the
real UI keystroke with 10 million find matches measured **40.776 ms / 300 ms**; all **10 rendering
gates** passed (fold scrolling **2.831 ms / 8 ms**, comparison **2.596 ms / 10 ms**, long-line
frame **2.093 ms / 8 ms**). All 269 source fingerprints stayed unchanged. The controller's scoped
re-review remains pending; E05 is still unchecked until acceptance.

## E06 behavior specification (before implementation)

`modelines` is a boolean file preference, default **false**, accepted in Preferences, scoped
rules and project preferences through the existing Settings UI/reload path. When enabled,
only a declarative Vim/Emacs subset affects indentation, tabs, wrapping and a single margin.
No modeline can execute code or change syntax/language, encoding, shell, plugins or save options.
No runtime or dependency is added. Existing document bytes, dirty/saved state, undo history,
virtual space, Unicode warning and format-on-save preferences are preserved.

Scan first five and last five logical rope lines, once each in document order; ignore an entire
line exceeding 4096 UTF-8 bytes. Markers/keys/values are ASCII and case-sensitive; surrounding
Unicode remains ordinary text. Vim markers `vi:`, `vim:`, `ex:` must start a line or follow ASCII
whitespace, then `set` or `setlocal`, whitespace, options, and a terminating colon. Options are
whitespace-separated: `ts`/`tabstop` and `sw`/`shiftwidth` integer 1–16;
`et`/`expandtab`, `noet`/`noexpandtab`; `wrap`, `nowrap`; `tw`/`textwidth` integer 0–1000.
Emacs uses one `-*- ... -*-` pair with semicolon-separated `key: value` entries:
`tab-width`, `standard-indent` integer 1–16; `indent-tabs-mode` `t`/`nil`;
`truncate-lines` `t`/`nil`; `fill-column` integer 0–1000. Margin fields set wrap width and one
ruler (0 clears the ruler), without independently enabling wrapping. Only unsigned ASCII decimal
numbers are accepted. Later valid fields/lines win individually; unknown/malformed fields are
ignored and never invalidate other supported fields. The first complete Emacs pair is read,
then the first complete Vim directive on that line overlays its fields. No multiline Emacs Local Variables block,
Kate syntax, Vim expressions, escapes or command evaluation is supported.

Precedence: automatic open-time indentation detection < explicit global/scoped preferences <
project/project scopes < supported modeline fields < EditorConfig indentation < existing
explicit shared document indentation and window/per-view display overrides (including distraction
free mode). Independent tab/indent widths follow the existing EditorConfig/document owners.
Unspecified fields fall back to those earlier layers, never a fresh Settings object. A removed,
invalid or disabled directive removes only its overlay and restores the current lower layer.

Scan on open/restore/new view, settings/project/syntax refresh, and document text changes
(including undo/redo and external reload). Shared views refresh together; their explicit wrapping
choices remain local. Scans are bounded, synchronous current-text reads with no asynchronous
result to become stale; normal edits only reapply preferences when parsed directives change.
Read-only files can use presentation/indentation preferences without becoming editable. No
scan mutates text or creates undo steps; saves retain original bytes unless ordinary edits or
existing save cleanup change them. Save As uses destination-scoped enablement/EditorConfig.

Acceptance: parser tests cover both syntaxes/aliases, bounds, order, Unicode, invalid numbers,
unsupported/malicious fields and oversized lines; settings tests cover default/scoped/project
compatibility and serialization; GPUI tests cover detection/settings/project/EditorConfig/manual
precedence, live edit/removal/undo/reload/disable, multiple views and explicit view ownership,
read-only/text/undo preservation and unrelated E03/E05/L04 preferences. Focused strict checks
and serial typing/render gates establish bounded edit-path cost before controller final gates.


## E06 implementation evidence

Implemented the specified optional `modelines` file preference through existing Settings,
project/scoped preferences and the Settings UI/template/reload path. A bounded data-only Vim/Emacs
parser overlays only indentation/tab/wrap/margin fields. Shared live edits, undo/redo, external
reload, new views and destination-scoped Save As preserve manual document/per-view ownership,
read-only protection, source bytes and unrelated E03/E05/L04 preferences. No dependencies,
runtime, executable content, persistence schema or other feature scope added.

Focused/full affected verification: config **60 passed**, editor **672 passed**, UI **1030 passed /
25 ignored**, no failures on final recheck; 4 new config regressions and 5 new E06 GPUI regressions.
Touched strict Clippy, workspace formatting and diff checks passed. One intermediate full run
exposed an unrelated L05 fixture publication-file race (EOF JSON), passing in isolation and on
the next full run; retained in `.superpowers/sdd/editor-features-plan/task-8-affected-final.log`
for a separate controller follow-up.

After self-review and source freeze, release UI test compilation passed; serial modeline scan
gate measured **0.003 ms** on 100k Unicode lines, **0.137 ms** for maximum bounded edge payload
and **0.000 ms** printed precision for rejecting a 3 MB line (each budget 1 ms). Existing typing
and undo/redo gates passed; large-find UI keystroke **40.292 ms / 300 ms** passed. Independent
review approved spec/quality without code findings. Exact commands, logs, default/grammar,
limits and self-review are in `.superpowers/sdd/editor-features-plan/task-8-report.md`.
Controller owns the final checkbox and workspace-wide build/test/performance gates.

L05 fixture verification follow-up (after E06): an affected-suite run exposed a publication
marker before its JSON writer closed it. Both identity markers now publish completed content
through atomic replacement. The two unchanged delayed-identity diagnostic tests pass; no
assertion or timeout was relaxed. Full workspace verification remains the final gate.

## F01 behavior specification (before implementation)

Default opens remain BOM-detected Unicode, then UTF-8 for unmarked bytes; no statistical legacy
encoding guessing or encoding modelines. Supported codecs are UTF-8, UTF-16 LE/BE, UTF-32 LE/BE;
Windows-1250 through Windows-1258; ISO-8859-1, ISO-8859-2, ISO-8859-5, ISO-8859-7, ISO-8859-15;
Shift JIS, EUC-JP, ISO-2022-JP, GBK, GB18030, Big5 and EUC-KR. Legacy mappings use the existing
locked encoding_rs implementation of the WHATWG Encoding Standard (Shift JIS includes Windows
extensions, EUC-KR is the extended Windows-949 repertoire, Big5 includes its WHATWG extensions).
ISO-8859-1 is implemented separately as the exact byte-to-U+0000–U+00FF mapping, including C1
controls; it is never the Windows-1252 alias. Unicode codecs retain native scalar/unit conversion.
Only Unicode encodings allow a BOM; UTF-32 signatures are recognized before UTF-16 LE.

File menu and command-palette Reopen with Encoding reinterprets disk bytes without writing,
refuses unsaved edits, and shares the result across views. Save with Encoding selects the output
codec and uses the ordinary serialized save/formatting path. Both BOM and no-BOM Unicode output
choices are reachable. Reopen choices have no synthetic BOM option: actual bytes determine it.
The selected codec/disk codec survive existing session and history metadata. A remembered hint
is only for BOM-less bytes; an external BOM wins during ordinary reload/recovery. Explicit reopen
conflicting with a BOM produces a protected malformed read rather than silently stripping it.

Malformed reads remain read-only and cannot overwrite their original (including aliases).
Unrepresentable output and non-round-tripping codec mappings fail before history capture or any
write. Text, dirty state and pending output choice remain available to fix/retry; disk encoding
changes only after a successful save. No substitution, HTML numeric-reference replacement,
normalization, or codec fallback is permitted. BOM/newline metadata follows the existing save,
reload, comparison, replacement, pipeline and history owners. Async reopen rejects stale text,
file identity, encoding metadata, save activity or changed disk bytes/stamp; queued saves retain
the existing capture/formatting serialization contract.

Project search uses live text with Use Buffer; disk searches use a known open document's disk
codec when available, otherwise BOM/UTF-8. Replacement snapshots retain that codec and validate
bytes before applying. Unopened ambiguous non-UTF-8 files are searched with protected replacement
characters and never rewritten; open/reinterpret them explicitly to edit/search exact text.
Closed-file pipelines and folder comparison use BOM/UTF-8 unless a safe known hint is available;
malformed/binary input is skipped/reported, never guessed. File comparison/history use their known
codec metadata. Git's byte-oriented patch/staging interface retains its existing explicit non-UTF-8
or binary refusal; it does not transcode Git blobs or patch coordinates. Statistics and checksums
report conversion failure instead of inventing bytes.

Acceptance covers exact literal UTF-32 and Windows/ISO/CJK fixtures, malformed scalars and byte
sequences, all codec round trips, C1 distinction, yen/overline/minus mapping rejection, unsupported
BOM requests, no-loss save/save-as/queued formatting and history ordering, named BOM-less reload
and recovery, UTF-32 search/replacement/pipeline/folder/file comparisons, legacy search hints,
shared views, read-only aliases, cancellation/stale selection and unchanged external bytes on
failure. Existing UTF-8 ownership and relevant serial storage/search performance budgets remain.
The only dependency change is a direct encoding_rs edge to the version already in Cargo.lock;
no package or runtime is added. Controller marks the F01 checklist after final review/gates.


## F01 implementation evidence (review/performance pending)

Implemented the 26-codec set and explicit reopen versus save-conversion workflows specified above.
Native UTF-32/Latin-1 and the already locked encoding_rs mappings preserve decoded Unicode text;
unmappable/alias-changing output and accidental BOM prefixes fail before history or writes.
Existing save queues/formatting, read-only protection and shared-document ownership remain.
Disk hints persist independently of pending conversion through recovery/recent closed files;
search/replacement/extraction, pipelines, comparisons and history use their known interpretation.
No encoding modelines or legacy guessing were added. Only the encoding_rs direct dependency edge
was added; no package/version/checksum changed.

Self-review complete. Full affected suites pass: project **78**, editor **677**, workspace **104**,
UI **1039 / 25 existing ignored** (1898 passes, zero failures). Touched strict Clippy, fmt and diff
checks pass. Initial wording-only expectation failures and corrections are retained in task-9
logs; no semantic assertions, fixtures or performance budgets were weakened. Product source is
frozen for independent read-only review and serial six filesystem/load/save/search/index/poll
release gates. Those gates and controller acceptance remain pending; F01 stays unchecked.
Exact commands, red/green evidence, changed files, codec semantics and limits are recorded in
.superpowers/sdd/editor-features-plan/task-9-report.md.

F01 review corrections: an actual close → folder scan → copy regression reproduced the
missing remembered legacy codec at capture; a literal UTF-16 regression reproduced statistics
missing a BOM prefix spanning two scalars. Both now pass, with 14 statistics and 55 comparison
tests, including existing copy/alias/race protections. Strict touched Clippy, formatting and
whitespace checks pass. Independent scoped re-review and final-source storage gates remain.

F01 final-source release test compilation passed (5m 33s). The corrected, listed
`testing::bench::filesystem` selector executed all six serial storage gates: all passed.
100 MB load 81.701/1500 ms, 99 MB-line search 185.012/1500 ms, disk poll 0.179/1 ms; UI
save capture/finish below 0.001/1 ms. The initial zero-test selector is retained and not counted.
Independent scoped re-review accepted both fixes; all 282 frozen source/manifest/lock hashes
remain unchanged. F01 is complete; full workspace acceptance remains the final gate.

## F02 behavior specification

File > Print… and File: Print… in the command palette open source print options; existing
Cmd-P navigation shortcuts remain unchanged. Defaults are whole document, 11-point system
monospaced black text, four-column tabs, wrapped long lines, and no line numbers. Font size
(8–24), tab width (1–16), and source line numbers are per-job form choices, deliberately not
persisted. The native macOS print panel supplies paper, orientation, page range, printer and
system PDF destinations.

The command captures an immutable rope and selection snapshot immediately, including folded
source. Selection scope includes all nonempty selections in source order, merges overlaps,
and separates disjoint excerpts with a newline; it is refused if no text is selected. Source
line numbers retain document positions. Read-only and shared views can print. Nothing changes
text, encoding, dirty state, undo, folds or selections. Edits after opening options do not
change that job; switching/closing its target before submission cancels through the existing
form guard. Escape cancels options; native Cancel ends the job without editing. Native failures
are reported, and a second native operation is refused while one runs.

Preparation runs in the background; AppKit presentation is queued on the foreground executor
outside App/Window updates, so its modal event loop never holds GPUI state. Native NSTextView
layout and NSPrintOperation own pagination and PDF drawing; no replacement renderer or new
dependency. Acceptance covers immutable Unicode/tab/multi-selection snapshots, options errors,
read-only/shared/undo preservation, reachable menu/palette/form, cancellation/stale targets,
and native disposable multi-page PDF output with extracted text and rendered-page inspection.
No physical print jobs are used in verification. Controller marks F02 after scoped review.

F02 snapshot clarification: active IME composition in any tab sharing the document, including
another window, refuses Print without committing or removing provisional text. A view whose
selection revision has not yet caught up with a shared edit also refuses until its normal
notification arrives. Once the options have been submitted, the captured job is independent
of later source edits. Native Cancel and system PDF Save have been exercised in a real GPUI
workspace using a disposable support directory, without a physical printer.

## F02 implementation evidence (controller review pending)

File/menu/palette print options and native paginated Print/PDF are implemented without new
dependencies. The detached print view explicitly uses AppKit TextKit 1: a stronger PDFKit
regression found that the default TextKit 2 paragraph renderer retained invisible duplicate
text outside page clips. The native-only initializer correction preserves every source record
exactly once in full-PDF extraction, including wrapped words and Unicode. Native pagination,
font fallback, the system panel and PDF saving remain AppKit-owned.

Ten focused tests cover snapshots, source/selection formatting, read-only/shared views,
composition in another view/window, stale shared-selection mapping, folded source, cancellation
and undo/redo preservation. The full UI suite passes **1050 / 25 existing ignored**, zero failures;
strict UI all-target Clippy, formatting and whitespace checks pass. Native disposable PDFs are
14 pages (document), 1 (selection), 25 (16-point numbered text), 1 (empty), and 3 (actual system
PDF Save). All records and wrapped-word counts pass PDFKit assertions; representative pages and
contact sheets were visually inspected. The actual native Cancel and PDF Save paths return to
a responsive clean editor. No physical jobs were submitted.

No editor rendering/input/search hot path changed; no interim performance gate is needed for
this isolated native command. Workspace-wide checks, release build and performance gates remain
controller-owned. Source is frozen for scoped review; F02 stays unchecked until acceptance.
Details, exact commands/logs and final QA paths: .superpowers/sdd/editor-features-plan/task-10-report.md.

F02 controller acceptance: independent spec/quality review approved with no findings.
All 288 source/manifest/audit/changed-file fingerprints match the frozen packet. Controller
visually inspected all-page contact sheets and representative default/numbered/selection/blank
PDF pages; actual geometry assertions confirm full-size uncropped pages. Full UI 1050 passed,
strict touched Clippy/fmt/diff and native print/PDF QA passed. No interim editor hot-path gate
is required; final workspace release/tests/all-performance checks remain pending.

## T07 source-safety follow-up specification (before regression edits)

Send/Execute retain their existing distinct newline behavior and allow read-only source text.
Before reading a selection, refuse a source view whose revision is awaiting shared-edit mapping,
and refuse while any shared view/window has marked IME text. Report a short status; create no
terminal, queue no input, and leave source/selection/composition/undo unchanged. This avoids
slicing current text with old offsets or sending provisional overtype/virtual-space text.
Acceptance: actual Send and Execute callers under a same-update shared deletion and cross-window
IME composition, plus the existing real PTY send/read-only/execute tests. No general input or
terminal protocol refactor. T07 stays unchecked during this preservation check.

T07 source-safety follow-up verified: both actual-caller regressions reproduced first, then
passed for Send and Execute. All 11 terminal UI tests and strict touched Clippy/fmt/diff pass.
Independent scoped review approved both guards with no new breakage; all 288 frozen files
match. Stable Unicode/read-only sending, session reuse and distinct Execute newline remain.
T07 is restored complete before F03; final workspace checks remain pending.

## F03 behavior specification (before implementation)

Log following is transient and off by default. File > Follow Log and the palette explicitly
start it; File > Stop Following Log and a persistent clickable status indicator stop it.
Start requires an existing clean file, no pending/queued save or write lease, and no IME
composition in any view/window of the document. Refusals preserve the document. One document
owns one background task, shared by all tabs/windows. New views inherit the lock. Input,
paste, IME, overtype/virtual padding, snippets, transforms, LSP edits, undo/redo, format/save,
Save As and writing pipelines cannot mutate a followed document; navigation, selection,
copy, Print and Send Selection remain available. Ordinary source read-only working copies
retain their existing behavior. No setting/session schema or automatic activation is added.

The task polls every 500 ms independently of focus, with bounded (existing 100 MiB limit)
regular-file reads and diffing off-thread. It follows the path across append, truncation,
replacement/rotation and temporary disappearance. Missing files retain the last snapshot and
retry. A stable malformed snapshot or read/size error pauses updates with a visible reason;
the mode stays locked until Stop, avoiding replacement-character publication or silent edits.
Incomplete final code units/escape sequences are withheld until completed; decoding uses the
actual disk codec, with external BOM precedence, including stateful ISO-2022-JP. A partial
snapshot is never treated as safe to overwrite: stopping it restores normal watching with
an unknown complete stamp and retains source protection until a normal complete reload.

Follow publishes clean snapshots through the shared change log. The first changed snapshot
clears text undo/redo history, and later snapshots keep it bounded; start/stop without changed
text preserves history. The start status states that incoming changes are not undoable.
The initiating view goes to EOF; shared views activate tail following by navigating to EOF or
invoking Follow Log themselves. Thereafter each view follows EOF only while its single caret
is at the previous EOF and its viewport is at the bottom; selecting or scrolling away preserves
inspection. Other selections/marks map through changes, including truncation. Stop retains the
latest accepted text and source-derived read-only policy, invalidates pending work and resumes
ordinary external-change handling. Closing the last tab, replacing a project/window's tabs or
renaming the source cancels ownership; closing just one sibling keeps the other following.
Save As is refused while following. Reopen starts normally, never following.

Acceptance covers real-file append/truncate/rename rotation/missing intervals, all Unicode and
CJK partial tails/stateful decoding/BOM changes, malformed input and size/read errors, shared
windows and late views, all mutation entry points versus read operations, bounded undo,
viewport/carets, dirty/save/IME refusal, pending read then stop/close/rename/restart, stale ordinary
reload and formatting, leases, ordinary insert/read-only and session compatibility. Async race
fixtures use explicit gates rather than sleep-only timing. F04 remains excluded.

F03 implementation and self-review are complete for controller verification. Final affected
suites pass: editor 683, project 85, UI 1064 (25 existing ignored performance tests); the UI
suite preceded the last narrowly scoped streamed-outline guard, which the final editor suite
and focused UI checks cover. Strict touched-crate all-target Clippy, formatting and whitespace
checks pass. An isolated native workspace verified explicit Follow, append, truncation,
rotation, blocked typing, menu/palette Stop, ordinary typing afterward and discard/quit.
The task report records RED/GREEN evidence and source-safety corrections. Workspace/release
and serial performance gates remain controller-owned; F03 stays unchecked until accepted.

F03 scoped review round 1 found and corrected three gaps: failed reads now retain the stamp
and file identity of the bytes they examined, stale/leased failures retry before pausing, and
accepted errors clear complete disk knowledge as well as protecting the source. Cut now
refuses before model selection expansion or UI clipboard/after-edit side effects. New real-file
regressions reproduce rotation during failed decoding and a write lease through a hard-link
alias. Malformed, oversized and unreadable files remain protected after Stop and cannot restart
without recovery. A separate two-gate test now covers Stop then restart before releasing the
old delivery; the earlier sequential cancellation test was renamed to match its actual scope.
Final affected suites pass 684 editor, 86 project and 1069 UI tests (25 existing ignored).
The task-111 patch/report records this correction; F03 acceptance and final gates remain pending.

F03 controller acceptance: all three review findings and the independent-gate restart case
passed scoped re-review. Final affected editor 684/project 86/UI 1069 (25 ignored), strict
touched checks, formatting and whitespace checks pass. Final-source release UI test compile
passed in 5m 46s; eight serial input/editing/disk-poll gates each executed exactly one test and
passed. 1000-caret typing 1.534/20 ms, undo/redo 14.579/30 ms, disk poll 0.194/1 ms, large-find
UI input 36.681/300 ms; block comment, delete, large edits, indent undo and uppercase gates
also pass. All 293 frozen-file hashes match after measurement. F03 is complete; final
workspace checks and the previously tracked global symbol benchmark remain pending.


## F04 — opt-in existing-file autosave: behavior contract

`file_autosave` defaults to false and is exposed in the existing Settings editor/template;
global, project and language/scope/path preferences use the normal resolver. An eligible dirty
existing file saves after one second without edits. Untitled, never-created/deleted, scratch,
read-only and Follow documents are skipped. Recovery `autosave(cx)` and optional file history
retain their separate policies. No background Save As or overwrite confirmation is opened.

One scheduler belongs to the shared OpenDocument. The oldest still-visible editor ViewModel id owns
its preference/formatting source, independently of focus; closing that view transfers ownership.
Every shared view/window must have finished IME composition and snippet fields before automatic preparation and
capture; temporary preview/source/padding stays untouched. Active saves/leases defer work.
Timer snapshots and generations reject edits, path/mode/source/policy changes and last-view
close. Disable and quit cancel pending automatic work; writes already captured finish through
the existing save queue and close/quit waiters. Discard cancels pending work before removal.

Automatic saves reuse asynchronous LSP formatting, cleanup, serialized capture, safe writes,
history and completion hooks. A retained last-read/successful-save disk baseline (independent
of disk-poll acknowledgments) guards background overwrites, including after asynchronous prep
and at publication. Unknown/restored-dirty baselines require an explicit Save or reload first.
Failures show useful status and pause that prepared text/format/path until a meaningful edit
or policy/output-format change; formatter-only edits must not make a retry loop. Changes made
during a successful write remain dirty and debounce again.

Acceptance: default-off/recovery/history separation; settings parsing/scoping; Unicode bytes;
shared ownership and IME in any window; delayed format + disable/path/close changes; edits during
writes; deleted/read-only/Follow/untitled skips; external changes before/during prep/publication;
failure suppression and codec-only retry; Save As/cancel; retained last-view/window close and
quit; THREE coalesced asynchronous preparations with all waiters and idle settlement. No new
dependency or configurable-delay surface is planned.

### F04 implementation and verification handoff

Implemented the default-off `file_autosave` preference through existing Settings/project/scoped
resolution and the normal save engine. One shared-document timer uses the oldest visible model
view id; composition/snippet navigation, shared Save As sheets, close decisions, mode/policy/source
changes and quit defer or cancel safely. A separate last-read/save baseline protects external
bytes, and the existing safe writer returns an actual-file receipt instead of trusting a replacement
observed after publication. Failure suppression includes output format and prepared text; fixing
only a codec retries without formatter-generated loops. Session recovery/history remain separate.

Regression evidence covers real files, Unicode, shared windows, IME started during formatting,
ordinary unmark without edits, snippet fields/mirrors and exit without edits, THREE coalesced
asynchronous preparations with all waiters/idle completion, edited/failed active writes, external
changes before/during writes and after publication, readonly/Follow/untitled/recovered-dirty skips,
Save As cancel, owner transfer, retained window/tab close, quit, metadata-only retry and cleanup
undo/redo. Default-off event handling short-circuits before scanning views/resolving preferences.

Affected config/editor/project suites passed (61/685/87). Final full UI passed 1,095 tests with
25 existing ignored gates. Final strict all-target Clippy for all four touched crates passed;
fmt/diff checks passed. A final frozen-source focused run passed all 25 autosave regressions and
the separate three-preparation queue regression. The only post-full-suite source change was the
behavior-preserving SaveValidation type alias required by strict Clippy. One new NotOnCaret test
fixture was corrected to place its caret in trailing whitespace; the existing cleanup policy was
not changed. The earlier failing run and correction are recorded, not counted as a clean suite.

Task-local patch/report: `.superpowers/sdd/editor-features-plan/task-12-review.patch` and
`task-12-report.md`. No dependency, codec set, session schema, Follow policy or original-audit
change. **F04 remains unchecked pending controller independent review and final release/performance
acceptance.** The writer checks cannot lock out arbitrary concurrent external filesystem writers;
that precise final-check race limit and unknown-baseline recovery are documented. No native/manual
IME, release build, performance pass or resolution of the existing bench_find issue is claimed here.


### F04 controller acceptance

Independent spec/quality review passed without findings. The frozen-source optimized UI test
build passed in 5m 52s. Four serial exact performance gates each ran one test and passed:
typing 1000 carets 1.757/20 ms, undo/redo 14.528/30 ms, save UI capture/finish below
0.001/1 ms, disk polling 0.190/1 ms, and large-find UI input 36.254/300 ms. All 295 frozen
file hashes still match. F04 is complete. Final full-workspace checks and the previously
recorded global symbol benchmark remain pending; native manual IME testing is unclaimed.


### F07 behavior specification (Task 13, acceptance pending)

- Default-on local rescue applies only to user-confirmed discard of dirty, ordinary untitled
  documents, including Discard All, when their last visible views close. Named documents,
  scratch/results/preview tools, clean tabs, cancellation, saves, and normal Quit/session
  recovery are excluded. Existing session/workspace schemas and historical backups stay intact.
- Keep at most 20 whole UTF-8 drafts, at most 1 MiB text per draft and 8 MiB total text,
  newest first, for seven days. Prune on store access; no truncation. Storage lives in the
  private DiscardedUntitled folder beside Session.json, using 0700 directories, 0600 atomically published
  text files, cleared inherited ACLs and no-follow reads. No new dependencies.
- File > Discarded Untitled and the command palette expose Restore Discarded Untitled… and
  Clear Discard History…. Restore creates a separate usable dirty untitled UTF-8 document,
  retaining Unicode exactly; it does not consume the history. Missing/expired/corrupt entries
  produce a useful status. Stale picker generations cannot restore after Clear.
- Capture completes synchronously at the explicit discard boundary, before close can quit the
  process. A bounded atomic write and a per-store serialization/generation guard prevent
  older captures from republishing after Clear reports success. Clear affects only rescue
  history. If retention cannot keep a draft (including size limits), the close asks whether to
  discard without rescue or cancel; no draft is silently truncated or reported as retained.
- Acceptance: real temporary-file tests for defaults, Unicode, all bounds/expiry, atomic errors,
  private modes/ACLs/no-follow, clear generation and older sessions; GPUI tests for explicit
  close/Discard All, shared same/cross-window views, cancellation, exclusions, reachable picker,
  stale restoration and dirty usable restoration, plus existing save/close/quit suites.


F07 implementation notes (controller acceptance pending): storage uses individual private
UTF-8 files so listing/pruning examines metadata only. A batch snapshots at most twenty
1 MiB texts and retains at most 8 MiB; synchronous capture at confirmation deliberately keeps
last-window quit and the existing save/close ownership protocol intact. This can delay the
UI on a slow disk. No typing, rendering, search or indexing path performs rescue I/O.
Active shared IME previews (including overtype source and virtual-space padding) require
finishing/cancelling composition or explicitly discarding without rescue. No session schema,
settings schema, dependencies, historical backup policy or F03/F04 ownership API changed.

Self-review also reproduced an existing close-lifecycle IME edge: closing an overtype composing
sibling left `界original` in the shared rope, later rescued instead of committed `界riginal`.
`close_editor` and window `finish` now use native unmark before removing a closing view.
Move-only `remove_view` remains unchanged. GPUI regressions verify exact shared text, Undo/Redo,
virtual-space committed saves and cross-window Discard All settlement. Last-view active IME
still offers Cancel or Discard Without Rescue before capture, as specified above.

F07 final agent verification: affected full suites pass (`orlo-ui`: 1108 passed, 25 ignored;
`orlo-workspace`: 111 passed; both doc-test suites pass). Strict touched-crate all-target Clippy,
workspace formatting and diff checks pass. Thirteen new GPUI regressions and seven storage
regressions cover the feature and the demonstrated close-lifecycle edge. F07 remains unchecked
for controller acceptance. No release/performance build or physical native IME check was run.

F07 review round1 corrected a stale extra loss-confirmation gap: any change to window tab
membership now cancels closing, alongside existing document identity/version/path checks.
Three actual pending-prompt/new-dirty-Unicode-tab regressions first failed, then passed for
single close, Close All and window Discard All. All17 rescue-related tests and70 existing
close-related UI tests pass on the fix; strict UI all-target Clippy/fmt/diff pass. The earlier
fullUI1108/workspace111 results precede only this narrow guard. F07 remains pending fresh
scoped re-review and controller acceptance; final whole-workspace gates are still pending.


F07 controller acceptance: fresh scoped re-review confirms the stale-confirmation finding is
addressed with no new breakage. All297 frozen-file hashes match. Final guard coverage is
17 rescue-related and70 existing close UI tests plus strict UI all-target Clippy/fmt/diff;
fullUI1108/workspace111/doctests precede only this narrow guard. F07 is complete, with the
documented bounded synchronous support-disk latency limitation. Global workspace/release/
performance checks and physical native IME smoke remain pending, not claimed.

### G04 behavior specification (Task 14, acceptance pending)

- Commit Staged remains the default and commits only the index. Git panel, Tools > Git and
  command palette expose Commit Staged and separate Amend HEAD actions. Both use local subject,
  multiline body and optional coauthors (one `Name <email>` per line); no AI or new dependency.
  Enter submits a subject; Command-Enter submits from multiline fields, where Enter adds a line.
- Amend asynchronously reads HEAD's message, refuses unborn HEAD, and permits message-only
  amendment with no staged changes. Subject/body are prefilled; existing trailers remain intact.
  The amend title identifies the target commit; reopened retained drafts are labelled.
  Git retains ownership of hooks, identity, signing and commit policy; no bypass flags are added.
- Repository identity, symbolic HEAD and commit ID are captured while loading and rechecked
  before submission/execution. Project/repository changes or a superseded/cancelled loading
  surface cannot execute an old request. Errors explain blank/invalid input, stale HEAD, writes
  in progress and Git failures. Cancel/failure retains the current in-memory fields for retry;
  successful commit clears that draft unless it was edited while Git ran. Each mode retains one
  repository draft, replaced when composing in another repository. Drafts are window-local and
  are not session persistence.
- Validate a single-line nonblank subject, body controls and complete coauthor names/emails;
  prevent newline/control/trailer injection in identities while accepting Unicode. Subject/body text
  is sent through Git stdin, never a shell. Ordinary commits still refuse an empty index via Git.
- Acceptance: disposable repositories verify staged-only commits, empty-index amend, unborn and
  moved HEAD, multiline/Unicode/trailers, invalid input, hook/signing failures; actual GPUI forms
  verify entry points, prefill, cancel/draft/error retention and stale project/repository/loading
  surfaces. Existing partial-stage, dirty-buffer and branch/history behavior remains covered.

G04 implementation notes (controller acceptance pending): the amend form names its original
commit, while a reopened retained draft is explicitly labelled. The backend pins repository
root/Git-directory identity, symbolic HEAD and commit ID; stale checks run after preparation
and again at execution. Normal commits and amendments use the index without automatic staging.
Hooks/signing failures leave exact form text available for retry; edits made while Git runs
survive the result for the earlier submitted snapshot. Drafts are bounded to two repository
slots per window (one per mode), with no persistence schema or dependency changes.

Self-review reproduced a pipe deadlock in the existing Git input helper: a pre-commit hook
filled stdout before Git consumed a large stdin body. The shared helper now drains output while
a scoped thread writes stdin, retaining the existing precedence of Git exit errors over pipe
write errors and all command environment/argument safety. A real noisy-hook/large-body test
first timed out, then passed. No source editor rendering/input algorithm changed; multiline
form fields reuse the existing panel-field mode. G04 stays unchecked for controller review.

G04 review round1 corrected trailer metadata loss during amend. Validated coauthors now use
Git's native --trailer handling (Git2.32+), preserving existing coauthors, sign-offs and folded
trailer values; configured trailer policy applies. The pure form validator runs no process.
A disposable amend first failed because Git parsed only the new coauthors, then passed with
all original/new metadata recognized. All43 Git tests and7 actualcomposition UI tests pass;
strict Git/UI Clippy/fmt/diff pass. The earlier fullUI1118 predates only this narrow change.
G04 remains unchecked pending fresh scoped re-review and controller acceptance.

G04 controller acceptance: independent scoped re-review confirms existing trailer metadata
preservation with no new breakage, and all299 frozen-file hashes match. Final43 Git tests,
7 actual composition UI tests, strict Git/UI all-target Clippy/fmt/diff pass; fullUI1118
precedes only the validated native-footer refinement. G04 is complete. Git owns hooks, signing
and trailer policy; native coauthor input requires Git2.32+. Final global workspace/release/
performance gates, real signing-key success and physical native smoke are not yet claimed.

### G02 behavior specification (Task 15)

Git sidebar, Tools > Git and the command palette expose Create Stash and Stashes. Creation
stashes saved tracked staged/unstaged changes by default; an explicit checkbox includes
untracked files (never ignored files). An optional message defaults to Git's message. Apply
and Pop offer normal restoration or explicit index restoration to recover partial staging.
List entries lead to Preview, Apply, Pop and confirmed Drop. Preview is a read-only scratch
patch, includes untracked files, describes binary changes and truncates at 2 MiB with a note.
All Git reads/writes run off the UI thread. Context pins repository/HEAD identity and the
stash stack (selectors plus object IDs); changed/missing entries or replaced repositories
require reopening. Pop applies the pinned object first and drops only after success and a
fresh stack check; conflicts keep the stash. Git's conflict protections remain in force.
Create/Apply/Pop refuse unsaved repository documents across all windows (including read-only
working copies), active save preparations and writes. The global directory lease pauses
background saves/follow/reload through completion. Clean documents use existing disk reload.
Cancelled/replaced pickers/forms and changed projects cannot redirect actions or publish a
late preview over a newer surface; retained file selection is preserved. Drop always prompts.
Acceptance uses disposable Git repositories and actual GPUI callers: Unicode staged/unstaged/
untracked restoration, bounded/binary preview, stale stack/repository/HEAD, conflicts and Pop
retention, missing entries, cancellation/selection, Drop confirmation, shared/read-only dirty
documents, pending preparations and write leases. No new dependencies or persistence schema.
G02 stays unchecked until controller review.

G02 implementation is ready for controller review. The local stash backend and reachable
sidebar/menu/palette workflows are complete; prepared contexts share G04's existing repository
identity reader without changing commit validation, native trailers or stdin handling. Six new
backend tests cover real Git state/odd filenames and eleven GPUI tests cover actual callers,
confirmation, cancellation, late surfaces, stale contexts, conflicts, shared read-only edits,
a retained document's asynchronous save preparation before its write lease, and directory leases.
Self-review reproduced a second-window Git-operation admission gap; the new regression failed
before the all-window operation check and then passed. Global saving/operation refusals are
conservative: an unrelated window's pending save or Git job also requires waiting.

Git's pathspec-free `stash push` needs a narrow `--no-literal-pathspecs` override because its
internal `:/` cleanup otherwise leaves untracked files behind. A real Unicode regression first
failed, then passed; leading-dash, glob/magic-looking and newline names also round-trip without
reinterpretation. The shared command/environment safety remains unchanged. Apply uses the
pinned stash object; Drop checks the entire stack immediately before Git's selector-based drop.
Concurrent external CLI mutations between validation and Git's own ref lock are not serialized
by Orlo's in-process lease; this is documented, not claimed as a cross-process transaction.

Final checks: `cargo test -p orlo-git -p orlo-ui --locked` passes all49 Git and1129 UI tests
(25 existing UI tests ignored), including G04 regressions; Git/UI strict all-target Clippy,
workspace formatting and `git diff --check` pass. All Cargo commands used the required shared
target directory. No new dependencies, persistence schema, product commits, pushes or PRs.
No release/performance build or physical native smoke is claimed. G02 remains unchecked.


G02 controller-requested lifecycle refinement: actual Stage and Commit callers each reproduced
an admission race after `clear_window` erased the old window's Git-operation flag while its
background job was queued. Both regressions were RED before the fix. The existing `git_run`
now accepts its explicit root and holds the existing directory lease through its worker;
branch switching delegates its previous lease to this owner. G04's separate commit worker
holds the same lease. No new coordination mechanism was added. Both tests are GREEN; all45
Git-panel tests (including13 stash tests plus commit/branch/partial staging) pass after this
refinement, with strict Git/UI all-target Clippy and formatting/diff checks. The full49Git/
1129UI result above predates only this narrow lifecycle correction; final global full-suite
coverage remains the controller's gate. G02 remains unchecked.

G02 fix round 1 specification: confirmed deletion must bind the selected entry and entire
prepared stack while holding Git's cross-process `refs/stash` lock. A terminal push/drop
before lock acquisition causes a stale refusal; competing changes while locked must fail
without changing the stack. Pop checks native-format support before application and reports
explicitly if application succeeds but removal fails. The narrowly scoped direct `git2`
dependency reuses the already locked version with default features disabled because the CLI
cannot atomically bind a positional reflog deletion to a checked OID. No new packages, native
reflog parser or general Git engine. Git owns ref/reflog locking; no claim of rollback across
worktree, reflog and ref I/O. Tests cover deterministic Drop/Pop races, lock contention,
top/older/last entries, linked worktree shared stashes, missing identity configuration,
unsupported formats and failure cleanup. G02 remains unchecked.


G02 fix round 1 implements locked native deletion, superseding the earlier disclosed concurrent
terminal limitation. Deterministic tests proved original Drop and Pop deleted an unconfirmed
terminal stash and returned success; both now refuse that changed stack. The native transaction
holds the shared `refs/stash` lock across rereading and matching the ordered OIDs, removing the
confirmed entry, and publishing. Real terminal push/drop fail while it is held; the identical
push succeeds after release. Tests also prove linked worktree common-directory locking,
top/older/last removal without user identity, stale/missing cleanup, and pre-Pop refusal for
SHA-256/reftable. Native ref/reflog I/O is not all-or-nothing: a packed-ref failure test proves
possible partial removal is reported with the selected OID. Pop explicitly reports successful
application followed by failed removal and tells the user not to apply the changes again.
The new direct git2=0.20.4 dependency has default features disabled and reuses existing locked
packages; the sole lockfile delta is the dependency edge. G04 and all other Git subprocess
behavior remain unchanged. Final full Git59 and Git-panel46 tests pass; global full-suite,
release/performance and physical native smoke remain controller gates. G02 stays unchecked.

G02 controller acceptance: fresh scoped re-review confirms wrong-stash deletion is fixed with
no new breakage; all303 frozen-file hashes match. Final59 Git tests and46 Git-panel tests
(including14 stash UI tests), strict Git/UI Clippy/fmt/diff pass. The prior fullUI1129
precedes lease/native-lock refinements; final global suites remain pending. G02 is complete.
Drop/Pop use the already locked git2 direct edge for identity-safe deletion; SHA-256/reftable
Drop/Pop fail closed, while Apply remains available. Native partial-publication I/O errors
are tested and explicit. No new package/version, unsafe fallback or real-repository test.

### G03 behavior specification (Task 16)

Git sidebar, Tools > Git and the palette expose Worktrees, Create Worktree and Prune
Worktrees. Worktrees lists local registrations with branch, current/main, locked and missing
state. Open validates the selected registration and opens its folder in a new window through
existing workspace opening, preserving the current window, unsaved text and session ownership.
Create chooses an existing local branch or a new branch at HEAD, with an explicit absolute
destination; the default is a new branch and creation does not automatically open a window.
No force, detach, branch reset or automatic branch deletion is offered. Remove requires an
explicit confirmation naming the selected path, refuses main/current/locked worktrees, and
lets Git protect tracked/untracked content. It also refuses unsaved or composing documents
under the selected folder across all windows, pending saves (including retained preparations)
and active writes. No documents are closed or replaced. Prune explicitly confirms removal of
missing unlocked registrations immediately (`--expire now`); existing folders and locked
registrations remain. All Git commands run off-thread with existing global directory leases.
Repository/root/HEAD, registration and selected-folder identities are pinned and revalidated;
changed project, root, selection or replaced/cancelled surfaces suppress stale results. Git
errors remain visible. No dependencies or persistent state are added. Acceptance uses only
fresh disposable repositories and actual GPUI callers: linked/common repositories, Unicode,
spaces/newline paths, create/open/remove/prune, dirty/untracked/locked/main/current refusal,
replacement/stale contexts, cancellation, cross-window shared dirty/IME documents, save
preparations and leases. G03 remains unchecked until controller review.

G03 destination clarification: the explicitly entered absolute destination must have an existing
parent folder. This keeps symlink resolution and the existing directory lease keyed consistently;
Git does not create intermediate parent directories through this UI. Open always uses a new
window, so the noted composing-view project-clear concern is outside this route and unchanged.

G03 implementation is ready for controller review. The local worktree backend and reachable
Git sidebar/Tools/palette flows list, create, open, remove and prune registrations. Seven new
real-Git backend tests and nine actual GPUI regressions cover linked/common repositories,
literal Unicode/space/newline paths, native dirty/untracked/locked/main/current protections,
branch retention, replaced directories/repositories/registrations, confirmed pruning, stale
and cancelled surfaces, cross-window read-only/shared/IME edits, retained save preparations,
and a sibling destination entered through a symlink alias whose lease survives window clearing.
Open preserves an originating named project and active composition while opening a new window.

Self-review caught two issues with failing regressions before fixes: Git's implicit creation
of missing destination parents could avoid canonical lease naming, and a stale registration
whose `.git` link disappeared could be mistaken for its ancestor repository. Destination parents
must now exist; every existing entry must match its own checkout root and common repository.
Invalid existing registrations require terminal Git repair. No new dependencies/persistence,
force removal, branch reset, session replacement, commits, pushes or PRs were introduced.
The original audit and G04/G02 backend/worker ownership remain unchanged.

Final affected checks pass: all66 Git tests and1141 UI tests (25 existing tests ignored),
strict Git/UI all-target Clippy, workspace formatting and diff checks. All Cargo invocations
used the required shared target directory. Existing dependencies still emit Cargo's future-
incompatibility notice for `block` and `proc-macro-error2`; no new warnings or failures remain.
Cross-process filesystem/Git changes are guarded by preflight identity checks and Git's own
protections, not an atomic external transaction. Final global workspace/release/performance
and physical native smoke remain controller gates. G03 stays unchecked for review.

G03 review round 1 corrected initial registration/Open validation for crossed linked .git
pointers within the same common repository. Each live linked checkout now matches its exact
administrative directory identity and canonical backlink; native relative backlinks and
Unicode/newline paths remain supported. The real crossed-pointer regression first failed,
then all68 Git tests and11 worktree-related UI tests passed, plus strict Git/UI Clippy/fmt/diff.
The fullUI1141 result predates only this backend guard. G03 remains pending fresh scoped review.

G03 controller acceptance: fresh scoped re-review confirms exact linked registration/backlink
correlation and no new breakage. All 307 frozen-file hashes match. Final 68 Git tests and
11 worktree-related UI tests, strict Git/UI Clippy/fmt/diff pass; full UI 1141 precedes only
this backend guard. G03 is complete. Open uses a separate window; no force, repair engine,
branch deletion or new dependency. Normal external preflight limitations are documented;
final global workspace/release/performance and physical native checks remain pending.


## G01 — explicit remote Git controls (specification before implementation)

Default: no remote contact during discovery, startup, status refresh, timers or configuration.
The Git panel, Tools > Git and command palette expose Fetch, Pull, Push and Remotes & Upstream.
Local asynchronous discovery lists configured fetch/push URLs and current branch/upstream.
Fetch explicitly selects one remote and does not change checked-out files. Pull requires a born,
attached current branch and configured upstream, and invokes Git without overriding pull.rebase,
pull.ff, signing, hooks or credential helpers. Push names the selected remote and current branch
explicitly, never force/mirror/delete; a form permits an explicit destination branch and upstream
setup. Configuration lists/adds/edits/removes remote URLs and sets/clears the current branch's
upstream using locally available remote-tracking branches. No clone or hosted service client.

Prepared contexts pin repository/HEAD, common metadata identity, effective configuration,
remote URLs and upstream refs. Async reads and forms additionally pin project generation, roots,
selection and active editor. Cancelled/replaced pickers discard late results; submitted jobs keep
leases until Git exits, even after window/project clearing. Once submitted, Git runs to completion;
there is no unsafe process-kill cancellation. Progress is indeterminate with bounded diagnostics.
Pull refuses dirty/read-only/composing documents across windows, pending retained save preparation
and active writes. All operations coordinate root/shared metadata directory leases and unsaved
Git metadata documents; configuration never contacts a remote. Existing Git credential helpers,
terminal-prompt suppression and inherited repository/index environment clearing remain intact.

Acceptance: real fresh disposable repositories and local bare remotes exercise fetch without
checkout movement, fast-forward/configured divergent/conflicting pull, normal push/upstream and
rejection, remote CRUD, upstream set/clear, detached/unborn/no-upstream and stale identities/URLs.
Actual GPUI callers cover palette/forms, cancellation/late results, shared dirty/read-only/IME,
retained save preparation, active-write refusal and durable leases. Preserve earlier Git/document
features. No new dependency is planned. G01 remains unchecked pending controller review.


G01 implementation is ready for controller review. The explicit Fetch/Pull/Push and Remotes &
Upstream routes are reachable from the sidebar, Tools and palette. Local forms configure fetch/
push URLs and current-branch upstream; Push optionally establishes tracking. Git still owns
pull/rebase/ff, credential helpers, signing and hooks. No automatic transport path was added.
The former local-only policy is deliberately replaced in README and architecture documentation;
the historical audit is unchanged. No dependency or persisted UI state was added.

Prepared contexts cover repository/HEAD, effective config and its origin files, remote URLs,
tracking refs and shared metadata identities. Existing read/worker helpers are reused with
context validation and explicit lease paths. Fetch/Push preserve dirty source buffers and permit
source saves; Pull additionally guards/leases the worktree. Shared unsaved/read-only/IME buffers,
retained save preparation and active metadata writes are covered. Leases outlive window clearing.

Six new real-Git tests and ten GPUI tests cover local bare-remotes, fast-forward and configured
ff-only/rebase divergence/conflicts, normal push/upstream and rejection, native signing/hooks,
noisy bounded diagnostics, stale/cancelled forms/results, detached/unborn/no-upstream, no-contact
configuration, included config outside the repository and linked common metadata. Self-review
produced failing regressions for included-config protection, stale running-progress cleanup,
and Git's remote mirror setting overriding `--no-mirror`; each is corrected. An invocation-local
mirror=false setting preserves ordinary non-force branch pushes while retaining signing/hooks.

Final affected suites pass:74 Git and1151 UI tests,25 existing UI tests ignored. The report contains
actual logs, intermediate failures and final strict/static results. G01 remains unchecked for
controller review. Cross-process changes remain a preflight/native-lock boundary; URL edits use
two config writes and identify partial success. Cancellation applies before submission; running
Git completes normally with leases retained. Global workspace/release/performance and physical
native checks remain controller gates; no release/performance build or product Git mutation ran.


G01 review fix1 specification: included config targets are protected even when they contain
only comments/whitespace or do not yet exist. Use Git's native include traversal and --path
expansion, resolve relative include paths against their declaring source, and canonicalize
existing ancestors for future saves. Conservatively guard declared conditional targets from
files Git traverses, including currently inactive targets, without reimplementing conditions.
Missing optional includes remain valid. Existing UI guards/leases consume the fuller path set;
no transport/default/UI route changes. Add real backend relative/tilde/nested/conditional/missing
regressions and actual GPUI empty-include unsaved refusal and worker-lifetime/save regressions.
Also retain partial-success context if launching the second URL-config process fails.


G01 fix1 implementation now includes declared empty/missing include targets in the existing
unsaved-document guard and worker lease path set. Native Git supplies parsing, traversal and
path expansion; a bounded filesystem resolver aligns future paths and dangling aliases with
save paths. Conditional targets from traversed files are guarded conservatively; optional
missing paths, including non-directory intermediates, remain valid. Four real backend cases
cover relative/nested/tilde/conditional, Unicode/newline filenames, missing paths, stale creation
and dangling aliases. Two actual GPUI regressions first failed, then proved unsaved settings
refusal and worker leases across window clearing, including a later real transport-config save.
The blank push-URL clearing launch error also retains explicit fetch-URL partial-success context.
No new dependency, transport/default change, or edits to another feature owner were introduced.
Full Git/UI and strict/static final evidence is appended to task-17-report.md; G01 stays unchecked.

G01 controller acceptance: fresh scoped re-review confirms include target guards/leases and
partial-success diagnostics with no new breakage. All 383 frozen-file hashes match. Final
78 Git tests, 1153 UI tests (25 ignored), doc tests, strict Git/UI Clippy/fmt/diff pass. G01
is complete at the feature gate. Explicit remote/upstream controls replace the local-only
policy; no automatic transport or real-remote verification. Global workspace/release/performance
checks and broad integration review remain pending; no overall completion claim yet.


## Final integration correction I2 / T02 specification (before edits)

Relative output links use the directory attached when their characters were printed, starting
with the launch directory and changing on valid OSC7. Live scrollback, search snapshots and
command review preserve that provenance through native vt100 layout, scroll, wrapping, resize,
reset and eviction. A token spanning different/unknown provenance fails closed; absolute and
web links remain usable. Retain at most 4096 directory identities per parser, never reuse IDs,
and drop old provenance rather than guessing. Existing output/command retention stays bounded.
Use a parallel vt100 attribute grid driven by its existing vte parser protocol, not a new layout
engine. The direct vte dependency reuses the already locked version. Acceptance covers identical
Unicode relative diagnostics in two directories, actual different files, OSC7 split sequences,
soft wraps/search/review, reset/alternate screen and retention/resize boundaries.

I2 implementation/verification: retained vt100 provenance now reaches live/search/command review,
including resize. Latest-cwd limitation is superseded in README. 27 terminal tests pass (one
release gate ignored), plus 11 terminal GPUI tests. Two new regressions cover Unicode directory
identity, identical diagnostics, search/review/wrap/resize/eviction and unknown/mixed metadata.

## Final integration correction I1 / E04–E05 specification (before edits)

Periodic crash recovery must not call native unmark or alter the live rope/history/composition.
For valid overtype/virtual-space previews, recovery excludes the provisional preview and its
padding, retaining the pre-composition source and selections. Ordinary insertion previews and
invalidated target metadata retain their existing insertion semantics. Snapshot lookup includes
shared views in other windows; no snapshot may retain the duplicate preview/target representation.
Actual project replacement, Close Project, clear and hot Quit settle composition through the
existing native unmark handler before capture/removal. Move-only transfers stay noncommitting;
explicit Save policy is unchanged. Tests use native marked-text callbacks, Unicode/multiple
carets, selected targets/virtual padding, recovery roundtrips, project lifecycle with shared peers,
commit/cancel and undo/redo, including a move-only removal.

I1 correction verified: actual native composition regressions first failed (2), then pass (2),
including periodic storage roundtrip, shared-window snapshot lookup, selected/virtual multiple
carets, move-only transfer, cancel/commit/Undo/Redo and actual project close/switch/hot quit.
Existing overtype (17), virtual (17) and project (11) GPUI checks pass. Snapshot policy is now
specified in Text Essentials; explicit Save remains unchanged.

## Final integration correction I4 / F03 specification (before edits)

Find in Files Apply checks the current Follow mode of every matching open document across
windows before routing either a buffer edit or disk write. Canonical paths and file identities
cover aliases; a followed target is skipped with an explicit Stop Following Log instruction,
including when Follow began after preview. File bytes, shared text and undo history remain
unchanged. Stop Follow permits normal replacement after refresh. Source-derived read-only
working-copy policy is unchanged. Acceptance covers disk and Use Buffer routes, Follow before
and after preview, another window, aliases, unchanged undo, and successful replacement after Stop.

I4 correction verified: 12 route/timing/alias combinations in the new GPUI regression first
reproduced a disk write, then all pass with unchanged bytes/text/history and normal replacement
after Stop. All 35 search and 17 Follow GPUI tests pass. The centralized apply guard and user
documentation now agree on the shared mode restriction.

## Final integration correction I3 / G01 specification (before edits)

Remote preparation adds applicable top-level global/XDG/system configuration roots even when
empty, comments-only or absent. Native `git var GIT_CONFIG_GLOBAL` / `GIT_CONFIG_SYSTEM` discovery
supplies environment-sensitive paths (including no-system/global overrides); the existing path
identity resolver, unsaved guards and worker-owned durable leases consume the resulting set.
Unsupported native discovery fails with an actionable error instead of silently dropping roots.
Ambiguous newline-delimited global discovery fails closed. No dependency or config parser added.
Tests run in child test processes with disposable HOME/XDG/global/system roots, never mutate
process-global environment, and never read/write the user's configuration. Cover comments-only,
missing and override/no-system roots, native discovery, dirty refusal, retained worker lease
through project clear/release, and unrelated source saves still permitted. Existing include,
signing/hook and disposable local-remote coverage remains required.

I3 correction verified: native root-discovery regression first failed for absent HOME config,
then passes across 11 isolated child environments. The actual UI dirty/lease regression passes
in three more isolated child environments, including native local transport gated after validation,
project clearing, deferred root saves and an allowed source save. Git 2.42+ and fail-closed newline
root behavior are documented; NUL-delimited includes and ordinary local Git remain unchanged.

Final self-review tightened I2's destructive-layout boundary: vt100 clips narrower visible rows
and clears their soft-wrap flags on resize. Directory IDs now identify print spans (paths share
Arc storage), bounded by retained capacity + screen rows + 4096. Resize invalidates affected
spans only, preserving complete lines. A private first-cell marker detects evicted prefixes;
command text-limit truncation likewise drops the incomplete head's directory. Historical row
widths remain usable in links/search/review after resizing. Unknown/mixed combining continuations
and numeric ID exhaustion fail closed. This supersedes the earlier 4096-directory-only bound.

Final provenance bounds also cap changed-directory epochs at 4096 (same-cwd output shares one
Arc path), and command-review metadata at a conservative 1 MiB in addition to its existing text
budget. Expiration drops relative links, preserving copied output and absolute/web links. A
5000-line/default-scrollback regression confirms routine same-directory output remains linked
past 4096 lines, through benign resize and search. Save Project As preserves live composition;
actual restore settles before applying settings, with native-handler recovery regression coverage.

Final consolidated correction verification: affected package suites pass Git 79, terminal 32
(1 ignored), UI 1157 (25 ignored); no failures. Strict affected-package all-target Clippy and
workspace formatting check pass. Final report records native-handler, subprocess-only Git and
terminal provenance regressions; parent retains release/performance gate ownership.

## Final symbol performance correction specification

No product defaults or commands change. The unchanged 100,000-line symbol fixture fails its
3,000 ms budget at 3,097.714 ms. A separate native sampling run confirms most symbol time
is repeated syntect parsing through the existing transition cache, not selector classification;
the earlier selector-only hypothesis is set aside without a code change.

Investigate collision churn in the cache's modulo-reduced keys. Preserve exact parse operations
and state, cancellation/checkpoint behavior, and all existing bounds: 16,384 entries, 4 MiB
transition payload, 256 shared states and 16 KiB captured-state admission. Use full hash keys
with the existing text/state equality guards and explicit entry admission if the collision
regression proves the gap. A full cache may decline new entries; it must never exceed bounds.

Acceptance: two distinct lines colliding in the old reduced slot are both reused on repeats,
with operations/state identical to fresh parsing; existing symbol, documentation and memory
bounds tests pass. Measure the exact release fixture before/after with its unchanged workload
and three-second budget; keep only a measurable improvement. Run final workspace/performance
gates after the complete correction delta is reviewed.

Final focused result: the collision regression failed with zero reuse before the change;
all 165 syntax tests then passed (3 existing ignored), including exact operations/state, docs,
checkpoints and memory bounds. Exact release symbol scans passed at 2,982.222 and 2,992.533 ms
after the 3,097.714 ms failure, against the unchanged 3,000 ms limit and 100,000-line rope. The
cache correction is retained; full final workspace/performance gates remain pending. The old
20k label was corrected to 100k, with no workload, sample-count or threshold change.

## Final acceptance — 2026-10-30

All 27 authorized IDs are implemented, reachable, documented and verified in the requested
order. The whole-change review found four integration defects; one consolidated correction
pass and independent scoped re-review accepted historical terminal directory provenance,
coherent IME recovery/project lifecycle, Follow protection in disk replacement, and empty or
missing top-level Git config guards/worker leases. The measured symbol-cache correction was
reviewed with exact-output and unchanged memory bounds. No Critical or Important finding
remains. The original comparison audit is byte-identical to the initial historical copy.

The final runtime source was unchanged throughout the following fresh checks. Every Cargo
command used `CARGO_TARGET_DIR=<checkout>/target`; verification children isolated
host Git config, with dedicated subprocess fixtures explicitly testing temporary global/XDG/
system configuration. Git operations used disposable repositories and local remotes only.

| Command | Actual result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed, exit 0; 1.040 s before this documentation-only update |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | Passed, exit 0; 5.609 s |
| `cargo test --workspace --locked` | 2,389 passed, 0 failed, 29 ignored; 136.575 s |
| `cargo build --workspace --locked --release` | Passed, exit 0; 127.637 s |
| `ORLO_INDEX_TREE="$PWD/crates" cargo test --workspace --locked --release -- --ignored --skip real_language_servers_smoke --nocapture --test-threads=1` | All 27 selected gates passed, 0 failed; 175.658 s including compilation |

Formatting and `git diff --check` are run again after this record is written; their final
command records accompany the controller's handoff. The 29 ordinary ignored tests comprise
27 separately executed release gates and two explicitly excluded real-server smoke tests.
Existing third-party future-incompatibility notices remain unsuppressed.

Selected final measurements: symbols over the actual 100,000-line rope 2,993.624 ms / 3,000 ms;
folded scrolling 3.924 ms / 8 ms; comparison scrolling 3.236 ms / 10 ms; 1,000-caret typing
1.122 ms / 20 ms; Undo/Redo 16.302 ms / 30 ms; find-panel keystroke 37.128 ms / 300 ms. The
symbol margin is modest; no broad or cross-machine performance improvement is claimed.

Native PTY/UI checks and the native Print/PDF path were exercised during feature verification.
PDFKit verified document (14 pages), selection (1), numbered larger text (25), empty document
(1) and actual system-panel PDF (3), with representative pages/contact sheets visually checked.
No physical print job was sent. Fixture servers cover protocol behavior; physical IME hardware,
real credential services/remotes, physical printer drivers and Intel hardware were not tested.

Important defaults remain: file autosave, format-on-save, modelines and virtual space are off;
overtype starts off per view. Terminal defaults are `$SHELL` (fallback `/bin/zsh`), `-l -i` and
10,000 retained lines. Unicode warnings and bounded discarded-draft rescue are enabled.
Recovery, historical backups and explicit file autosave remain separate. There are no new
locked packages or version changes: four direct edges reuse `encoding_rs` (strict legacy
codecs), `sha2` (fold fingerprints), `git2` (native identity-bound stash locking), and `vte`
(decoded control/print events for the existing terminal layout engine's provenance).

Remaining documented bounds include supported-zsh automatic hooks; primary-selection range
formatting and no formatting on first untitled Save As; bounded fold/terminal/recovery records;
the exact 26-codec set; 100 MiB Follow snapshots; and native Git 2.42+ / unambiguous top-level
config paths for explicit remote controls. SHA256/reftable stash Drop/Pop fail closed before
application, ordinary Apply remains available, and existing UTF-8 Git patch limits remain.
Discard rescue is bounded to 20 entries, 1 MiB each, 8 MiB total and seven days; its synchronous
support-volume I/O is a disclosed nonblocking latency follow-up. Native multi-file Git and
uncoordinated external writers retain the documented transaction limits and truthful errors.

F05, F06, E07, E08, E09 and G05 remain deferred. No product commit, push, PR, publication or
release was performed; this worktree and its private verification evidence are preserved.

## Expanded phase — N01 behavior contract (2026-10-30)

URI Component Encode / Decode extends Edit > Convert and the shared command palette,
with serialized codec `uri_component` also available to existing conversion actions and
text pipelines. Encoding uses UTF-8 bytes and uppercase `%HH`, preserving only ASCII
letters, digits and `-._~`. Decoding validates every escape and the resulting UTF-8,
keeps literal `+` and unescaped whitespace, and accepts lowercase hex escapes.
Each nonempty physical selection transforms independently; carets and pure virtual ranges
among selections keep their endpoints, mapped through neighboring edits. The non-line
transform target flag now skips empty physical ranges; line transforms retain their policy.
URI conversion with only carets (including pure virtual ranges) changes nothing; existing
Base64/hex whole-document fallback remains. All targets validate
before one undoable edit, and invalid input leaves text, selections and history unchanged.
Existing read-only working copies remain editable; Follow keeps its stronger editing lock.
The URI guard checks selections after shared-document catch-up. There is no asynchronous
result or new setting. Acceptance includes exact `a b/é` ↔
`a%20b%2F%C3%A9`, literal plus, emoji, multiline/multiple selections, `%`, `%GG`, invalid
UTF-8, shared-view mapping and one-step Undo/Redo. Canonical N01 remains unchecked pending
verification and independent controller review.

N01 implementation verification (pending independent review): full editor suite 690 passed;
URI GPUI/action checks 5 passed; conversion/catalog checks 4 passed; decode-action checks
4 passed; strict editor/UI all-target Clippy passed. RED/GREEN logs include malformed codec
support, selected-text-only fallback, mixed virtual endpoint retention, and same-update peer
selection collapse. Exact commands and limits are in
`.superpowers/sdd/editor-expanded-features-plan/task-1-report.md`. Canonical N01 remains
unchecked; final global/release gates remain the controller's responsibility.

### Expanded phase N01 accepted

N01 passed independent spec/quality review with no findings. The canonical checklist is now
1/64 complete; every other selected entry remains unchecked against the updated criteria.
Final evidence: 690 editor tests, 5 URI UI checks, 4 existing conversion/catalog checks and
4 decode checks; strict editor/UI Clippy, fmt and diff checks passed. Mixed virtual endpoints
and stale shared-view selection regressions were reproduced and corrected. URI's strict codec
retains decoded bytes; document insertion keeps existing logical-line normalization, including
CRLF to LF. N25 separately specifies lossless file-boundary preservation.

## Expanded phase — N02 behavior contract (2026-10-30)

HTML Entity Escape / Unescape extends the existing Edit > Convert menu, shared palette,
serialized conversion action and pipeline codec as `html_entities`. Escape replaces exactly
`&`, `<`, `>`, double quote and apostrophe with `&amp;`, `&lt;`, `&gt;`, `&quot;` and `&#39;`.
Unescape recognizes those names and decimal/hex Unicode scalar references (ASCII digits,
required semicolon, x/X hex prefix), rejects malformed/overflow/surrogate numeric input,
and preserves unknown names. Decoding consumes source once: `&amp;lt;` becomes `&lt;`.
Literal Unicode, plus and whitespace remain unchanged in the codec; normal logical-LF
insertion remains. Styled Copy as HTML stays independent. No setting or dependency changes.
Actions require current nonempty physical selections after catch-up; no targets means no
edit/history. Multiple targets validate before one undoable transaction, preserving mixed
empty/virtual endpoints. Invalid later targets leave text, selections, version and history
unchanged. Existing read-only working-copy and Follow/viewer editing policy remains.
Acceptance covers all five characters, numeric boundaries and invalid forms, unknown names,
single-pass decoding, multiline/multiple selections, Undo/Redo, shared-view mapping, stale
peer selections, no-target no-op and Follow lock. N02 remains unchecked pending independent
controller review.


N02 implementation verification (pending independent review): full editor suite 695 passed;
HTML entity UI/action/palette checks 6 passed; accepted URI UI checks 5 passed; conversion,
decode and independent Styled Copy as HTML checks 4/5/7 passed. Strict editor/UI all-target
Clippy passed, with formatting/diff checks recorded in the task report. RED/GREEN logs prove
missing codec/catalog support, no-selection document fallback and same-update shared-peer
selection collapse before the existing guard was extended. Actual commands, logs and limits:
`.superpowers/sdd/editor-expanded-features-plan/task-2-report.md`. N02 remains unchecked;
canonical progress stays 1/64 until independent controller review.

### Expanded phase N02 accepted

N02 passed independent spec/quality review with no findings. Canonical selected progress is
2/64. Final checks: 695 editor tests; HTML UI6, URI UI5, convert4, decode5 and separate
Copy-as-HTML7; strict editor/UI Clippy, fmt and diff passed. The small single-pass entity
parser validates numeric Unicode scalars before any edit and leaves unknown names literal.
No new dependency, setting or persistence schema was added.


## Expanded phase — N04 behavior contract (2026-10-30)

Subword navigation extends `Motion` with serialized `subword_left` / `subword_right`,
reusing `editor::Move` and its select flag, and adds `editor::DeleteSubword {forward}`.
Edit > Subwords and the shared command palette expose all six actions; custom keymaps
can bind them without replacing ordinary word shortcuts. No default bindings/settings change.
Source-order segments split lowercase-to-uppercase, acronym-to-title, letter/Unicode-number
transitions, and runs of underscores, whitespace and punctuation. Grapheme clusters remain
whole (including combining marks, ZWJ emoji, long flag runs and oversized clusters).
Expected char boundaries: camelCase [0,5,9], HTTPServer [0,4,10], snake_case [0,5,6,10],
item42Value [0,4,6,11]; navigation from boundaries is symmetric.
Movement/select uses existing row/fold/wrap/reveal and selection history paths. Deletion
removes selected physical text or the adjacent segment, merges overlapping caret targets
through the existing atomic edit engine, and supports one-step Undo/Redo. Pure virtual
positions never materialize padding or delete unselected source (backward carets first
collapse their gap); empty/end targets are no-ops. Existing shared catch-up, Follow and
immutable-viewer locks, native IME commit policy, snippets and overtype remain in force.
No asynchronous work or stale result is introduced. Acceptance includes exact/reverse
boundaries, Unicode extremes, selected and overlapping multicaret deletion, ends, history,
real catalog/JSON/keymap dispatch, shared-view mapping and locked sources. N04 remains
unchecked until independent controller review.


N04 implementation verification (pending independent review): full editor suite 703 passed;
subword UI/action/palette checks 6, editor UI motions 10, accepted URI 5 / HTML 6 and keybindings 2
passed. Strict editor/UI all-target Clippy, formatting and diff checks passed. Both required
serial exact release gates selected and passed one test each: typing/undo 0.003/0.111/1.214/15.130 ms
against 1/3/20/30 ms budgets, and find-panel input 35.786 ms against 300 ms. Frozen Rust source
hashes are unchanged through final checks. All-offset programmatic movement exposed an interior
grapheme edge; targeted deletion RED proved a surviving ZWJ fragment before subword endpoints
were made cluster-safe. An accidental pre-existing HTML fixture edit was restored verbatim and
all accepted URI/HTML tests pass. Exact APIs, named tests, actual command/exit/count logs,
self-review and limits are in `.superpowers/sdd/editor-expanded-features-plan/task-3-report.md`.
N04 remains unchecked; canonical accepted progress remains 2/64 until controller review.


## Expanded phase — N22 behavior contract (2026-10-30)

File > Save a Copy As… and File: Save a Copy As… capture the selected document
after native destination selection and validation, before any asynchronous wait. The
immutable rope, output encoding/BOM/current line ending and history preference are
authoritative; no formatter, cleanup, native unmark, typing-group split, source saved
callback or rebind runs. Like explicit Save, the currently visible native-input rope
is captured; recovery-only provisional-input exclusion does not apply. Follow refuses
the action. Named dirty, untitled, read-only and shared documents retain source state.

The native save panel owns overwrite consent. The accepted destination stamp (including
absence), resolved path and device/inode are checked again before safe publication.
Source canonical/symlink/hardlink aliases and any other live document owner (even
retained or in another window) refuse both at capture and immediately before writing.
Destination turns and file/path leases serialize accepted copies with existing writers;
accepted work retains its document and close/quit waiters until it lands. Cancellation,
lossy encoding, stale destination and I/O errors report status without source changes.

Acceptance covers real action/menu/palette routes, named/untitled Unicode copies,
Encoding::ALL (26; legacy codecs only where representable), BOM and LF/CRLF/CR, undo/redo
and grouping, paused source edits/format/path changes, destination aliases/owners/leases,
overwrite races/errors/cancel and source close. Ordinary Save/Save As, autosave/recovery
and Explorer Duplicate disk semantics remain covered. Mixed-newline sidecars belong
to N25; this captures today's format metadata without changing that future contract.

N22 implementation verification: 707 editor tests, 59 UI document-save tests (including
12 copy regressions), 13 shared-document tests, 20 Save As compatibility tests, 8 recovery
tests and the Explorer Duplicate disk-content regression passed. Strict affected editor/UI
all-target Clippy, workspace fmt check and diff check passed. Full commands, causal RED/GREEN,
known filesystem boundary and snapshot/freeze evidence are in
`.superpowers/sdd/editor-expanded-features-plan/task-4-report.md`. Canonical N22 remains
unchecked pending fresh controller-owned independent review.


N22 fix round 1 contract: document admission must honor both destination path and inode
leases through publication, including pending Open/Reopen/session reads. A disk read
invalidated by a matching Orlo writer reservation is discarded and reread after leases clear
(external appends retain ordinary read/polling policy), inside the existing navigation/generation/focus continuations.
Recovered unsaved session text and metadata remain in pending restoration/capture until
admission; deferring a named recovery must never replace it with new disk content.
Acceptance pauses after copy's last owner preflight and after a real background read,
then exercises hardlink Open, pending same-path Open, Reopen and disk/dirty recovery
restore, cancellation and the existing save/open/restore/recovery/lease checks.

N22 fix round 1 verification: the hardlink Open and pending read cases failed after the
last copy preflight, then passed with shared path/inode admission and short-lived Orlo
read tickets. Ten post-preflight regressions now pass, including external appended-log
admission/Follow, templates after failed copy, pending Reopen, recovery sharing/capture
IDs, native close and project routing. Full amended UI tests: 1,196 passed / 25 existing
ignored; strict affected editor/UI all-target Clippy, fmt and diff passed. Editor model
and codec sources match the reviewed v1 hashes and were not retested unnecessarily.
Fix evidence is appended to `task-4-report.md`; both full and scoped fix patches/freezes
are supplied. Canonical N22 remains unchecked for fresh independent scoped review.


## Expanded phase — N16 behavior contract (2026-10-30)

Selection > Expand Syntax Selection and Shrink Syntax Selection are recorded editor actions,
shared with the palette and reconstructible by user keymaps; no new default binding replaces
Smart Expand or another command. Expand uses only the current bundled Tree-sitter snapshot,
selecting the smallest enclosing named node whose source range strictly contains the current
range. Equal wrappers and zero-width/missing nodes are skipped. A caret uses the character on
its right; at EOF it uses the final character on its left. Nonempty selections use half-open
ranges. Empty text has no larger range. Parse errors do not disqualify valid named ranges.

Every caret expands independently; a caret with no larger range stays put when others can
grow. Ordinary selection normalization merges overlaps. Shrink reverses each successful step
exactly, including direction, primary selection, goals and virtual endpoints before merging.
History is transient and per view, tied to text instance/version, parser identity/revision,
expected selections and the selection checkpoint generation. Unrelated movement (including
away and back), text/syntax changes, unavailable/stale/cancelled parser state invalidate it.
Shared views catch up before querying. Immutable/Follow views may select. Native composition,
fold reveal, wrapping and scroll use existing selection-command policy. No source bytes, dirty
state or text-undo steps change.

If a supported parser is not current, queue existing highlight work and leave selections
unchanged with a useful retry status; never retain an asynchronous selection result. Unsupported
languages/oversized files and no-larger-node/empty shrink history report status unchanged.
Acceptance covers identifier → expression → statement and exact reversal, strict/equal ranges,
Unicode and combining marks, reversed/multiple/overlapping/virtual selections, malformed/empty/EOF,
current/stale/cancelled parsing, edits/carets/syntax changes during work, shared/locked views,
folds/wrap/composition, ordinary selection history and unchanged lexical Smart Expand.


N16 scoped verification: 709 editor, 169 syntax and 1212 UI tests passed (3 existing syntax
and 25 existing UI tests remain ignored). The 22 focused N16 tests, 28 existing fold tests
and lexical Smart Expand regression passed. Strict affected syntax/editor/UI all-target
Clippy, fmt and diff checks passed. Both mandatory exact serial release input gates selected
one test each: typing/undo 0.003/0.094/1.625/15.687 ms against 1/3/20/30 ms budgets;
find-panel input 36.985 ms against 300 ms. Related exact release warm-up and highlight-frame
gates also passed. Source hashes remained frozen through the final checks. Causal RED/GREEN,
current-tree/identity/history APIs, actual commands/counts/raw logs, warm-cache compatibility
and limits are in `.superpowers/sdd/editor-expanded-features-plan/task-5-report.md`.
Canonical N16 remains unchecked pending fresh root-owned independent review.

## Expanded phase — N17 behavior contract (2026-10-30)

Enter in Python, Rust, JavaScript and TypeScript uses the selected syntax and resolved
indent width, tab width and tab policy. Python block colons and dedent keywords, and
unclosed brace or continuation contexts in all four languages, affect leading whitespace;
string and comment contents do not. Incomplete code is supported. Enter retains the
existing single undo step, selection replacement and brace-pair splitting. Unsupported or
unavailable language context retains the existing newline behavior. Ordinary character typing
must not scan the entire document.

Edit > Line > Reindent Selection and its palette/keymap action operate on selected logical lines
(or caret lines), merge overlaps and exclude a final line selected only at column zero. The
operation computes and validates all leading-whitespace edits against a single source before
one undo transaction; it never modifies nonwhitespace text or invokes formatting. Shared
views catch up before calculation; Follow and immutable snapshot views refuse mutation.
Lossily decoded read-only documents retain the existing editable working-copy behavior.
No new default shortcut conflicts with existing bindings. Existing Tab, indent/unindent,
paste, snippets, overtype, virtual carets, native composition and formatting remain coherent.

Acceptance includes Python nesting/else/dedent, brace and continuation nesting, strings and
comments including multiline, malformed source, tabs and independent widths, Unicode,
multiselection/reversed/overlap/end boundaries, Undo/Redo, shared/native/locked views and
unsupported languages. Exact heuristic limits and unavailable-context behavior must be recorded
with implementation evidence before this contract is accepted.

Context bound: analyze at most the first 64 Ki characters through the target position; if
that prefix exceeds the bound or language context is unavailable, Enter falls back to the
existing newline rule and Reindent Selection declines without an edit. This conservative
limit avoids guessing a multiline string/comment state from an arbitrary truncation point.


N17 scoped implementation verification: 726 editor, 169 syntax and 1,221 UI tests
passed (3 syntax and 25 UI existing ignored); strict affected all-target Clippy and fmt
passed. Exact checks, heuristic bounds, review-found regressions and release input results
are recorded in `.superpowers/sdd/editor-expanded-features-plan/task-6-report.md`.
Canonical N17 remains unchecked pending fresh controller-owned independent review.


## Expanded phase — E01 behavior contract (2026-10-30)

Reuse the bundled Tree-sitter worker and its current immutable-rope snapshot for discovered
brace/bracket, multiline-comment, JSX/TSX and supported keyword-block folds. Fold, Fold Level
and gutter discovery share those candidates; syntax endpoints preserve following source text.
Dirty, cancelled, changed-text or changed-syntax results cannot supply coordinates. Unsupported
syntaxes, unavailable parser/rule data, pending parses and sources above the existing 16 MiB
UTF-8 bound retain indentation discovery. A current empty syntax index is authoritative, so
indented string contents do not become code folds. No new parser, query platform or setting.

Candidate normalization must be deterministic across input order: choose the outermost range
per header, deduplicate, retain nested headers and reject crossing candidates. Recovered parser
nodes in incomplete/malformed text may supply bounded current-source ranges; errors/missing
nodes themselves are excluded. Manual inline selections remain exact; multiline selections,
nested active folds, edit mapping, source bytes, undo, selections, native IME and per-view folds
retain their existing behavior. No persistence/schema work belongs to E01.

Acceptance proof covers braces and Unicode/CRLF endpoints, multiline comments, JSX/TSX,
nesting, malformed/incomplete text with complete neighboring structures, deterministic duplicate/
crossing normalization, unsupported/pending/oversized fallback, stale/shared edits and Undo/Redo,
manual inline folds, gutter/level actions, wrapping and native composition. Existing behavior
is verified rather than replaced; add only absent regression coverage or a demonstrated fix.


E01 scoped verification: production paths already satisfy the entry. Missing direct coverage
for malformed/incomplete neighbors and duplicate/crossing input-order normalization now passes.
Full suites pass 726 editor, 170 syntax and 1,221 UI tests (3 syntax/25 UI existing ignored);
35 focused UI fold checks and strict affected Clippy/fmt/diff pass. No runtime code changed,
so no new release timing run is needed. Actual commands, coverage and frozen task delta are
in `.superpowers/sdd/editor-expanded-features-plan/task-7-report.md`. Canonical E01 remains
unchecked pending fresh controller-owned review.


## Expanded phase — E02 behavior contract (2026-10-30)

Keep the existing optional `ViewState::folds`, version-1 exact character ranges and SHA-256
fingerprint of the entire decoded rope. Capture is per view, only after its shared revision
catches up, with no new setting; bounds stay 4096 active ranges, 16 MiB decoded UTF-8 and
32 closed-view records per window. Unchanged captures reuse the immutable-rope fingerprint;
painting/input must not hash persistence text. Restore through existing session/project and
Reopen Closed/Open paths against the document actually attached after sharing/recovery.

Old missing fields, malformed optional records, unknown versions, invalid/duplicate/crossing/
unsorted/out-of-bounds/oversized ranges and changed content restore expanded. Ignore only fold
metadata; retain recoverable dirty text, buffer sharing and selections. Closed history owns
folds and decoding hints only, never unsaved text or stale document preferences. Preserve
N22 pending recovery payloads/read tickets/capture IDs/close protection and N16/N17 behavior.

Acceptance covers close/reopen of two independent views, nested and exact inline folds,
saved Unicode/CRLF/BOM bytes, dirty hot exit and actual project workspace reopen, changed
disk and attached shared-model mismatch, old/corrupt optional fold data without recovery
loss, read-only/Follow and native composition lifetime. Native provisional recovery text
is excluded by the existing snapshot path; folds for that different live rope are omitted.
Inspect named existing coverage before adding only absent acceptance combinations; no
persistent undo, history framework, replacement folding engine or serialization flow.


E02 scoped verification found no production gap. New acceptance combinations cover actual
project dirty shared-view close/reopen, corrupt optional fold records preserving recovery and
valid sibling folds synchronously and after a write lease, read-only/Follow shared reopen,
and native recovery fold omission while live composition remains intact. Fresh full suites
pass 726 editor, 111 workspace and 1,223 UI tests (25 existing UI ignored). Focused persistence
13, native recovery 2 and fold geometry/input 35 pass, along with strict affected all-target
Clippy/fmt/diff. Only tests and documentation changed; production is equal to snapshot8, so
no runtime performance repeat is needed. Details and frozen evidence are in
`.superpowers/sdd/editor-expanded-features-plan/task-8-report.md`. Canonical E02 remains
unchecked for fresh controller-owned independent review.

## Expanded phase — E03 behavior contract (2026-10-30)

Warnings now opt in: `unicode_warnings` defaults to false in resolved preferences and
standalone editor views. Existing scalar-keyed `unicode_warning_characters` and their 21
default descriptions/glyphs remain compatible. Add independent per-file
`unicode_warning_ranges` (default `{}`), mapping `U+XXXX` or inclusive
`U+XXXX..U+XXXX` keys to the same glyph/description descriptor. Hex digits may use either
case, with 4–6 digits and literal uppercase `U+`; no whitespace, reversed bounds,
surrogates, surrogate-crossing intervals or values above U+10FFFF are accepted. Explicit
scalar entries win over ranges; overlapping ranges use greatest numeric start, then greatest
end, regardless of JSON object order. Supplied maps replace that map at the ordinary
global/language/path/scope/project settings layer. Bad maps/glyphs report normal recoverable
preference warnings and retain valid fallback settings. No range enumeration or Unicode-name
database is added. A settings-time boundary sweep compiles disjoint lookup intervals in
O(N log N) time/O(N) space; per-scalar paint lookup is logarithmic in configured intervals.
Character Info shares lookup with paint and displays configured descriptions
and actual U+ codes even when marks are disabled.

Painting remains synchronous and source-coordinate based over clipped, non-folded visible
scalars, retaining independent warning ink and whitespace marks. Active IME ranges remain
unannotated until commit; setting refresh cannot mutate text, selections, native input
positions, versions or undo. No async warning state exists to become stale. Acceptance covers
NBSP/U+200B/U+202E, ordinary and configured combining sequences, a private-use user scalar,
range endpoints/gaps/overlaps and malformed ranges/scalars/glyphs; resolved scoped/project
settings and Character Info; unchanged clipping/folds, caret/selection/wrap/hit/IME geometry,
copy/save bytes, shared views and undo. Existing enabled-default fixtures will explicitly opt
in while retaining their assertions. Native OS/hardware limits will be recorded.


E03 expanded implementation changes warnings to opt-in/default-off and adds validated
singleton/inclusive-range maps, preserving the existing scalar map, glyph/color policy,
Character Info and geometry. A settings-time boundary sweep produces disjoint lookup intervals;
visible paint never enumerates range members. Causal config and actual Character Info
regressions first failed, then passed. Final full suites pass 67 config, 726 editor and
1,228 UI tests (25 existing UI ignored); focused config selects 9 (8 warning-owned plus one
existing modeline), and UI warning coverage selects 12. Strict affected all-target Clippy and
workspace fmt pass. Exact serial release typing, folded-file scrolling, comparison scrolling,
highlight-frame, Find-panel input and enabled expired-range stress checks each select one
passing test with unchanged workloads/budgets. Fold/comparison scrolling measure 2.793/2.431 ms
against 8/10 ms budgets. Compiled 4,096-expired-range lookup over a clipped 200,000-scalar row
measures ten release frames at 0.240333 ms for misses and 0.749542 ms for lower-start live
matches. Tests preserve whitespace marks, shaped/caret/selection/wrap/hit/native-IME positions,
copy/save bytes, shared views and undo. No physical OS IME/font smoke was performed. Full logs,
self-review, syntax/defaults, limits and frozen task delta are recorded in
`.superpowers/sdd/editor-expanded-features-plan/task-9-report.md`. Canonical E03 remains
unchecked pending fresh controller-owned independent review.

Expanded E03 review fix contract: reject duplicate numeric range intervals even when keys differ by hex padding/case or singleton versus equal-endpoint syntax. Both JSON orders and direct/normal/scoped preferences must report the collision; normal preferences retain unrelated settings and fall back safely. Canonical nonduplicate serialization round-trips unchanged. No paint/index/geometry/IME/save change. Cover causal direct and preferences regressions, full affected tests and strict checks before scoped re-review.

Expanded E03 review fix evidence: two direct/normal/scoped duplicate-alias regressions failed on silent acceptance, then passed after MapAccess validation rejects repeated numeric intervals before index compilation. Post-fix full69config/726editor/1228UI(25ignored) and strict affected Clippy/fmt pass. UI/index/lookup/input and benchmark code remain unchanged; six earlier exact serial release gates remain hot-path evidence. Final diff/freeze and fresh scoped review required before canonical acceptance. Original v1 evidence preserved.


## Expanded phase — F01 behavior contract (2026-10-30)

Default opens remain BOM-detected Unicode, otherwise UTF-8, with the existing exact 26-codec
catalog and named BOM-less disk hints. Reopen with Encoding keeps its current dirty-buffer,
saving, Follow, identity/revision/metadata and disk-stamp guards. Save with Encoding, ordinary
Save/Save As, immutable Save Copy, autosave and closed-file conversions retain the common
lossless encoder, history ordering and safe writer. No dependencies, persistence schema,
encoding guesser, mixed-newline policy or N22 ownership/lease/read-ticket behavior changes.

Missing conversion detail is added at the codec owner: the first source Unicode scalar that
fails the encode/decode round trip is reported as a character and U+ code point, with one-based
line and column. Columns count Unicode scalars (including tabs/combining marks individually),
not UTF-8 bytes, UTF-16 units or display cells; LF, CRLF and lone CR each end one diagnostic
line. Diagnostics describe the text submitted for encoding after existing newline conversion.
The extra scan runs only after a failed legacy round trip or Latin-1 range check. Failure stays
InvalidData and occurs before history capture/write, preserving disk, source dirty/save state
and undo/redo. Successful-but-changing aliases such as Shift JIS yen still fail.

Expanded closed-input policy supersedes the earlier F01 search exception: Find in Files disk
search reports malformed/unknown input as unsupported instead of matching decoded substitution
text. Use Buffer remains explicit interpreted live text; protected open/reinterpret behavior is
unchanged. Comparison/history/pipeline already reject malformed input and keep their guards.
No new asynchronous operation is introduced; current stale/cancel guards continue to own entry.

Acceptance: causal regressions for Unicode-prefix emoji and legacy alias diagnostics with
LF/CRLF/CR and scalar columns, unchanged disk/history/source metadata and undo/redo on failed
Save/Save As/Save Copy/guarded save, reachable shared UI save error detail, disk-search refusal
(including a replacement-character query) and successful known-codec search. Existing all-codec
literal fixtures/Unicode BOM and no-BOM round trips, metadata/read-only/Follow/save-race,
shared/reopen dirty/cancel/stale tests remain covering evidence. Run focused regressions plus
full project/editor/UI suites, strict affected all-target Clippy, workspace fmt and diff checks.
Because disk-search admission changes, run the existing serial release filesystem/search/index
suite with its unchanged workload/budgets; no rendering/input hot-path change is intended.


### Expanded phase F01 implementation evidence (independent review pending)

Retained the existing exact 26-codec/BOM/disk-hint implementation. Failed round trips and
Latin-1 range checks now report the first source scalar, U+ code point and one-based scalar
line/column, treating LF/CRLF/lone CR as diagnostic line breaks. Successful conversions keep
their existing work; no per-scalar codec calls were added. The common error reaches ordinary,
Save As/Copy, guarded/autosave, replacement/pipeline and statistics/checksum callers before
history/writes. Disk Find now reports malformed/unknown text instead of searching substitution
text; explicit Use Buffer and known disk codecs remain available, with cancellation honored.
Writer/reopen/Follow/copy ownership/lease/read-ticket/recovery behavior is unchanged.

Causal diagnostic and real UI Find/Save REDs pass after the narrow owner fixes. Final actual
checks pass: 88 project, 728 editor, 1,230 UI tests (25 established ignored), five focused
expanded regressions, strict affected all-target Clippy, workspace fmt and whitespace checks.
All six unchanged serial release filesystem gates pass: 99 MB search 190.273/1500 ms,
100 MB load 87.557/1500 ms, 10 MB line load 4.345/300 ms, disk poll median 0.2015625/1 ms,
and UI save capture/finish each 0.000/1 ms at printed precision. The current crates tree
index (311 files) built in 7.0 s and completed its memory report (no numeric memory threshold). Workloads/budgets
are unchanged. Exact commands, causal versus fixture-failure records, covering test names,
self-review, limits and final diff/SHA freeze are in
`.superpowers/sdd/editor-expanded-features-plan/task-10-report.md`. No physical OS UI/IME
smoke is claimed. Canonical F01 remains unchecked pending controller-owned scoped review;
final whole-workspace/global/native gates remain outstanding.

Expanded F01 review fix contract: closed-file lexical Go to Reference uses strict FileText search admission and reports malformed/unknown input with its file path. Known closed disk-codec hints apply; explicit live-buffer search and existing cancellation/generation guards remain. Acceptance exercises a malformed closed file, a normal valid closed file, known legacy bytes, and live source text without changing disk bytes. Add causal UI regression before the callsite fix; rerun full affected and strict checks and preserve existing performance budgets.

Expanded F01 review fix evidence: actual GoToReference malformed closed-location regression RED1→GREEN1; known1252/validUTF8/live-source and unchanged bytes verified. GoToReference now uses shared strict disk admission, closed hints and path-context failures; cancellation/generation flow unchanged. Post-fix full1231UI(25ignored), strict UI all-target Clippy/fmt pass; unchanged core keeps88project/728editor results. Repeated six filesystem gates and fresh scoped review remain pending before canonical acceptance.

Expanded F01 fix1 final performance: repeated6serialfilesystemgates pass on amended source (Find191.734/1500ms,load87.914/1500ms,10MB4.397/300ms,pollmedian.2065/1ms,savecapture/finish.000/1ms,index7.1s/1800s timeout). Workloads/budgets unchanged; no numeric index-memory threshold claimed. Final diff/patch/hash readback and fresh scoped review precede acceptance.


## Expanded phase — L01 behavior contract (2026-10-30)

Before Task11 edits: verify the existing opt-in server completion route,
Control-space/Goto > Language Server > Complete through the actual completion picker. The
transport advertises snippetSupport, not InsertReplaceEdit/itemDefaults capabilities. Supported
insertTextFormat=2 text uses parse_lsp and the existing primary-caret snippet session: nested
fields, mirrors/transforms, Tab/Shift-Tab and $0; choice fields start at their first alternative
(N31 owns popup UI). Local multicaret snippets and plain completions remain unchanged. Primary
TextEdit/InsertReplaceEdit replace ranges and additional edits convert negotiated UTF-8/16/32
coordinates, validate before application, and insert as one shared-document undo step. Invalid
syntax/ranges/overlap report a status error without partial edits/history. View/epoch/full
selection/source/protocol/version/server guards reject stale or cancelled resolved responses,
while deliberate prefix typing on an open list keeps its existing range mapping. Read-only,
Follow and native composition protections remain intact. Exact acceptance to verify: fixture
server range/replace+Unicode+additional edits; nested fields, live mirrors/transforms and final
tabstop through the picker; shared undo/redo; atomic unsupported/overlap failures; delayed
resolve guards and existing typed-prefix/local-snippet/formatting coverage. Current runtime is
substantial; add meaningful missing combination coverage only unless a causal failure proves
a production gap. No runtime changes or performance-budget changes are planned.

### Expanded phase L01 worker verification (independent review pending)

The runtime already met L01. Task11 adds four acceptance-combination GPUI tests and two
fixture response overrides, preserving default fixture output. The actual completion picker
now has named proof for six UTF-8/16/32 range/replace cases, nested fields, mirrored live
Unicode typing/transforms, Shift-Tab/Tab/$0, Unicode additional edits and shared undo/redo.
Separate snippet cases prove deliberate prefix mapping, atomic unsupported/overlap failure
and paused-resolve refusal after read-only/Follow/composition/version changes. Full affected
suites pass 728 editor / 170 syntax / 1,235 UI (3 syntax + 25 UI existing ignored tests); focused completion 27,
server-snippet 5 and local/model snippet 64 pass. Nonzero protocol checks and strict affected
Clippy/fmt/diff are recorded with final hashes in task-11-report.md/freeze. Earlier harness
failures and a zero-test protocol selector are retained and not claimed as product regressions
or coverage. No runtime/dependency/capability/settings/performance changes; no real server or
physical OS smoke. Root owns independent review and final global gates; L01 remains unchecked.

## Expanded phase — L02 behavior contract (2026-10-30)

Explicit Signature Help remains available and capability-gated. Committed single-character input requests help only when the configured server declares that trigger; retrigger characters are sent with the protocol retrigger context while help is active. No provisional IME text initiates requests. The existing source, caret, view, server generation and viewport guards reject obsolete responses; empty results, errors, dismissal and context exit close old help. Display all valid signatures with the server-selected signature and active parameter, bounded plain documentation, and focus-safe keyboard navigation/dismissal without editing text. UTF-16 label offsets remain independent of document position encoding and malformed spans are ignored. Acceptance covers nested/comma triggers, retriggers, Unicode, multiple signatures/docs, empty/error/delayed results, keyboard and actual popup rendering, focus/IME and shared read-only behavior.

L02 design clarification: signature response cancellation uses the existing LSP cancel notification for an in-flight request and drops superseded jobs waiting for the manager; late IDs are ignored by the existing response router. Capture follows source-view catch-up and its ordinary selection checkpoint, then pins the selection generation, full selections, primary index, editor epoch and viewport. The checkpoint is verified not to add a selection-history or text-undo step. The command palette restores focus before the explicit request; Find/blur cancels pending help. Committed typing continues a pending context; non-trigger content changes, including closing delimiters, ask the server for the current call and empty results dismiss help. The client displays plain bounded documentation and uses Up/Down for overloads; Escape dismisses visible or pending help.

### Expanded phase L02 worker evidence (independent review pending)

Final affected suites pass 172 syntax and 1,253 UI tests (3 and 25 ignored); focused UI signature selector passes 22 and focused syntax passes 4. The two source-session/identity regressions, 50-trigger contention proof, nested exit, cancellation and stale-context tests are included. Strict affected all-target Clippy, fmt and diff checks pass. Four exact serial release gates pass with unchanged workloads/budgets: typing, UI keystroke, highlight frames and folded scrolling. Task-local report, logs, patch and hashes record the commands and limits; real-server and physical OS smoke remain unclaimed. Independent review and root global gates remain pending; L02 stays unchecked.

Expanded L02 review fix contract: distinguish initial/pending continuation from previously active help when classifying retrigger-only characters; bound retained protocol context/counts/aggregate before copying, preserving UTF-16 offsets/overload indices and sharing admitted immutable data across handoffs; give pending and visible help the same prompt source/full-selection/caret/viewport/focus cancellation and cleanup, scoped so stale completion cannot clear newer work. Acceptance adds delayed initial-open/comma, excessive raw payload/count/aggregate and Unicode selected-context tests, held-request mouse/selection/resize prompt release and no subsequent non-trigger resurrection. All three remain required before final checks/re-review.

### L02 review fix 1 contract

Pending-only input remains distinct from displayed active help: a retrigger-only character while the first request is pending sends a content-change continuation, while initial triggers retain trigger-character semantics. Retain at most 64 signatures, 128 parameters per signature and 256 KiB aggregate serialized protocol JSON (including unknown fields and parameter documentation); reject an excessive response whole before cloning/parsing and report the existing “No signature help available” outcome, preserving original indices and UTF-16 label offsets for admitted replies. Share admitted raw context immutably across input and pending ownership, copying only the bounded payload when constructing an outbound retrigger with the selected overload. Pending work pins the same source, selection generation/full selections, viewport and focus as visible help; context exit cancels and clears it promptly, and stale callbacks may clear only their own request. Regressions exercise held first-trigger/comma, bounded Unicode contexts, mouse/selection/resize cancellation and non-trigger typing after exit.

L02 fix1 worker verification: pending-only comma continuation, shared bounded context and cancellation/cleanup on mouse, selection and resize are covered causally. The held fixture session test verifies the transport releases the manager and that non-trigger typing cannot resurrect help; the regular workspace gate tests cover the actual callback path. A same-frame visible-selection regression and repeated explicit command regression protect the unified lifecycle. Final affected suites pass 174 syntax / 1,260 UI (3 / 25 ignored), focused UI 29 / syntax 6, strict Clippy/fmt/diff and four exact serial release gates. Independent scoped re-review remains pending; L02 is unchecked.

### Expanded L02 fix2 contract — explicit invocation starts fresh

An explicit Signature Help invocation discards prior visible/pending help before capturing
its new request. It never inherits activeSignatureHelp or the previous overload from a call
whose selection/source has changed, including selection plus invocation before a paint. A
following non-trigger edit while this first explicit response is pending continues through
ContentChange with isRetrigger false and no activeSignatureHelp. Automatic input retriggers
retain their freshly captured admitted context. Matching cancellation tokens still protect
newer work from old callbacks. Defaults, menus, protocol bounds and UI focus policy stay as
specified above. Acceptance: hold a displayed-call retrigger, change selection and explicitly
invoke before paint, type a non-trigger before the response, then assert the actual fixture
request context and final visible help; retain existing repeated explicit/automatic tests.

Expanded L02 fix2 verification: the new real-fixture request sequence failed with
isRetrigger true before repair and passed with false / no activeSignatureHelp after the
explicit-request-start reset. Existing automatic retrigger and repeated explicit tests remain
green: 30 focused UI cases, full UI 1,261 passed / 25 existing ignores. Strict UI all-target
Clippy and formatting passed; final diff check follows these documentation changes. Syntax
was unchanged from fix1's 174 full / 6 focused passes. Fix2 has four production lines in the
explicit command setup, no paint or ordinary editing change; fix1's four serial release gates
are historical evidence for those unchanged hot paths, with final global gates still required.
Independent scoped review is pending; L02 remains unchecked. Fixture GPUI/subprocess coverage
does not establish physical IME or every external language-server version.

Expanded L02 accepted after two review fix rounds. Fresh fix2 review approved spec and quality,
with the residual explicit request context ADDRESSED and no new finding. Root rechecked all
16 frozen changed files and 318 source/build inputs before changing acceptance metadata only.
Current UI proof remains 30 focused / 1,261 full passes (25 existing ignores); unchanged syntax
proof 6 focused / 174 full passes (3 ignores). Strict affected checks pass. The canonical audit
now checks L02; final workspace/native/release/global performance checks remain pending.


## Expanded phase — T01 behavior contract (2026-10-30)

Terminal-local Find retains literal, case-sensitive defaults; reachable Case and Regex buttons
and terminal-search keyboard actions change only that terminal's query. Next/Previous wrap
and reveal the current match, with source cell highlights and actual matched-source copy.
Matching joins soft-wrapped cells into logical lines; Unicode cells keep CJK/supplementary/
combining spans. Empty queries and zero-width regex matches produce no hits; invalid/oversized
regex reports an inline error, never shell input. Regex uses the already locked linear
regex-automata engine, bounded compilation and logical-line input; no backtracking dialect.

Query/options/session/size/screen-mode changes reject obsolete jobs. Output alone marks a
pending snapshot scan dirty rather than cancelling it forever; the next refresh follows its
publication. Snapshots are independent retained views, including output later evicted from
the live session. Close retains the exact displayed snapshot, scroll and valid selection;
coordinates are never restored into live cells. Typing, Paste or Terminal Live Output returns
to live output. Refresh/navigation clears snapshot-relative selection before changing views;
resize and alternate-screen changes discard incompatible retained views. A new Find starts
from current live output. Search actions never write PTY bytes.

Acceptance: causal regressions for case/regex/actual-copy, invalid/empty/zero-width patterns,
soft wraps and Unicode, literal specials/nonoverlapping candidates, pending output refresh,
eviction and close preservation; real PTY keyboard routing/no-input and visible navigation;
stale query/options/session/size/mode admission, existing mouse/links/provenance isolation.
Canonical T01 remains unchecked pending independent review.

T01 geometry/lifecycle refinement: reuse one fixed 28px local terminal row for Live output,
Find, retained output, command review/controls and backend error/exit status. This replaces
the conditional command/status footers, reserving 28px in the normal view so opening/closing
Find and first command records/exit cannot resize the grid or invalidate a valid selection.
Real window resize still resizes the PTY and discards incompatible snapshots. A completed
job owns a guarded delayed dirty refresh, so final output is caught after the PTY poll exits;
closing or replacing query/options cancels that job and cannot reopen Find.

T01 seed-owner refinement: opening Find from command or retained review explicitly starts from
current live retained output and clears the prior review's cell selection, rather than copying
those coordinates onto live cells. Opening directly from the same live view still carries its
valid pre-Find selection/scroll. The real OSC133 fixture reproduced the command-review bug
before this narrow entry guard and now covers both review transitions.

### Expanded phase T01 worker evidence (independent review pending)

Final affected full run: 36 terminal tests pass (1 ignored), 1268 UI tests pass (25 ignored),
including 8 terminal-search UI regressions and real PTY/OSC133 fixtures. Strict affected
all-target Clippy, workspace fmt and whitespace checks pass. Causal REDs cover case/invalid
regex, Close scroll loss, pending-output starvation, final-output catch-up after exit, and
command-review selection crossing into live Find. Exact-byte shell input proof rejects query
or mouse-report bytes; valid pre-Find selection and held-close copy remain preserved.

All 7 unchanged exact serial release gates pass: retained search 12.777/250ms, typing and undo
all budgets, Find-input 35.763/300ms, highlight-frame/job budgets, scroll 22.781/60ms, comparison
2.853/10ms and folds 2.766/8ms. Source/defaults, the fixed 28px row tradeoff, regex limits,
retained-view return-to-live rules, earlier failures, source/log SHA freeze and native limits
are recorded in `.superpowers/sdd/editor-expanded-features-plan/task-13-report.md`. Canonical
T01 remains unchecked for independent scoped review; global/native final checks remain root-owned.

### Expanded T01 fix1 contract — display lifetime and compact controls

Keep the displayed snapshot/selection/viewport through actual query edits, option changes,
debounce/scanning and errors; clearing obsolete matches must not discard the displayed source.
Ordinary output-only refresh must preserve a valid reviewed selection/viewport and a later
current match, either by retaining the reviewed snapshot or by a validated mapping. Never
copy old coordinates onto unrelated live cells. Keep live draining, pending-scan fairness,
final-exit catch-up and cancellation/session/layout/reset/mode guards.

The 28px Find row must remain usable in a narrow 206px output body and with long error/exit
text: retain usable query space, reachable case/regex/navigation/Close, bounded status with
full detail available, and Find focus. Reuse compact existing widgets; Terminal remains at
the bottom (the existing Move Right route refuses it), with no new docking capability.
Defaults, regex semantics/caps, no PTY input and the retained-close/Live behavior stay as above.
Acceptance: actual query plus early Close and option/error variants; append without eviction
from a later match/scrolled selection through completed publication and Close/copy; real
narrow bounds/button/focus/status test. Independent review is still required.

T01 fix1 display policy before edits: query/options changes clear obsolete match catalogs,
not the displayed snapshot, viewport or current local selection; pending/error Close retains
that view. New query/options match the currently displayed snapshot. User navigation, scroll
or selection pins that independent snapshot, including its later current match, while live
output scans may finish without replacing reviewed cells. No coordinates map across content
revisions. The existing bounded count/status area shows Paused and becomes the explicit Update
control (click or field-local Cmd-Alt-U): it clears review selection and starts a fresh live
seed with the same query/options. Query/option changes keep a reviewed pin; navigation clears
selection but stays pinned. Clearing selection cannot resurrect it through pending work.
Unpinned views keep useful output refresh and stopped-PTY final catch-up; epoch/cancel guards
remain. The root-owned compact206px row/test/tooltips are preserved. Acceptance uses actual
query text/option edits and debounce/held/error Close, completed ordinary append from selected
and navigation-only later matches, explicit resume/final exit and clear/supersession races.

### Expanded T01 fix1 worker evidence — fresh review pending

Actual first-query/debounce Close reproduced selection loss; completed ordinary append from
match3 reproduced a reset to match0. Both are fixed with a retained display lifetime and explicit
Paused snapshot/update policy. Query/options clear only obsolete match catalogs; errors keep
the displayed view. Navigation/scroll/selection pin, so completed output publication preserves
later matches and the selected viewport without mapping across content revisions. Paused click
or Cmd-Alt-U explicitly resumes current live source with cleared selection and Find focus.
Known-outdated pins retain dirty output without repeated scans; unpinned and final-exit refresh
remain. Causal real-input, held-close, append, clear/supersession, Paused width/focus/resume and
exact-hex no-PTY-input regressions pass, alongside the root compact206px repair.

Final combined source:37 terminal passes/1ignore,1272 UI passes/25ignores,12 focused search UI,
7 focused model passes/1release-ignore; strict affected all-target Clippy/fmt/diff pass. All7
unchanged exact serial release gates pass:search15.466/250ms, typing/undo budgets, Find-input
38.141/300ms, highlight budgets, scroll23.278/60ms, comparison2.399/10ms, folds2.829/8ms.
Full/scoped patches13+13-fix1, immutable reviewed-v1 evidence, both-baseline unchanged proofs,
source/build/Python/log/patch/report SHA freeze/readback and native limits are documented in
task-13-report.md's fix1 appendix. Actual audit T01 one entry and plan three boxes stay unchecked
until fresh independent scoped re-review; final global/native gates remain root-owned.

Expanded T01 accepted after one review fix round. Fresh scoped review approved spec and quality,
with all three original findings addressed and no new findings. Root rechecked all10 frozen
changed files and327 source/build/orchestration inputs before metadata-only acceptance; source
fingerprints remain unchanged. Current proof:37 terminal and1,272 UI passes (1/25 ignores),
12 focused UI and7 focused model passes, strict affected checks and seven unchanged exact
serial release gates. Canonical T01 is checked;13/64 selected entries accepted. Manual/native
walkthroughs and final workspace/release/global performance gates remain pending.

### Expanded T02 contract — displayed output links

Default terminal output remains selectable, with Shift as the local-selection override when
an application requests mouse reporting. Explicit Cmd-click activates HTTP(S) through GPUI's
external URL API or an existing file through the workspace's existing navigation. Locations
are one-based source characters and clamp to the target text. Missing files report a status
and never create an untitled replacement. Bare prose and unsupported URI schemes do not open.

Relative paths use the clicked displayed screen's per-cell cwd, including paused Find,
retained-close output and command review. Valid local OSC7 changes affect only later output;
malformed/remote updates leave the last valid cwd intact. Quoted Unicode/spaced paths and
soft wraps retain their original context, including offscreen prefixes. Mixed/unknown/expired
or truncated provenance refuses relative paths. No path or shell escape is evaluated.

Acceptance proof will use the existing protocol/model regressions plus real GPUI mouse events,
workspace document/caret outcomes, HTTP(S) platform capture and exact PTY input sentinel bytes.
It covers before/after cd, snapshots while live cwd/cells change, missing files, one-based
Unicode locations/clamping, malformed OSC7/file URLs and unsupported schemes. Preserve T01's
pin/update/selection lifetime and fixed 28px toolbar geometry. Initial inspection found these
production branches already implemented; add missing tests/docs rather than rewriting them.

T02 location syntax also includes compiler diagnostic trailing separators (`src/file.rs:12:3:`)
and quotes enclosing the whole location (`"my file.rs:2:3"`), as well as quoting only the path.
Add causal regressions before the narrow parser extension if these ordinary forms fail.

T02 explicit locations also identify extensionless code files (`Makefile:12:3`, with an optional
trailing separator). Unqualified bare words still do not link. Existing explicit `./Makefile`
and local file-URL forms remain valid; no filesystem scan or token engine is introduced.

### Expanded T02 worker evidence — independent review pending

Three causal parser REDs now pass: compiler trailing separators, fully quoted locations and
extensionless explicit locations. Existing OSC7/per-cell provenance, displayed-screen activation,
selection and workspace navigation were retained. New real GPUI/PTY mouse proof opens old cwd
Unicode/spaced wrapped paths from paused Find, retained Close and command review after live
cwd/cells change; every phase resets the caret before activation and reuses the document.
Live EOF clamping, actual extensionless activation, HTTP(S) external API, missing/refused tokens,
plain/Shift selection, fixed bounds and exact SENTINEL-only PTY input are covered.

Final affected suites pass42terminal/1274UI (1/25existing ignores);13focused link protocol/model
and24focused terminal UI tests pass, including accepted T01 regressions. Strict affected
all-target Clippy/fmt/diff pass. Runtime changes are confined to explicit Cmd-click parsing;
hot owners and benchmark workloads/budgets are unchanged, so no irrelevant performance repeat
was added. task-14-report.md records commands, fixture corrections, self-review, native limits
and source/log/patch/report freeze/readback with outside-baseline proof. T02 and all three
plan boxes remain unchecked for independent review; native/global final gates remain root-owned.

Expanded T02 accepted: fresh scoped review approved spec and quality, no findings. Root verified
nine frozen changed files, all393 repository files and325 source/build/orchestration inputs
before metadata-only acceptance; input fingerprints remain unchanged. Final42 terminal and
1,274 UI passes (1/25 ignores),13 link model and24 terminal UI focused passes, strict affected
checks. No hot-owner/benchmark change or irrelevant timing repeat. Canonical14/64 selected
entries accepted; native OS URL-handler and final workspace/release/global gates still pending.


## Expanded phase — T03 behavior contract (2026-10-30)

Default mouse mode is off, with ordinary local selection/scrollback. Existing X10/button/
drag/any-motion and default/UTF-8/SGR encoders remain the owners of requested event classes,
one-based cell coordinates, modifier bits and overflow refusal. Live UI focuses on press,
floors coordinates from painted terminal bounds, clamps reporting drags/releases, suppresses
same-cell motion and uses Shift from gesture start for local selection. Search, retained
output and command review stay local; Cmd-click links send no PTY input. Blur releases a live
reported drag once, then clears its state; release outside is clamped.

The missing lifecycle branch will clear mouse reporting mode/encoding on both normal exit
and abnormal output-channel disconnection. Use only mouse-mode resets, preserving final
primary/alternate output, scrollback/selection, cwd/provenance, screen identity/layout epochs,
commands/status, cursor/keypad/paste modes and accepted T01 fixed28px geometry. Never use RIS
for exit cleanup. Error events alone may be recoverable and must not reset a running shell;
disconnection uses existing fallback exit1 while retaining any prior error/status. UI drain
clears reporting gesture state on stop without writing a stale release to the dead PTY,
while an active local Shift-drag continues across exit if displayed cells/layout are unchanged.

Acceptance adds causal exit/disconnect regressions (including pending parser sequences),
protocol mode/encoding/alternate/reset checks and real GPUI/PTY exact-byte drag/release/wheel/
modifier/focus/Shift-selection verification. Re-run accepted T01 retained/pin/update/geometry
and T02 no-input-link tests via the full terminal UI suite, full affected terminal/UI suites,
strict affected all-target Clippy, fmt and diff. Mouse encoder/input/render algorithms stay
unchanged; lifecycle-only production edits require no unrelated hot-path gate repetition.
Record actual commands, causal failures, final proof and native limits in task-15-report.md;
leave the one canonical T03 entry and three plan boxes unchecked for independent review.


### Expanded T03 worker evidence — independent review pending

Reused the existing mouse protocol encoder and real terminal listeners. Added only stopped
lifecycle cleanup: normal exit clears supported mouse mode/encoding without RIS; abnormal
PTY disconnection notifies the emulator using existing fallback exit1. A fresh decoder for
the mouse-only reset cannot be swallowed by incomplete child escape input and preserves
final output/scrollback, cwd/provenance, content/layout identity and ordinary cursor/paste
modes. The UI clears reporting_button/last_mouse on stop, retaining an active local Shift
gesture across exit. No new dependency, setting, session schema or terminal/input engine.

Causal REDs establish normal-exit mode leakage, disconnected-channel mode leakage, stale UI
reporting state, and the first cleanup draft incorrectly cancelling local Shift selection.
Final UI coverage proves exact press/drag/outside-release/wheel/modifier/blur bytes received
by a real mouse-aware PTY, Shift selection, RIS restoring plain selection/scroll, exit7
selection/geometry and post-exit RE→READY drag/copy. Accepted T01 pin/update/retained/fixed28px
geometry and T02 displayed-cwd/no-input link fixtures run in the same terminal UI suite.
Actual complete commands/results, intermediate harness corrections, self-review, unchanged
hot-owner evidence and freeze/readback are in
`.superpowers/sdd/editor-expanded-features-plan/task-15-report.md`. T03's single canonical
entry and three plan boxes remain unchecked for independent scoped review.

Expanded Task15/T03 accepted: existing requested protocols, focus/cell handling and Shift selection retained; normal/abnormal stop now narrowly resets mouse flags and stale reporting state while preserving final output/provenance/identity and active local drag. Four causal regressions, 5 protocol +2 session +28 terminal UI tests, full47 terminal/1278 UI (1/25 established ignored), strict affected Clippy/fmt/diff pass. Fresh independent Spec/Quality APPROVE with no findings; 10 frozen files and326 source/build/script inputs corroborated before metadata-only acceptance. Final native/global gates pending.

### Expanded phase — F02 behavior contract

File > Print and File: Print capture the entire source, selected ranges, title and syntax choice before the form. Defaults remain 11 pt, tab width 4, whole document, no numbers, wrapping on and syntax colors off. Page setup uses native paper, orientation and margins; cancellation does not print. With wrapping off, long lines fit the printable width rather than losing text. Color uses the captured syntax and a readable print palette, falling back to black when classification is unavailable. Source preparation runs off the UI thread with no live editor entity; native modal operations run on the foreground executor outside GPUI updates. Invalid options, empty selection, stale shared views, active IME and cancellation leave text, selections, path, dirty state and undo unchanged. Tests cover document and multiple selection excerpts, tabs, Unicode, blank lines, wrapping and colors, then native page setup, multi-page PDF scope/order/clipping and cancellation without a physical print job. Folds do not suppress source.

### Expanded F02 worker evidence — independent review pending

The existing print snapshot and TextKit 1 path now carry wrap and syntax-color choices and an
optional native page-setup step. Source preparation maps captured syntax through excerpt clipping,
tabs, normalized line endings and source-number prefixes into UTF-16 ranges; oversized or
unavailable classification falls back to plain output. Native fitting uses the measured longest
line when wrapping is off. A modal guard spans page setup and printing outside GPUI updates.
Source/state regressions and disposable native PDFs exercise these paths; the task report records
commands, limits and concrete native evidence. The canonical F02 entry and plan boxes remain
unchecked pending independent review.

Expanded Task16/F02 accepted after fresh independent Spec/Quality APPROVE with no required findings. Added wrapped/unwrapped and syntax-color print options plus native page setup, preserving detached immutable TextKit1 path. Actual native preview/systemPDF/cancel+PDFKit/PNG proof and1283UI pass/25existingignored, strictClippy/fmt/diff0; root489hashes/9changed/385unchanged plus328source inputs checked before metadata-only acceptance and all328stable afterward. Exact bounds/defaults/artifacts/provenance in task16report; no physical job, no shared hot path change. Final workspace/release/global gates pending.

## Expanded Task 17 / F03 behavior contract (before verification edits)

Existing Follow is retained: transient/default-off, explicit File/palette Follow Log and
Stop Following Log plus the clickable FOLLOW status. A clean saved regular file admits one
shared background ingestion owner; dirty text, pending saves/writes and composition in any
shared view refuse entry without changing text, selections, undo or recovery. Resolve changes
through ordinary Save/discard first. No Follow save/autosave, shell command or text-edit path
is introduced. Copy/search/selection remain available under the shared editing lock.

Stable bounded snapshots use the existing strict 26-codec/100 MiB policy and disk codec with
BOM precedence. Partial character/BOM bytes stay on disk until a later complete snapshot;
malformed/error snapshots pause safely. Missing paths retry; truncation and rotation replace
the shared clean snapshot. Stop/path change/last-view close cancel retained tasks; stale
versions, disk facts, rotations and path/inode write leases must prevent publication. Accepted
N22 read-ticket/save-copy and F01/F02 interfaces remain unchanged.

Task17 will verify existing behavior and add only missing acceptance proof: real append-only
file descriptors delivering split supplementary UTF-8/16/32 and legacy/stateful tails through
actual Follow actions; rendered shared views with independent bottom intentions, upward-scroll
stability and return-to-bottom resumption; dirty-entry rejection preserving ordinary undo,
recovery and exact source bytes until ordinary Save resolves it. Reuse existing real temporary
file/GPUI coverage for rotation/missing/truncation, stop/restart and stale errors/leases, native
input/IME/overtype/virtual-space, formatting/pipelines and shared closure. No runtime change is
expected unless these meaningful tests expose a missing branch. Canonical/plan stay unchecked
until independent scoped review.

Task17 causal RED found one existing gap before runtime edits: after upward scrolling and
new output, scrolling back to the rendered bottom leaves the caret at the old EOF, preventing
the next append from resuming tail. The existing user-scroll seam will restore EOF only for
a followed document with one empty nonvirtual caret at the bottom. Nonempty/virtual/multiple
selections keep inspection state. No document/source/undo mutation is involved; ordinary scroll
behavior remains unchanged. The new rendered two-window regression reproduces this failure.

Task17 review-intent refinement before the next runtime edit: returning to bottom may restore
only the remembered auto-tail caret. Capture N16's explicit selection generation when that
EOF caret scrolls away; source ingestion maps selection state without advancing that generation,
while explicit review invalidates it, including moving away and back before a frame. Arbitrary
carets and nonempty/multiple/virtual selections must remain untouched. The first broad draft
was rejected by new causal unrelated-caret and no-frame away/back regressions. Viewport and
caret tail currently remain coupled: selected/unrelated caret inspection stays paused until
an explicit Follow Log or navigation to EOF; ordinary scrolling does not relocate carets.

Task17 implementation, focused/full affected verification and exact serial hot gates are
complete pending independent scoped review. The engine and source/save protections were
retained; only view-local paused-caret generation and the existing user-scroll seam changed.
Four new actual-file GPUI proofs cover split append-only codecs, rendered independent shared
views, arbitrary/selected/multiple/virtual and no-frame review intent, and dirty refusal with
ordinary undo/recovery/Save. Full affected suites pass 728 editor/88 project/1287 UI (25 existing
ignored); backend 6/codec 11/I/O 2/model 6/Follow UI 21 focused tests and strict affected all-target
Clippy/fmt pass. The task report retains all causal REDs, fixture/strict corrections, exact
commands/results and native limits. Canonical F03 and the Task17 plan remain unchecked.

Task17 exact serial release gates pass with unchanged workloads/budgets: comparison scroll
2.987 ms/10 ms, folded scroll 2.759 ms/8 ms, ordinary restart scroll 22.817 ms/60 ms (10 lines
reparsed), and UI keystroke with the find panel on 10M matches 39.648 ms/300 ms. Each exact
command selected one actual test and ran with one test thread. Snapshot17 diff and the final
source/build/script/log/report SHA readback are recorded in `task-17-report.md`; source and
benchmark inputs match their final-check anchor. Native physical wheel/menu/palette and final
whole-workspace/release/global checks remain root-owned. Canonical/plan stay unchecked.

Expanded Task17/F03 accepted: fresh independent Spec/Quality APPROVE,no findings. Shared strict read-only snapshot engine retained; narrow generation-based remembered auto-tail return fixes resume while preserving manual/selected/multiple/virtual/no-frame review. Full728editor/88project/1287UI pass+25existingignored; focused6backend/11codec/2IO/6model/21UI,strict/fmt/diff0. Four exactserialrelease gates pass unchanged:comparison2.987/10ms,fold2.759/8,ordinary22.817/60,UIFindkeystroke39.648/300. Root827hashes/7changed/387outsidebaseline and377source/script inputs rechecked before metadata-only acceptance,all377stableafter. ActualOSfile+GPUIrender proof; freshphysicalnative/global/release checks pending.


## Expanded Task 18 / F04 behavior contract (before product edits)

Retain default-off `file_autosave` and the existing shared-document scheduler/protected
ordinary save engine. Add `file_autosave_delay_ms`, default 2000, to Settings, the known file
key catalog and commented default template. It accepts whole numeric milliseconds in
1..=4294967295, including 2000.0 under the existing whole-number convention. Zero, negative,
fractional, overflowing and non-number values are invalid. Global parsing warns and keeps
2000; invalid project/scoped layers warn and preserve inherited preferences under existing
validation. User/project/language/scope/path layering remains the existing Settings overlay.
This unit/range is the implementation choice; no dependency or new save engine is needed.

Every eligible edit restarts one idle timer with the current resolved delay. An unchanged
pending key does not restart on refresh. Owner/policy/delay changes cancel the task, advance
its existing generation and restart a full new interval; disabled policy cancels without
writing. Oldest visible ViewModel ownership, default-off typing fast path and no per-frame
work remain. Busy IME/snippet/save/path-write/chooser/Close guards defer as before; manual
save and recovery stay independent. Stale preparation is rejected by existing validation;
known disk baseline, failed prepared-key suppression, leases/turns/read tickets, lossless codec,
history and dirty/error behavior stay unchanged. No L04 timeout/failure-policy expansion.

Acceptance uses virtual executor clocks and actual temporary-file UI protected saves: default
no write at 1999ms/write at 2000ms; rapid edits coalesce into one real held write only after
the last full interval; positive override changes the exact boundary; pending delay/project/
owner changes cannot leak an old timer; IME and in-flight saves defer; conversion/permission/
external-conflict errors remain visible and dirty with original external bytes untouched;
untitled/read-only/Follow/recovered-unknown/deleted/scratch never prompt or write. Retain all
existing alias/shared/close/save-copy/history/recovery tests and their interleavings, expressing
the new two-second default without weakening them. Run full config/editor/project/workspace/UI
suites, focused settings/timer/save/race checks, strict affected all-target Clippy, fmt/diff,
and unchanged exact serial typing/UI-keystroke hot gates. Root retains independent review and
final global/native/release gates. Snapshot18 already exists and will not be recaptured.


### Expanded Task18 / F04 worker evidence — independent review pending

Added `file_autosave_delay_ms`, whole numeric 1..=4294967295ms/default 2000, to existing
Settings/catalog/template and scheduler Duration. Normal invalid global warning/default and
project/scoped inherited fallback are covered. All save/format protections, stable oldest
ViewModel policy, shared timer, default-off input fast path and N22/F01/F03 owners are retained.
Six causal REDs establish missing default/config/project/delay/coalescing/owner behavior;
final config 2/project 1, UI autosave 28 and protected save/copy 73 pass. Full affected70 config,
728 editor,89 project,111 workspace,1,290 UI pass with 25 established UI ignores. Strict affected
all-target Clippy/fmt/diff pass; exact serial unchanged release gates: typing/undo 0.003/0.098/
1.567/15.810ms against 1/3/20/30, UI-keystroke 37.475/300, disk-poll median 0.1944709/1,
100 MB save capture/finish printed 0.000/0.000ms against 1/1. No benchmark/assertion was weakened.

The first UI autosave run exposed six old one-second formatter helper assumptions; only its
clock step became two seconds. A test-table i32 literal inference compile error was repaired
before final source/input capture and all final suites. Logs retain every individual command,
exit and timestamp. Task18 report records causal evidence, actual source/fixture/OS-file
provenance, self-review and limits; physical native smoke/real-server and final global/release
checks remain root-owned. Snapshot18 was supplied and never recaptured. Canonical/plan boxes
stay unchecked for independent review, with final patch/changed-file/source-input/artifact
hash freeze and readback under `.superpowers/sdd/editor-expanded-features-plan/task-18-*`.

Expanded Task18/F04 accepted after fresh independent Spec/Quality APPROVE,no findings. file_autosave stays offdefault; file_autosave_delay_ms positivewhole1..4294967295 defaults2000 using existing protected shared timer/policy. Full70config/728editor/89project/111workspace/1290UI+25existingignored,focused2config/1project/28autosave/73save,strict/fmt/diff0;4exactserialreleasegatespass unchanged. Root835hashes/6binaries/11changed/383outsidebaseline+378source/scriptinputs rechecked beforemetadata-onlyacceptance,all378stableafter. Native/global/workspace/release finalstage pending.


## Expanded Task 19 / F07 behavior contract (before product edits)

Extend the existing default-on explicit-discard rescue Store and catalog. Capture only
nonempty dirty ordinary untitled text when its last visible ownership is explicitly discarded;
Save, ordinary removal, hot exit, previews, scratch/results and surviving shared views stay
excluded. Native IME/close and synchronous capture-before-last-window-quit policies remain.
Keep existing record IDs and metadata, per-directory generation/publication locking, private
0700 directory/0600 owned regular records, no-follow/hardlink/traversal protections.

Retain the latest 50 entries for strictly less than 30 days, with whole-text 1 MiB per draft
and the existing 8 MiB aggregate cap (so fewer than 50 large drafts may fit). Empty payloads
are never captured. No truncation or silent capture-success on failure. Existing loss prompts
remain. Invalid UTF-8 payloads fail locally while valid siblings remain available; unsafe
filesystem metadata continues to fail explicitly.

The existing menu/palette Rescue picker loads metadata and at most a 1 KiB prefix per entry
once on a worker; filtering uses the bounded derived snippets/time/size without payload I/O.
Selecting opens a read-only scratch preview via the existing dock/editor presentation, with
Restore as New Untitled, permanent selected-entry Delete after confirmation, Browse and Clear.
Full preview and restore reads occur on workers. Loading surface identity, Store generation
and selected record membership are rechecked before results publish; Cancel/replacement,
Clear or Delete cannot publish stale text. Restore always rereads the record and creates
an independent dirty writable untitled buffer/undo history. Deletion validates the exact
record ID under the Store lock, never arbitrary caller paths; stale actions fail usefully.

Acceptance: actual Unicode close/discard into disposable support; fresh Store discovery;
content search, read-only preview, independent restore/undo; Delete/Cancel/Clear and stale
load/actions; real 51-entry count and exact controlled 30-day boundary/aggregate pruning;
corrupt valid siblings, empty dirty draft exclusion, private modes, unsafe paths/hardlinks/ACLs,
shared ownership/IME/quit retained. Run full workspace/UI/editor/project suites, focused close/
storage/races, strict affected all-target Clippy, fmt/diff. No shared input/render/search/index
algorithm changes are planned, so general release frame gates remain root-owned. Snapshot19
already exists. Canonical and plan checkboxes remain unchecked for independent root review.


### Expanded Task19 / F07 worker evidence — independent review pending

Existing private rescue now keeps latest50 records younger than30days, bounded by whole1MiB
per draft/8MiB aggregate. Worker-loaded1KiB excerpts/time/size feed existing Choices search;
selected full read-only scratch dock preview offers independent dirty-untitled Restore,
confirmed exact-entry permanent Delete, Browse and Clear. Generation/publication serialization
and private guards stay; empty dirty drafts are excluded. Displayed cross-window stale previews
cannot Restore/Delete removed text, and never enter hot-exit/session recovery; independently
restored working documents retain normal recovery. No persisted index/schema/database/dependency.

Causal REDs prove old20-count retention, empty payload capture and absent Unicode content search.
Final focused26UI/12Store, full728editor/89project/1298UI+25existingignored/116workspace and
doctests pass. Strict affected all-target Clippy/fmt pass. One unchanged exact serial release
UI-keystroke gate passes37.590ms/300ms. Real disposable close/freshStore/preview/restore/undo/
Delete/Clear,51-entry/exact30-day/8MiB pruning, escaped50-row bounds, corrupt siblings, private
files/ACLs, shared/nativeIME and pending/cross-window races are covered. First full run's
unchanged parser warmup test failed once; exact isolated and final parallel full runs pass,
with no parser/opening/assertion/time changes and no established cause. The report retains this
observation, all individual command exits/timestamps, self-review and native/global limits.

Snapshot19 was supplied and never recaptured; final diff19 and source/build/scripts/assets/
executables/logs/artifact SHA freeze/readback are in `.superpowers/sdd/editor-expanded-features-plan/task-19-report.md`.
Canonical F07 and Task19 boxes remain unchecked for root independent scoped review. Physical
native smoke, whole-workspace/global/release assessment remain root-owned.

### Expanded Task19/F07 — fix round1 pre-edit contract

Independent review requires five corrections before acceptance. Oversized unusable records must not consume retention capacity and evict valid siblings: exercise eight valid1MiB records with newest extended to8MiB, preserving every valid sibling physically and through restore across list/summary/delete. Existing owned/private regular-file checks remain. All filesystem metadata/pruning/permissions and waits for those critical sections run on workers; final foreground admission uses nonblocking in-memory freshness and keeps Delete/Clear/capture/expiry safeguards. Delayed Clear completion dismisses only the matching invalidated rescue surface, preserving replacement pickers and new-generation previews. Failed list/read closes or replaces its loading surface with visible error and a live surviving preview/editor/workspace focus, including status-bar-hidden cases.

The toolbar must keep actions and preview usable at supported220px right,440px default and narrow bottom dock sizes. Root owns this isolated small layout fix and causal actual-bounds/hit/focus tests first; original owner then resumes substantive Store/async/focus corrections serially. Preserve existing commands/shortcuts, private bounds, default-on explicit discard, no session/index schema. Capturegen remains independent of any read-publication freshness mechanism. Record exact policy for invalid oversized artifacts without losing valid history. Every required finding needs causal regression evidence; full affected/strict/fmt/diff and relevant exact gates follow final source. Fresh scoped re-review checks all five findings. No parser/assertion/timeout changes for the earlier unresolved warmup observation.

Fix1 substantive policy refinement before production repair: safe owned/private oversized
artifacts remain discoverable/deletable with a local payload error, but are excluded from
valid count/byte admission. Retain at most50 total rows (newest valid records have priority);
invalid overflow/expired artifacts may be unlinked without ever evicting usable siblings.
Valid payloads remain <=1MiB/8MiB aggregate/<30days. A separate atomic read-publication revision
belongs to the same existing Store guard: selected/catalog workers return a stamped result;
UI admission compares only atomic revision/generation and time, without filesystem work or
waiting for the storage mutex. Delete/pruning/Clear invalidate before removal; Clear alone
advances capture generation. No persistent membership/index/schema is introduced.

Clear captures the initiating rescue picker input and preview viewer identity; its delayed
completion only dismisses those matching pre-Clear surfaces, never replacement palette/Choices
or a post-Clear preview. Terminal list/read/stale admission errors retain a focused Choices
error surface with visible progress and Browse/Return actions, observable with hidden status
bar. Return explicitly focuses the surviving preview/editor/workspace. Preserve root's compact
toolbar and its actual bounds/pointer regression. Causal red cases precede substantive edits.


### Expanded Task19/F07 fix1 candidate evidence — fresh scoped review required

All five required findings have candidate corrections: root compact220px toolbar with actual
bounds/pointer proof; size-invalid artifacts cannot spend usable retention capacity; serialized
worker catalog/load return atomic generation/revision/expiry stamps for nonblocking foreground
admission; delayed Clear preserves replacement/new-generation surfaces and keyboard focus;
terminal hidden-status loading failures retain focused visible Browse/Return error choices.
Safe oversized artifacts can occupy remaining slots within50 total rows but never evict valid
<=1MiB/8MiB history; external damaged bytes may exceed usable capacity until Delete/expiry.
Only Clear advances capturegen; capture/Delete/Clear/pruning invalidate read stamps.

Causal REDs are retained per finding, including a second Clear focus-loss refinement. Final
35UI/15Store focused, full728editor/89project/1307UI+25existingignored/119workspace+doctests,
strict affected all-target Clippy/fmt pass. All373 anchored source inputs stable;371 unchanged
from v1 outside the two cold rescue owners, including keybindings/catalog/generic picker/
input/render/parser/workloads. All829 original archive items verified. No release gate was
rerun or claimed for changed fix1 source; v1 measurement is explicitly historical. Singlev1
parser warmup failure remains cause-unknown; unchanged isolated/v1final/fix1full tests pass.

Full snapshot19 and scoped19-fix1 patches/changed paths/current full+scoped SHA manifests, exact
root/worker command exits/timestamps, archive integrity/readback and detailed self-review are
in task19 report. No baseline recapture, dependency/schema/engine, parser/assertion/timeout or
coverage/budget weakening. Root corroboration and fresh scoped re-review of all5 findings are
required; canonical/plan stay unchecked. Native/global/whole-workspace/release final gates stay
root-owned.

Expanded Task19/F07 accepted after fix round1: fresh scoped re-review marks all5 required findings ADDRESSED, Spec/Quality/Fix Round PASS. Final35UI/15Store;728editor/89project/119workspace/1307UI+25existingignored/doctests,strict/fmt/diff0. Root2092 hashes/4binaries/13full7scoped files/829v1archive and382source/script inputs rechecked before metadata-only acceptance;all382stableafter. No fix1 release rerun claimed. Optional P3 input/query display issue is ledgered for final triage; initial N16 warmup observation remains cause-unknown. Final native/global/workspace/release stage pending.


### Task20 / E04 pre-edit behavior contract (2026-10-31)

The existing `ViewModel::overtype` defaults to false, is transient per view, and is reset by
new/duplicate/restored/reopened views. `editor::ToggleOvertype` is reachable through Edit,
the palette/custom action catalog and the clickable INS/OVR status indicator, which reads
the active source view independently of temporary messages. Switching tabs/windows must
restore each view's displayed mode; Find/Replace/palette/form fields keep their own insert
state. No session persistence or new default key binding is needed.

`type_text`/`type_literal` overwrite one following whole grapheme per new input grapheme,
counted in each caret's left context; selections replace exactly, targets merge through the
ordinary atomic edit engine, line endings are never consumed and EOL extends normally.
Paste/programmatic `insert`, snippets, Tab/newline and input chunks with tabs/line breaks
retain their established insertion behavior. Explicit native replacement ranges replace
exactly; macro InsertText preserves the corresponding typed/explicit-range distinction.
Normal Undo/Redo changes source and selections, leaving mode intact.

Native marked updates retain original source beside the preview, tracking actual char and
UTF16 ranges plus virtual padding/full selections. Cancel restores originals; commit/unmark
performs the replacement once. Changed document version/selection/primary invalidates old
targets and leaves preview as ordinary inserted text. Recovery excludes only valid tracked
preview/padding; actual close unmarks; Save Copy retains its existing explicit native-rope
capture policy and Print refuses any shared marked source. Editable read-only working copies
retain ordinary insertion/overtype and undo but cannot overwrite the protected source file;
immutable input-disabled viewers and Follow refuse native mutation.

Task20 will extend proof only if the current behavior meets this contract: existing model
and native tests already cover ASCII/emoji/combining and multiple input graphemes, long/interior
clusters, LF/CRLF/CR, EOL, selection/overlap/multicaret, shared views, history and IME update/
cancel/commit/staleness. Add actual rendered tab/window switching and Find/Replace field
isolation, plus explicit read-only working-copy native commit/history proof. Reuse full
editor/UI suites and focused overtype/IME/snippet/virtual/recovery/Save Copy/Print/Follow tests;
strict affected all-target Clippy, fmt and diff checks remain mandatory. No hot-path change
is expected, so unrelated release performance gates need no repeat. E04 and all three plan
boxes remain unchecked for independent root review.


Task20/E04 proof complete for independent review: no runtime branch was missing. Added three
GPUI tests for two-window/shared-document and tab-local modes with rendered status labels,
real focused Find/Replace fields while the source remains OVR, and protected read-only
working-copy native typing/IME/history. Strengthened immutable viewer refusal with mode
already enabled. Resizing before label-bound checks improves stale-label detection when the current control renders; independent review noted that asserting the control itself moved would further harden the test. Source mutation/IME owners and all prior
regressions remain unchanged.

Actual full affected checks passed: 728 editor, 89 project, 119 workspace and 1,310 UI
(25 existing ignored), including a final full UI run after the label-proof refinement; doc-tests pass.
Focused checks passed: 20 UI / 13 model overtype, 49 undo, 14 virtual-space, 20 snippet,
13 recovery, 22 Save Copy, 6 Print and 1 exact Follow. Strict affected all-target Clippy,
fmt and diff checks exited 0. Initial test compile
borrow correction and stale-selector assertion correction are retained with exit 101 in the
exact ledger; neither is a product RED or hidden failure. All final checks pass. No runtime
or benchmark code changed, so no redundant release performance rerun is claimed. Final
whole-workspace/native/release gates remain root-owned. Task20 report and frozen Snapshot20
patch/changed paths/source-build-script-asset/log/executable SHA readback provide handoff;
canonical E04 and the three plan boxes stay unchecked.

Expanded Task20/E04 accepted after fresh independent Spec/Quality approval, no required findings. Existing runtime retained; three GPUI tests and stronger immutable-viewer proof added. Root rechecked 833 frozen hashes and 377 source/script inputs before metadata-only acceptance, all 377 stable afterward. Full 728 editor/89 project/119 workspace/1310 UI pass (25 established ignores), focused/strict/fmt/diff pass. Test-only delta needs no redundant hot-path gate. Optional indicator geometry freshness hardening and inherited dependency future-incompatibility notices remain in final triage. Final whole-workspace/native/release verification is pending.


## Expanded E05 behavior contract (2026-10-31, before product edits)

Keep the existing Selection/Rows/Measure, text-edit and native composition engines. Replace the
Boolean `virtual_space` preference with `"off"` (default), `"rectangular"` and `"all"` policies;
legacy false means off and legacy true means all, retaining ordinary click/Right/vertical motion.
The existing global, scoped and project preference pipeline remains the entry point. Fields and
immutable viewers stay off; source read-only and Follow guards retain their existing behavior.

Rectangular provenance is transient per selection: Option-drag (including a zero-width box) and
keyboard Add Caret Above/Below create it. Ordinary click/source selection creates ordinary carets.
Motion and edits retain rectangular caret provenance; duplicates retain it independently; text,
selection and composition histories compare it with the other selection fields. Normalization
requires the policy to admit that provenance before retaining a virtual endpoint. Narrowing to
rectangular clamps ordinary gaps; off clamps every gap, including restored history. A settings
change ends typing and clears native composition/completion metadata by the accepted existing
policy: inserted preview/padding remains ordinary source. Other-view edits retain unaffected
provenance/gaps and clear stale endpoints. No persisted selection/session schema is added.

Virtual padding is still deferred until an edit and shared once per EOL. Respect the document's
current indentation policy: spaces when translate-tabs is enabled; otherwise tabs to existing tab
stops plus residual spaces. Preserve every original explicit space/tab. Every caret/selection
endpoint inside a shared pad remains representable (a tab never skips one), with character offsets
mapped separately from display-column distances. Padding and inserted text share the existing
single undo step. Explicit Pad to Column and Column Editor padding retain their documented spaces.

Acceptance adds causal regressions for the three policies, legacy Booleans, malformed/global/
scoped/project settings, plain versus rectangular clicks and keyboard carets, policy cleanup,
mixed-provenance selections, tab-aware shared materialization, empty distributed targets, undo,
IME update/cancel/commit and policy changes, tabs/CJK/combining/supplementary text, wraps/folds,
shared views, read-only/Follow, recovery/session physical offsets, N01/N02 codec refusal and N04
motion. Existing E04 native/source/field guards, explicit Save/Save Copy and Print contracts stay
covered. Run full editor/config/UI suites, focused actual EntityInputHandler checks, strict affected
all-target Clippy, workspace fmt and whitespace checks, and unchanged exact serial typing/find/
rendering release gates. Root owns independent review and final global/native/release verification.


### Expanded E05 mapped-tab/provenance refinement (before repair)

The new origin-only native regression exposed two selection reconstructions that discard
rectangular provenance: backend compose ranges and the IME's final caret/selection placement.
Keep their original positional semantics and carry the existing target's origin alongside them,
so a provenance-only explicit change advances selection generation and invalidates composition.

A folded display line can have a different tab-stop remainder from its physical source line;
soft-wrap continuation rows can too. Add actual rendered-target preview/cancel/commit/undo proof.
At the existing UI text-change boundary, admit tab padding only when physical and mapped virtual
endpoint tab stops agree; otherwise use spaces to preserve the requested display target. Native
replay or compound edits with no current virtual endpoints use spaces conservatively in mapped
views, because restoring the original selections can change the rows. Keep that permission local
to one synchronous edit, recompute it from current view/text context, and reset it afterward.
The default-off/no-virtual fast paths remain; no fold/wrap geometry is stored in sessions or
composition state. Ordinary unmapped hard-tab padding keeps honoring document indentation.


E05 pre-freeze self-review refinement: a selection formed by merging ordinary and rectangular
origins is ordinary. It must not promote an ordinary virtual endpoint when narrowing to
rectangular-only. Add causal mixed-overlap proof and preserve the existing selected source;
normalization clamps only disallowed virtual columns. Two rectangular contributors retain
rectangular origin. No text is changed by cleanup.


## Expanded E05 candidate result (2026-10-31; independent scoped review pending)

Implemented `off`/`rectangular`/`all` virtual-space policies in existing settings and editing
owners. Legacy false remains off and true remains all-carets. Per-selection rectangular origin
comes from Option-drag/Option-click and Add Caret Above/Below, survives ordinary motion/edit/
undo/IME/duplicate paths, and participates in full selection freshness. Mixed-origin merged
selections are ordinary; narrowing clamps their gaps and preserves the correct primary group.
Settings changes clear stale drag/completion/composition targets under the accepted preview policy.

The original column materialization path now emits document-policy tabs plus residual spaces,
sharing one EOL pad and mapping every original target to a character boundary (including empty
paste targets). Explicit padding stays spaces. Current mapped/source tab-stop mismatches fall
back to spaces; mapped native replay does too conservatively. Synchronous permission is recomputed
and reset for every edit, with no cached geometry or session schema. Actual drawn-target tests
cover folded joined lines and final soft-wrap rows, preview/cancel/commit/undo, and shared edits
that change tab-stop agreement. Existing codecs, N04 motions, E04 native/field/source guards,
read-only working copies, Follow, recovery, Save Copy and Print retain their contracts.

Final suites pass:71 config,734 editor,1319 UI+25existing ignored,119 workspace and doctests.
Focused24editor+2config/23UI virtual,13actual EntityInputHandler,13editor+20UI overtype,
2recovery,2physical-codec guards,22SaveCopy,6Print,21Follow and8subword tests pass. Strict affected
all-target Clippy, workspace fmt and whitespace checks pass. The final full run's unchanged
parser-warmup test passes; this does not resolve the earlier Task19 cause-unknown observation.

All12 unchanged exact serial release gates select1test and pass. Typing1/1000carets measures
0.001/1.167ms (budgets1/20ms); undo0.047/15.061ms (3/30ms); real10M-find keystroke36.389ms
(300ms); comparison/folded scrolling2.345/3.315ms (10/8ms). All10render gates pass. The first
wrong-prefix typing command selected0 after its optimized compile and supplies no performance
claim; its log remains beside the corrected exact invocation. No gate, budget or ignored flag
was changed, and no other build/test competed with measurements.

Supplied Snapshot21 was never recaptured. Exact commands/UTC/exits, causal REDs, all failed
iterations, selected-test counts, owner/API changes, self-review, changed-file manifest,
source/build/script/fixture/asset/executable hashes and readback are in
`.superpowers/sdd/editor-expanded-features-plan/task-21-report.md` and task-local artifacts.
Canonical E05 and all Task21 boxes remain unchecked for root corroboration and fresh review.
Physical native smoke, whole-workspace/final-release stages and the optional Task20 indicator
geometry suggestion remain root-owned. No helper, dependency, commit, push, PR or remote mutation.


### Expanded E05 fix round1 behavior contract (2026-10-31, before repair)

I1: Explicit native UTF-16 replacements keep the rectangular origin corresponding to each
replacement target. Overtype explicit-range previews retain that origin through commit, while
cancel restores full original selections. Empty physical marked-text cancellation retains the
composed target's origin. Subsequent Right stays virtual only for the rectangular contributor;
ordinary carets retain ordinary motion. Preserve exact Unicode text, one-step commit history,
provisional-source policy and all native input/read-only/Follow guards. Add actual callback REDs
before constructor repairs. M1 adds the expected two drawn-carets count before geometry checks.

I2: Carry per-origin metadata through explicit and server snippets, field/final placement and
server completion without granting rectangular origin to ordinary source-range selection commands.
The snippet session must preserve the appropriate origin across Tab/Shift-Tab, mirrors, primary
ordering and mixed contributors. Add causal continuation tests and retain all existing snippet/
completion stale/additional-edit/history coverage. No new selection/snippet/undo engine or session
persistence. Root handles the isolated native repair first; the original owner handles the coupled
snippet/completion repair afterward. Final source must pass full affected/focused/strict checks,
with exact performance gates as relevant and fresh scoped re-review of both findings.

Root I1/M1 candidate proof: compiled3native reconstruction regressions failed before production
repair (direct UTF-16 mixed origins, explicit overtype commit, empty physical marked update).
Three reconstruction maps now copy the corresponding existing selection's rectangular flag; no
new allocation or ordinary typing branch change. All3 pass, as do26virtual UI/13native handlers/
21overtype/2recovery tests and strict UI all-target Clippy/fmt/diff. The initial missing test Motion
import compile failure remains separate from the causal RED. The two-caret geometry assertion
also passes. I1/M1 still require combined fresh re-review; I2 remains open. Final full affected
source checks and serial release gates will run after the coupled snippet/completion repair.


I2-specific pre-edit contract: each expanded caret contributes its rectangular origin to the
existing Session slots (all primaries, plain/transformed mirrors, nested fields and implicit end).
Session selections must carry the slot's origin in the existing first-primary-last order;
Tab/Shift-Tab must not infer it by zipping the current selection list, whose field count/order
can differ from the contributing expansion list. Filtered trigger expansions retain origins only
for the matching contributors. Ordinary select_ranges remains ordinary. A snippet-only placement
helper reuses current normalization and primary-last behavior. Server snippet and plain completion
capture the original primary's origin before additional edits/map/normalization; secondary origins
remain independent. No persisted metadata, alternative snippet engine or new async behavior.
Causal tests will exercise Rectangular continuation, mixed/heterogeneous fields and mirrors,
Tab/Shift-Tab, filtered triggers, Unicode/additional edits, primary grouping and normal undo/redo.


I2 mixed-merge clarification before correction: server snippets keep their existing primary-only
selection contract and retain that target's original origin. Plain server completion retains peer
selections, so its final caret inherits the already-mapped/normalized primary group's origin.
A replacement or additional edit that coalesces rectangular and ordinary peers must stay ordinary
under the accepted AND ancestry rule; do not restore an earlier unmerged primary flag. Add paired
continuation/undo/redo proof for these distinct contracts, including an unrelated primary group.


E05 fix1 additional I3 macro contract, before repair: explicit customization InsertText range
replacements must retain each existing caret's rectangular origin, just like native replacements.
Keep relative physical character offsets, exact replacement behavior in insert/overtype modes,
ordinary versus rectangular continuation, one-step text undo/redo, and existing input/read-only/
Follow guards. No new binding, setting, persistence or asynchronous work. A real action regression
with Unicode and mixed origins must fail before the minimal reconstruction repair.

The previous optimized compile was stopped with SIGINT only on its owned Cargo PID after this
additional source gap was found. Cargo exit-2/tool254 and absence of remaining runner/compiler/
linker are recorded in task-21-fix1-worker-macro-handoff.json; no performance result is claimed.
The pre-macro373-input anchor is preserved separately. Final combined source requires fresh
verification and all12 exact serial release gates before scoped re-review.

Root I3 macro candidate proof: the compiled action regression failed with Right crossing the
newline rather than entering virtual space. The explicit replacement reconstruction now copies
the existing caret's rectangular flag; the regression passes for Rectangular/All/Off in both
insert and overtype modes, exact Unicode text and one Undo/Redo. Covering30virtual UI/9command/
13native/21Follow tests and strict UI all-target Clippy/fmt/diff pass. The native I1/M1 source
files remain byte-identical to their earlier handoff. Root stage command records are preserved
in native and macro journals with a full original pre-split copy; no failed invocation was lost.
I1/I2/I3/M1 remain candidates pending final combined checks and fresh scoped re-review.


### Expanded E05 fix round1 combined proof (2026-10-31; scoped re-review pending)

I2 now stores each expansion's origin on existing snippet slots, preserving heterogeneous nested
fields, mirrors/transforms, filtered triggers and final stops through Tab/Shift-Tab. Ordinary
source-range selection remains ordinary. Plain completion inherits its mapped primary group's
origin, including mixed-origin merges; server snippets retain their primary-only origin contract.
Eight backend continuation/undo regressions and actual snippet-picker/keyboard plus plain/snippet
language-server resolve paths cover the repair. I1 native reconstruction and I3 explicit macro
replacement keep their corresponding existing origins; M1 asserts two carets before geometry.

Final combined full suites pass:71config/742editor/1326UI+25existing ignored/119workspace and
doctests. Focused snippet72editor+22UI, completion30UI, virtual2config+32editor+30UI, native13,
commands9, overtype13editor+21UI, recovery2, SaveCopy22, Print6, Follow21, codecs2 and subword8
pass. Strict affected all-target Clippy, workspace fmt and diff checks pass. The unchanged parser
warmup test passes; its earlier Task19 cause-unknown observation remains unresolved.

All12 unchanged exact serial release gates select1test and pass on the final combined source.
Typing1/1000carets measures0.003/1.700ms (budgets1/20ms), UndoRedo0.100/15.486ms (3/30ms),
real10M-find42.502ms (300ms), comparison/folded scrolling2.250/3.223ms (10/8ms). No competing
build/test ran during measurement. The earlier interrupted compile remains Cargo-2/tool254 with
no performance claim; every causal RED, test-authoring failure and metadata audit remains recorded.

The detailed report freezes full28/scoped12 deltas,373final source inputs, six final executables,
both supplied baselines and the macro baseline,17native+20macro evidence items and868archived V1
items. Root journals preserve all22records, including the later macro diff check separately from
the original21-row split. Canonical E05 and Task21 boxes remain unchecked. Root corroboration,
fresh scoped I1/I2/I3/M1 review, global/final release and physical native smoke remain root-owned;
the optional Task20 indicator geometry suggestion remains final triage.

Expanded Task21/E05 accepted after fix round1 fresh scoped Spec/Quality PASS: I1/I2/I3/M1
addressed, no new required breakage. Three policies defaultoff/legacycompat, deferred endpoint-safe
indentation padding and mapped-stop spaces fallback preserve all existing native/macro/snippet/
completion/source/history/persistence guards. Final71config/742editor/1326UI+25ignored/119workspace
+doctests, focused preservation/strict/fmt/diff and12correctexactserialreleasegates pass. Root
rechecked2203frozenhashes plus finalization evidence/381source+scripts before metadata-only
acceptance, all381stableafter. Reviewed report is archived; live M1 function label corrected,
original frozen generator retained as historical evidence. Native/global/workspace/release final
stages and the existing F07/E04/unknown-warmup/dependency triage remain pending.
## Expanded E06 behavior contract (2026-10-31, before product edits)

Keep `modelines: false` by default. Extend the existing data-only parser and workspace
preference refresh: first/last five physical LF lines, at most 4,096 bytes per rope line
(including its terminator), no duplicated short-file lines, later valid fields win.
Keep Vim `vi:/vim:/ex: set/setlocal …:` and Emacs `-*- … -*-` fields and margins;
add Vim `ft/filetype/syntax`, Emacs `mode` and short `-*- language -*-`, and Kate
`kate: tab-width N; indent-width N; replace-tabs on/off; word-wrap on/off;
word-wrap-column N; syntax NAME;`. Widths remain 1–16; margins remain 0–1,000.
Language values are trimmed ASCII identifiers/names of at most 64 bytes (letters,
digits, spaces, `_+.#()-`); only installed registry IDs/names, available syntax names,
bundled protocol IDs and documented common aliases resolve. Unknown/malformed values,
commands, expressions, coding/save/plugin/shell fields remain inert, with no declaration
evaluation, execution, filesystem write or network operation.

For each supported field use explicit document/view/window menu choice > enabled modeline >
EditorConfig > existing scoped/project/global settings (detection stays below configured
settings). This supersedes the first-phase EditorConfig-over-modeline routine choice.
EditorConfig save-format fields remain independent. Indentation stays document-owned;
wrap/rulers and syntax stay view-owned. No text/undo/selection/IME reconstruction occurs.

Language is derived view data, never a manual View > Syntax override or session field.
Resolve opt-in from the explicit language, otherwise ordinary path/content detection,
without the declared language; resolve other scoped preferences from the admitted language.
That stable anchor prevents language-scoped opt-in oscillation. Recompute on open, new shared
view, preferences/project/customization/path changes and changed edge directives. Disabled
edits keep the existing no-scan fast path; unchanged parsed directives keep the no-reapply
fast path. Removal/disable returns to ordinary detection; explicit syntax wins. Switching
language drops stale highlight/language decorations and expires asynchronous request identities.
Parsing/loading never starts a server: accepted L02 automatic signature synchronization keeps
its existing-session-only policy, while explicit/triggered assistance retains its startup policy.

Acceptance: causal parser tests for Kate/language forms and hostile values; per-field
precedence including partial EditorConfig/modeline fallback and explicit wrap/indent/ruler
choices; all documented aliases; stable scoped enablement; enabled edits/removal/undo,
disabled edits/re-enable, shared windows/views, session restore, unchanged text/disk/undo/
full selections/IME, stale LSP identity/routing and no server startup. Existing bounded scan,
save-as/read-only/reload regressions stay. Run full config/UI/syntax suites, strict affected
all-target Clippy, fmt/diff, and unchanged exact serial release typing/Find/render/scan gates.
Snapshot22 stays fixed; independent scoped review and global/native/release stages are root-owned.

Expanded E06 inspection refinements before repair: a terminal LF ends its physical record;
Ropey's extra empty display row does not consume one of the last five records. Retain bounded
safe language candidates in source order and resolve newest known candidate, so an unknown
later name cannot mask an earlier valid declaration. Candidate payload is bounded by the
ten scanned 4,096-byte lines; load the trusted catalog once per changed directive application.
When a derived route changes, reuse the existing completion/hover/signature dismissal and
freshness counters immediately, including same-grammar protocol changes and settings-only
disablement. Pending/visible UI work must not wait for a later frame to notice the route change.
Retired LSP URI identities are bounded by the unfiltered current open capture, including an
open document temporarily choosing Plain Text or another language without an LSP route.
Retire an obsolete binding before filtering missing routes; preserve sibling polling and
defer safe re-admission/reset, then prune the identity only when its document leaves the capture.

## Expanded E06 candidate evidence (2026-10-31; independent scoped review pending)

Extended the existing parser/preferences with documented Kate fields and safe catalog language
choices. Explicit choices > enabled modelines > EditorConfig > existing scoped/project/global
layers now apply field by field. Stable opt-in uses the manual/ordinary detected language;
declared language selects other scoped preferences without oscillation or manual/session state.
Changed/removal/disabled/undone declarations update highlighting, close stale help and expire
LSP capture identities. Polling only syncs admitted matching derived URIs, closes obsolete
bindings while keeping siblings live, preserves retirement through an open Plain Text gap and
uses the existing transport reset for safe manual/assisted re-admission. Actual close prunes it.
No grammar/runtime/dependency or session schema was added; selection/IME and E05 owners stay.

Causal RED/GREEN coverage includes Kate, language/routing/EC precedence, unknown candidate
fallback, final-LF physical-record bounds, pending help, sibling continuity, manual re-admission,
open route-less membership and closed-URI pruning. Full final 73 config/742 editor/175 syntax/
1336 UI pass (3 established syntax and 25 established UI ignores), focused 6/1/15, strict affected
all-target Clippy and workspace fmt pass. Native input-handler/marked/full rectangular/virtual
selection and source-version preservation are exercised in GPUI; physical OS application smoke
and final whole-workspace/native/release/global stages remain controller-owned.

Thirteen of fourteen unchanged exact serial release gates pass, each selecting one test:
typing, UI Find, all ten render gates and bounded scan. UI Find 41.824/300 ms, comparison
2.367/10, folds 3.335/8, bounded scan .001/.075/.000 versus 1 ms. Additional broad model
`testing::bench::editing::bench_find` fails only Go to Symbol twice: 3074.610 and 3018.109 versus
unchanged 3000 ms; search/fuzzy/keystroke branches pass. Cause is unresolved, with no claimed
repair or all-pass release result. Read-only proof shows timed Highlighter code (except added
uncalled `declared` method), benchmarks/parser/editor/search/Cargo unchanged from Snapshot22.
Both failed observations stay in the report/freeze for root-owned final-performance adjudication.
The task report records exact UTC commands/exits, scope, limits and frozen source/input/artifact
readback. Canonical E06 and all three Task22 plan boxes remain unchecked.


### Task22 supplemental verification contract — Go to Symbol performance

The unchanged exact release Find gate has twice exceeded the 3,000 ms symbol budget
(3,074.610 and 3,018.109 ms). A CPU sample locates most symbol work in repeated Syntect
parse-line calls; it does not yet explain cache misses. Diagnose the existing bounded
transition cache before changing runtime behavior. Preserve exact text/parser-state guards,
all symbol/docs/checkpoint/cancellation behavior, current memory bounds and public APIs.
Any fix must have a causal regression, beat baseline variance on the unchanged serial gate,
and pass affected syntax/UI, strict and relevant rendering/indexing checks. No setting,
entry point or document byte changes are planned; original failures remain recorded.

Performance investigation narrowed the issue to retained scope-operation payload:~3.93MiB
versus~269KiB text in5,050 entries. Two eviction-policy experiments were measured and
reverted (whole-table4475.357ms; LRU-state3149.936ms diagnostic versus3057.264ms). The
next proposed change compacts operation storage in the existing cache, with bounded
interning charged to the unchanged4MiB payload cap, exact operation decoding and all
current parser/state/cancellation policies retained. No benchmark budget or fixture change.

### Task22 / E06 — verified and accepted

Independent scoped Spec Compliance and Task Quality reviews pass with no required findings.
Default-off Vim/Emacs/Kate declarations, physical edge bounds, catalog-resolved derived languages,
manual > modeline > EditorConfig > scoped/global settings, and inert executable/unknown forms
are implemented and documented. Language changes retire stale help/diagnostics and preserve
shared views, native composition, complete virtual/rectangular selections, undo and document bytes.
Ordinary polling never starts a declaration-selected server; route-less gaps preserve retired
protocol identity until real close or safe manual/assisted re-admission.

Final source passes 73 config, 742 editor, 177 syntax and 1,336 UI tests (3/25 existing ignores),
focused modelines 6/1/15 and 8 symbol tests, strict affected all-target Clippy, workspace formatting
and diff checks. All 14 unchanged exact serial release gates and the real-crates index check pass.
The formerly failing symbol gate passes at 1059.887, 1066.288 and 1082.068 ms / 3000 ms after compact
operation storage in the existing bounded cache. Every original cap and parser/admission policy
remains; catalog additions are atomic and charged within the original 4 MiB payload. Initial
failures, two reverted experiments and corrected strict lint remain preserved. Exact commands,
results, reviewed reports, snapshots and source/artifact hashes live in task-22-root-report.md
and task-22-root-final-corroboration.json. No final whole-workspace/native/release claim is made.


## Expanded L03 behavior contract (2026-10-31, before product edits)

Extend the existing explicit Format Selection action and request/staging path. No setting,
automatic save hook or new formatting engine is added. Format Document retains its separate
existing behavior. Show Format Selection in the Goto > Language Server menu and command palette
only after the active document route advertises rangeFormatting; unsupported direct/custom
invocations report the existing capability error without falling back to document formatting.
Capability discovery reuses polling and must preserve E06's assistance-only admission of derived
languages, with no new server start from menu/palette construction.

Capture one original document rope/version, language identity/provenance and complete source
selection state/generation. Send each nonempty physical range once using the negotiated UTF-8,
UTF-16 or UTF-32 codec and original tab/indent options. Skip empty carets and pure virtual
selections; all-empty input reports a selection prompt and sends no formatting request. Reject
overlapping source ranges before any request. Aggregate every successful reply against the
same original snapshot, reject malformed/overlapping edits (including cross-reply conflicts),
and validate the complete batch before one existing replace_ranges edit/undo transaction.
A null reply contributes no edits; any error, cancellation/restart or invalid/stale reply drops
the complete batch. Do not mutate or write disk between range replies.

Freshness includes originating view identity, full Selection/SelState metadata, primary and
monotonic generation, including away/back moves without a frame. Preserve E05 physical offsets,
virtual columns, goals and rectangular provenance through the existing source-follow mapping.
Recheck language/registry/service generations, version, writable/read-only/Follow/shared-view
alias and protected write guards at application. Refuse active composition or immutable source
views and drop replies if composition starts in any shared target view. Preserve established
completion, signature, navigation and full-document formatting contracts.

Acceptance: causal real-transport/editor regression for multiple nonadjacent Unicode/tab ranges
with an empty primary; UTF-8/16/32 wire positions, one Undo/Redo and shared-view text; all-empty
and virtual-only no-op; overlapping requests, overlapping/malformed/invalid-position replies,
null/error/unsupported cases and second-reply atomicity; same-cursor secondary/metadata and
no-frame away/back staleness, restart/disable/route changes, read-only/Follow/IME/write/alias guards;
actual palette/menu reachability after capability discovery. Retain all old regressions and
benchmark workloads/budgets. Run full affected UI/editor/syntax suites, focused preservation,
strict affected all-target Clippy, workspace fmt and diff checks. Exact unchanged hot gates are
needed only if implementation changes a measured hot path. Snapshot23 stays fixed, and all
canonical/Task23 boxes stay unchecked for independent controller acceptance.


## Expanded L03 candidate evidence (2026-10-31; independent scoped review pending)

Format Selection now sends every nonempty physical selection against one original rope/version,
using negotiated UTF-8/16/32 positions and existing indentation options. Empty carets and pure
virtual selections contribute no request or padding. Overlapping source ranges are refused before
server startup; null replies contribute nothing, while failed/cancelled/invalid/overlapping replies
reject the complete batch. The existing staging and replace_ranges transaction preserve one
Undo/Redo, shared text and E05 virtual/rectangular provenance. Document formatting stays separate.

Advertised rangeFormatting capabilities from admitted polling gate the native menu and actual
workspace palette. Those surfaces do not start servers. A range-only Request context pins full
SelState, originating view and monotonic generation, including no-frame away/back changes.
Original version/language/provenance/registry/service and shared writable/Follow/IME/alias/writer
checks remain. Source/save/staging, native input, codec/transport, parser/registry and benchmark
owners are byte-identical to Snapshot23. No setting, persistence field or dependency was added.

Final checks pass:10 focused range tests,742 editor/177 syntax/1344 UI tests plus doctests,
strict affected all-target Clippy, workspace fmt and diff checks;3 syntax/25 UI existing ignores
remain. Actual palette success, native EntityInputHandler composition refusal, held-second-reply
transport cancellation, exact Unicode/tab wire positions, overlap/null/error/stale cases and
single-step shared Undo/Redo are exercised. All168 prior language test/helper functions remain.
The final old parser-warmup test passes; its historical Task19 cause-unknown observation remains.
No measured hot path changed, so no release performance gate is claimed; all workloads/budgets
stay untouched. Root owns physical OS/native smoke and final global/workspace/release stages.

The report distinguishes causal REDs from test-authoring, GPUI blocking-executor, thread-local
Registry setup and strict test-lint corrections. All raw failures/UTC commands/exits remain.
The final373 source/build/script/fixture/asset inputs were pinned before final checks, with
actual test executable hashes and Snapshot23 outside-delta proof retained in task-local evidence.
Snapshot23 was never recaptured. Canonical L03 and all three Task23 plan boxes remain unchecked.
No helper/reviewer agent, product commit, push, PR, publish, release, reset, checkout or cleanup.

### Task23 / L03 fix round1 — active split-pane menu publication

Defaults and formatting behavior remain unchanged. Clicking another visible source view must
publish that view's advertised Format Selection availability even when delimiter overrides match.
Use the existing deferred/coalesced native-menu refresh when focus changes the active view;
retain the delimiter check for an already-active view. Menu/palette construction keeps cached
capability admission and starts no server. No formatting, selection, transport, save or undo
policy changes. The regression uses actual text clicks between supported/unsupported groups,
equal delimiter overrides, active view/focus assertions, and the existing menus_built counter
at the actual native installation path in both directions, followed by ordinary language polls.
GPUI's test platform discards native menus, so correlate that real installation counter with
the stable current payload rather than claiming retained OS menu state or a physical smoke.
Keep one original batch/complete-selection guards, all earlier tests, strict checks and final
input identity. No measured per-key/render/search/index path changes are planned.

The same P2 occurs when a modeline removes the active language route: the cached accessor is
already false before the next poll, so comparing accessor values misses the formerly installed
true action. A real modeline-edit regression confirms this independently of split focus. Record
range-formatting availability beside the existing last-installed delimiter state, update it only
when the active window actually installs its menu, and compare poll output with that installed
value. Inactive windows keep their cached capabilities without scheduling menu installations.
This remains a cold menu-publication extension, with no per-keystroke source-event work, server
lifecycle change or persistence field. Both regression paths must publish the correct menu state.

### Task23 / L03 — verified and accepted after fix round1

Independent Spec Compliance and Task Quality are approved; required P2 menu publication is
addressed with no new required finding. Format Selection covers every nonempty physical range
against one original version, validates the full reply batch before one undoable replacement,
and preserves complete selection, IME/shared/read-only/Follow and protected edit guards. Cached
advertised capabilities gate the native menu and palette without starting a server. Actual split
focus and modeline route-loss regressions prove the installed menu refreshes through the existing
coalesced active-window path, using transient publication state beside the delimiter record.

Final 12 focused range tests and all1,346 UI tests pass (25 established ignores), with strict UI
all-target Clippy, workspace fmt and diff checks. Unchanged backend source/executables remain
covered by V1's742 editor/177 syntax passing tests; no redundant backend/performance/physical-OS
claim. Root corroborated1,708 frozen hashes,373pre-check inputs,384full/387fix outside files,
838archivedV1 mappings,3actual executables and10fix commands. All373source inputs remain stable
through metadata-only acceptance. Reports, review, actual logs and source/artifact proofs remain
in the Task23 evidence. Final whole-workspace/release/native/broad review is still pending.

### Task24 / L04 — pre-edit implementation contract

`format_on_save` remains off by default and keeps the existing language/path/project settings
layering. Explicit Save/Save As/Save All and accepted F04 existing-file autosave use one document
formatter in the existing reserved save slot, before normal cleanup and protected disk capture.
Add a positive `format_on_save_timeout_ms` file setting, default 5000, covering the formatter's
queue/start/request work; cancellation uses the existing LSP cancellation protocol. Unrelated
request deadlines remain unchanged. Manual Format Document and L03 range-only selection pins,
E05 selection origins, E06 route/admission and installed menu capability publication remain intact.

Unsupported/missing/disabled formatters, invalid/overlapping output, errors, timeout, cancellation
or a changed source/version/route/registry/service fail preparation, perform no cleanup or disk
write, and retain dirty edits. Source/shared-view composition, immutable/read-only/Follow state,
alias and writer/read-ticket protections are rechecked before the single undoable edit. Null
formatting output is a valid no-op and may save normally. A source with no named LSP route cannot
format its first Save As; an enabled request must report that failure and await an explicit bypass.
Existing read-only Save As conversion/copy behavior remains protected and does not format.

Add always-reachable File > Save Without Formatting and File: Save Without Formatting palette
commands, plus a registered custom-bindable action. It uses normal Save/Save As path selection,
composition snapshot, cleanup, safe writer, saved identity and callbacks. Inspection corrected
the initial composition-commit assumption: ordinary Save intentionally includes native previews
without unmarking, and the bypass preserves that exact behavior. Bypass requests
share the existing serialization but cannot merge with formatter-required requests. Save a Copy
As remains independent, with no source formatter, cleanup, unmark, save callbacks or rebinding.
Explicit failures show the normal save error with the named bypass; autosave reports a status
and suppresses retries of the same failed edit until edits/settings or an explicit save change it.

Acceptance uses causal real fixture/editor failures before product edits: unsupported/error/
malformed/overlapping replies and stale typing must keep disk bytes and undo history unchanged;
success/null plus cleanup, shared one-step Undo/Redo, repeated/merged saves and a queued bypass;
formatter-specific default/configured timeout and cancellation wire messages; automatic failure
suppression/success/cancellation; actual shared command/palette/native installation reachability,
untitled bypass, Save Copy isolation and all existing save, formatting, recovery/IME guards.
Full affected UI/syntax/config/editor suites, strict affected all-target Clippy, workspace fmt,
diff checks and unchanged relevant exact performance gates if a measured hot owner changes are
required. Snapshot24 is fixed; canonical L04 and all Task24 boxes stay unchecked for root review.

Task24 self-review extends the stale guard through capture: successful formatting/cleanup pins
its resulting document version and route in the same existing save slot. Recheck that validation
immediately before capture, in the capture update. An edit, route change or composition after
preparation yields must fail the write and leave the legitimately formatted/current edits dirty.
Default-off and explicit bypass saves keep their ordinary capture semantics. A test-only held
pre-capture boundary proves this gap causally; the initial final-input pin is archived and the
complete final source is repinned before covering suites and strict checks.

### Task24 / L04 — verified candidate, independent scoped review pending

Opt-in document formatting now fails its existing save slot on unsupported/error/invalid output,
timeout, cancellation or stale source, retaining dirty edits and original disk bytes. Its own
positive-millisecond timeout setting defaults to 5000, without changing unrelated LSP defaults.
Successful formatting and cleanup pin the resulting document through the final capture update;
post-preparation typing, route changes and composition cannot silently enter the saved snapshot.
Autosave uses the same path and records its own formatted attempt before capture, suppressing a
rejected edit while allowing a later user edit or codec/settings change to retry normally.

File > Save Without Formatting and the real command palette expose the registered bypass. It
keeps ordinary cleanup, composition capture, safe writing, saved point and callbacks, including
untitled Save As. A policy tag in the existing queue keeps normal/bypass requests distinct.
Save a Copy As remains independent and byte-identical in its owner. Manual document formatting,
L03 range-only pins and installed-menu state, E05 origins, E06 admission, save writer/leases,
shared undo, recovery/history and F04 delay/cancellation owners retain their accepted behavior.

Final checks on the V4 pre-check pin pass: 23 focused formatter cases, all 31 autosave cases,
74 config / 742 editor / 177 syntax / 1,360 UI tests plus doctests, strict affected all-target
Clippy, workspace fmt and diff. Three syntax and 25 UI established ignores remain. Real fixture
transport checks prove default/configured timeout and cancellation; native installation/dispatch,
palette, EntityInputHandler, atomic shared Undo/Redo, ordered repeated/bypass saves and failed-close
waiters are covered. Compiled REDs also prove the preparation/capture gap and recursive autosave
retry, with a later-user-edit retry retained. All failures, corrections and three superseded
input pins remain in the task report and literal UTC/exit logs; no assertion, deadline or gate
was weakened. The final 373 source/build/fixture/asset hashes and four task scripts were pinned
before checks and revalidated afterward, with four actual test executable hashes retained.

The task delta is 16 files against immutable Snapshot24, with 378/394 outside-delta files unchanged.
Static preservation verifies 113 editor/parser/project/measured owners plus the write, copy,
manual/range and staging paths. No measured hot path changed, so no release performance gate is
claimed. Root retains physical OS/native smoke and final global/workspace/release/performance
stages. The frozen report/diff/manifest/logs and readback are in Task24 evidence. Canonical L04
and all three Task24 boxes remain unchecked for fresh independent root review; L05 is untouched.

### Task24 / L04 — verified and accepted

Fresh independent Spec Compliance and Task Quality pass with no required findings. Formatting
stays off by default and uses the same explicit/autosave slot, with a scoped 5000 ms default
timeout and no silent unformatted write on error, unsupported server, cancellation or staleness.
The final capture validation closes the preparation yield; autosave suppresses its own failed
formatted snapshot while allowing later user edits to retry. File/palette/custom-bindable Save
Without Formatting keeps normal cleanup, safe writing and callbacks, with a distinct queue
policy. N22 Copy, manual/range formatting, shared Undo/Redo, IME, leases and E05/E06 remain intact.
No dependency, schema, second queue, general hooks or unrelated transport deadline change.

Final 23 formatter/31 autosave tests and 74 config/742 editor/177 syntax/1360 UI tests pass,
plus strict affected all-target Clippy, workspace fmt, diff and preservation checks. Root
independently corroborated 854 frozen hashes,373pre-check source inputs+4scripts,378outside
files,4actual executables,46command rows+2finalization artifacts and six compiled causal REDs.
The exact reviewed report is archived; all373source inputs and four scripts remain unchanged
through this metadata acceptance. The review's physical OS/production-server/global-final
limitations remain final root verification work, not claims made by this scoped fixture gate.


## Expanded phase — L05 / Task25 behavior contract (before product edits, 2026-10-31)

Existing stable manifest IDs, `files.names`/`extensions`/bounded linear `first_line` (including
shebangs), arbitrary protocol IDs/families/argument arrays, root markers and bundled/installed
syntax fallback remain the implementation. Add `files.globs`, compiled once with the existing
EditorConfig glob engine: case-sensitive basename matches without `/`, full-path matches with
`/` (use `**/` for any parent). Exact names and longest extensions win before globs, then
first-line patterns; existing user-before-bundled/ID and content-pattern precedence remain.
No manifest parser, WebAssembly or executable/plugin runtime is activated.

Reload Customizations and saving a support-folder language manifest keep the existing global
request epoch, final save-capture validation and stale result refusal. Manager must compare
commands/options per family, retiring only affected root/family sessions; unchanged routes
survive registry replacement with the same process and didOpen serial. Same-family protocol
or root-marker identity changes keep the accepted fresh-transport policy. Disabled/missing or
invalid family config never retires unrelated clients; errors name the program or field and
are visible through existing status/alerts. Invalid manifests remain isolated by path. Server
preferences validate commands/options per family without dropping valid sibling overrides.
A reload retries cached startup failures while preserving unrelated live sessions. Immediate
popup cancellation/cache invalidation must retain signature/range/manual/save freshness and
L04 deadline/own-edit/final-capture guarantees, including old background formatter replies.

Executable config remains explicit user configuration: global Preferences or support-folder
Languages only. Existing project/scoped application-setting rejection remains; no project
folder manifest discovery or implicit project executable authorization. No new trust mode.

Acceptance adds causal regressions for glob detection/validation, two-root/two-family live
process and session preservation across no-op/affected/invalid/missing-executable reload,
per-family preference validation and pending signature cancellation. Extend the genuinely
uncompiled fixture language through glob/shebang detection, fallback highlighting, Unicode
completion/diagnostics/definition and root markers; retain existing L03/L04/E05/E06/N22 coverage.
Run full syntax/config/project/UI (and editor save regression coverage), strict affected
all-target Clippy, workspace fmt/diff and applicable unchanged exact serial detection/index
release gates. Pin final inputs before final checks, preserve raw UTC/exit failures and actual
executables, then Snapshot25 diff/outside-delta proof plus freeze/readback for root review.
The canonical L05 entry and all three Task25 boxes remain unchecked.

L05 pre-final self-review amendment: reuse the EditorConfig engine's leading-slash normalization
on the full input path too, so an anchored absolute glob works without changing the shared
matcher or relative/scoped semantics. The new literal absolute-path regression fails before
this one-line normalization; retain the first pin/checks as superseded evidence and repin all
final inputs before final checks. No prior parser/detection gate or budget is changed.

### Expanded phase L05 worker evidence — independent review pending

Added precompiled EditorConfig `files.globs` with documented case, basename/full-path and
leading-slash semantics; reused existing stable arbitrary IDs, bounded shebang detection,
protocol/root routing and Plain Text/installed/bundled syntax fallback. Manager now reconciles
server configuration per family and retains unchanged root/family processes, openings and
capabilities across registry reload. Changed protocol/settings/root identities retain existing
fresh-transport isolation. Missing startup tools are retried on reload; invalid family settings
produce a named warning while valid siblings remain active. Reload cancels pending signature
assistance and invalidates assistance caches under the existing service/save freshness guards.
Global/user-manifest executable configuration remains explicit; project/scoped server settings
and project-local manifest discovery remain rejected. No dependency, parser/plugin/WASM runtime
or new trust framework was added.

Six compiled causal RED failures cover absent globs, no-op reload PID replacement, pending
signature cancellation, missing invalid-family warnings, accepted whitespace manifest commands
and anchored absolute-glob normalization. Final fixture coverage proves uncompiled
`noncompiled-42` glob/shebang detection, Plain Text fallback, Unicode UTF-16 completion and
diagnostic offsets, target navigation, root markers, two-root/two-family session preservation,
missing-tool recovery and rejected project executable settings. Two fixture-authoring failures
(Apple Python shim alias and a miscounted literal Unicode first line) remain in raw logs;
assertions were corrected without changing budgets or relaxing behavior. The settings observer
cancellation test already passed and is recorded as covering verification, not causal RED.

Final V3 inputs were pinned before final checks: 373 source/build/fixture/asset hashes plus four
task scripts. Full affected suites pass 75 config, 742 editor, 89 project, 179 syntax and 1365 UI
(2450 total), with 3 syntax/25 UI established ignores; all five doctest suites pass. Focused
runtime selection passes15. Strict affected all-target Clippy, workspace fmt and diff checks
pass. Preservation verifies 106 accepted editor/project/save/transport/native/fixture/performance
owners, 194 original language tests/helpers and 59 unchanged request/save/edit methods against
Snapshot25; the canonical plan is unchanged. V1/V2 anchors and all failures remain archived.

Four unchanged exact serial release gates pass after compilation/tests finish: 100MB load
87.487/1500ms, 10MB line4.466/300ms, bounded modeline scan .001/.083/.000ms against1ms each, and
index313files/9393symbols in7.4s against the existing1800s timeout. Index memory is reported by
the existing gate, without claiming a new numeric memory budget. No isolated Registry-detection
timing gate exists. Actual UI evidence is GPUI Reload Customizations, Settings alerts, completion
and Go to Definition with real fixture subprocesses; no physical OS or real external-server
smoke is claimed. Detailed APIs, limits, UTC commands/exits, actual executable hashes and
Snapshot25 outside-delta/freeze/readback are in
`.superpowers/sdd/editor-expanded-features-plan/task-25-report.md`. L05 and all three Task25
plan boxes remain unchecked for root's independent review/final global checks.

### Task25 / L05 fix round1 — active settings observer contract

Keep all L05 defaults and selective session reconciliation. When global server commands/options
or the LSP-enabled setting changes, immediately cancel workspace navigation/signature work,
clear language completion/range-capability caches, close editor assistance and forget old trigger
metadata through the existing owners. Retain the shared service epoch, L04 save guards and
unchanged server transports. Other settings keep their existing view/epoch behavior; no new
persistent state, transport default or per-frame invalidation is introduced. Use a transient
observer snapshot to distinguish server-setting changes.

The causal regression must keep its GPUI window active and editor focused before and after a
valid settings edit, without a warning prompt. Cover pending and displayed signatures, options
changes and LSP disable; assert actual token/state/cache cleanup and unrelated live PID/didOpen
continuity. A real held fixture request must observe wire cancellation and release the Manager
for reconciliation. Strengthen the invalid-family regression so inactivity cannot supply its
cancellation. Correct the architecture map's obsolete all-session-restart description. Full UI,
strict UI all-target Clippy, fmt/diff and exact frozen-source checks follow; unchanged backend
and measured owners retain their verified V1 evidence. Fresh scoped re-review is required.

### Task25 / L05 fix1 — verified candidate pending scoped re-review

The active/focused Settings regression causally failed before the observer fix. Valid server
command/options or enabled changes now use existing cancellation/cache/popup owners while
retaining the service epoch and selective Manager. Pending/displayed/disabled cases and the
real held transport pass, including wire cancellation, Manager release and unrelated session
continuity. The older epoch-choice regression retains request rejection and now verifies that
Settings dismisses choices and ignores a queued old item. Its first full-run failure is retained.
Final17runtime/1epoch/1367UI tests, strict UI all-target Clippy, fmt/diff/preservation and source
pin readback pass.105protected owners/59request-save-edit methods remain byte-identical;
193old helpers/tests are identical and one is strengthened. Backend sources/executables retain
V1 coverage; the cold observer fix claims no new release gate. Final373source+3scripts were
pinned before checks and remain unchanged. Full task13files/fix4files; original V1 archive and
both snapshots remain intact. Canonical24/64 and L05/Task25 boxes stay unchecked until re-review.

### Task25 / L05 — accepted after fix round1

Fresh scoped re-review marks R1 ADDRESSED and approves Spec Compliance and Task Quality, with
no new required finding. Runtime language configuration, precompiled globs, bounded shebang
matching, syntax fallback, selective root/family reload and executable boundaries are complete.
The Settings observer cancels obsolete assistance in active/focused views while retaining the
service epoch and unchanged live transports. The old-epoch test preserves request refusal and
now also verifies dismissed UI/ignored queued choices. No dependency or plugin/trust runtime.

Final17runtime/1epoch/1367UI tests and strict UI all-target Clippy/fmt/diff/preservation pass.
Unchanged backends retain75config/742editor/89project/179syntax passing checks. Four original
exact serial release gates remain V1 evidence; the cold observer fix claims no repeated gate.
Root rechecked1308frozen hashes,373pre-check inputs+3scripts,856archived V1 mappings,381full/
390fix outside files and21fix command rows. All373inputs/3scripts stay stable through metadata
acceptance. Canonical25/64; exact reviewed fix report and acceptance record preserve the evidence.
The plan's commit policy now reflects the latest user authorization for root's accepted work.
Final whole-workspace/native/release/performance checks and broad review remain outstanding.

## Expanded phase — G01 / Task26 behavior contract (before product edits, 2026-10-31)

Extend the existing explicit sidebar, Tools > Git and bindable palette Fetch/Pull/Push/Remotes
& Upstream actions. No network work occurs on open, status, refresh, local discovery or config
changes. Fetch shows the selected remote URLs and configured refspecs; Pull confirms the current
branch, native upstream remote/ref and URL, and uses fast-forward-only with no implicit rebase
or merge commit even when repository/global pull settings request those strategies. Push shows
its selected remote URLs/current branch/upstream and editable destination before dispatch and
continues ordinary explicit non-force/non-mirror current-branch push. Upstream changes remain
local Git configuration; no new app settings or stored job state are needed.

Reuse asynchronous prepared targets, root/HEAD/config-origin/include/config-root freshness and
worker-owned metadata/config leases. Pull retains its checkout lease and all shared-window
unsaved/read-only/IME/pending-save guards; Fetch/Push retain dirty source and normal saves.
Inherited helpers, SSH configuration, hooks and signing remain in force; terminal credential
prompts stay disabled. Cancellation before dispatch contacts nothing. A helper/SSH cancellation
or subprocess failure returns useful bounded Git diagnostics, releases leases and never reports
success; submitted processes keep the existing finish-normally policy. Stale project, selection,
branch, remote URL, upstream or effective configuration requires reopening controls.

Acceptance uses only disposable repositories, local bare remotes and fixture processes: explicit
fetch, linear ff-only pull (including merge-preferring config), divergence refusal under merge
and rebase settings without history/index/worktree changes, ordinary push/upstream and rejected
non-fast-forward push, missing upstream, native credential/SSH cancellation and command failures.
UI checks exercise actual target display, cancel-before-dispatch, async error/lease release,
no-contact open/status/local controls and stale target/config refusal. Existing partial staging,
ordinary commits/signing, worktree/stash and document lifecycle suites remain required. The
expanded ff-only policy supersedes first-phase G01's configured merge/rebase behavior; the
historical audit/progress evidence stays unchanged. Snapshot26 was captured once after accepted
Task25 at 82987ec and is never recaptured. Canonical G01 and all three Task26 plan boxes stay
unchecked until root's independent review.

### G01 Task26 presentation amendment (before repair)

The first candidate's source strings contained full targets, but shared picker details clip
single lines and form titles have a fixed-height header. Reuse the existing wrapped native
confirmation path after Fetch selection and after Push form submission, showing the complete
prepared remote name/URLs, refspecs or source/destination refs, and current branch/upstream.
Only confirmed actions enter the existing validated worker/lease path; Cancel contacts nothing.
This adds one deliberate confirmation to Fetch/Push, without changing the common picker/form
widgets or introducing a new job framework. Preserve all existing dirty/config/include/root,
stale-context and worker-owned lease checks at their intended post-confirmation stages. Add a
causal long-target/cancel regression, archive V1 evidence, then pin and rerun final checks.

### Expanded G01 / Task26 worker evidence — independent scoped review pending

The existing asynchronous remote owner now performs explicit native-upstream fast-forward-only
Pull and ordinary non-force/non-mirror Push. Prepared fetch refspecs and native upstream remote/
ref atoms feed the exact target UI; custom tracking mappings default Push to the remote branch.
Fetch selection and Push form submission use wrapped native confirmations before any transport,
retaining context/config revalidation, cancellation and the existing worker-owned lease path.
Eight compiled behavior failures across seven test functions were observed before their fixes;
all intermediate compile/fixture/strict failures and the superseded V1 pin/evidence remain.

The final pre-check pin binds373 source/build/fixture/asset inputs and5 scripts. Fresh final
14Git/21UI focused tests, full82Git/1372UI with25 established UI ignores and doc tests pass;
strict Git/UI all-target Clippy, workspace fmt, diff, preservation and pin readback pass.
All49 original assertions in confirmation-adapted UI tests remain, alongside76 byte-identical
existing tests/helpers and387 unchanged outside files from the once-captured Snapshot26.
Actual native Git2.56 processes use only disposable local bare remotes and controlled credential/
SSH fixtures; no real credential service or remote is tested. No measured hot path or benchmark
workload/budget changed, so no new release/performance run is claimed. Final root whole-workspace,
release and physical native verification remain pending. Report, raw UTC/exit command logs,
source pin, task-local diff and freeze/readback evidence are under
`.superpowers/sdd/editor-expanded-features-plan/task-26-*`. Canonical G01 and all three Task26
plan boxes remain unchecked for independent root review. No worker commit, push, PR or helper.

### Task26 / G01 — verified and accepted

Fresh independent Spec Compliance and Task Quality pass with no required findings. Explicit
remote/upstream actions use complete captured targets and wrapped confirmations; native upstream
identity handles custom tracking and local-dot Pull. Fast-forward-only Pull supersedes configured
merge/rebase behavior, while ordinary non-force/non-mirror Push, native hooks/signing and inherited
helpers remain intact. Cancellation, stale targets, dirty/shared/read-only/IME/pending-save guards
and worker-owned metadata/config/include leases are covered using disposable local fixtures only.
No automatic network, dependency, persistent job framework or common widget change was introduced.

Final14Git/21UI focused and82Git/1372UI full tests, strict Git/UI all-target Clippy, workspace fmt,
diff and preservation pass. Root rechecked1295final mappings+16binary paths,373pre-check inputs+
5scripts,387outside files,820V1 archive mappings,55command rows and4initial/corrected finalization
rows. Initial1289frozen mappings remain valid through explicit metadata relocations. Actual final
combined Git5b/UI binaries match the logs; standalone Gitb43 is separate earlier evidence. All
373inputs/5scripts stay stable through acceptance. Eight compiled causal failures and12nonzero
invocations remain. No measured hot path changed, so no extra performance gate is claimed.
Canonical26/64; final whole-workspace/release/native/performance gates and broad review remain.


## Expanded phase — G02 / Task27 behavior contract (before product edits, 2026-10-31)

Retain the existing Git sidebar, Tools > Git and palette stash entries, cancellable list/patch
readers, create form and confirmed Drop. Saved staged/unstaged tracked changes are included;
Include untracked files stays an explicit default-off choice. Preview remains a bounded,
read-only actual patch including chosen untracked content and binary summaries. Prepared
repository/HEAD/root identity plus the whole ordered selector/object-ID stack remain mandatory;
application uses the selected object ID and native Git conflict behavior. Pop removes an entry
only after success through the existing locked native transaction. No persisted setting changes.

The missing expanded branch is completion routing: a failed Apply/Pop with actual unmerged
paths must expose the existing Git sidebar/diff and first conflicted source with Next Conflict
and Accept Ours/Theirs/Both available. Preserve the original Git failure and retained stash
identity through navigation. A failure without actual conflicts must not claim success or open
conflict tools. New project/root/selection/surface choices prevent late automatic navigation.
Dirty/read-only/shared/IME/save protections and worker-owned leases remain intact; clean source
reloads use the existing document reload operation only after identity/version/save-point,
shared composition, disk and write freshness checks. Never replace unsaved buffers to show disk.

Acceptance uses disposable repositories only: staged/unstaged/optional untracked, multi-entry
selection, stale indices/objects/stack/repository/HEAD, actual Apply and Pop conflicts with exact
stash OID retention and usable undoable conflict choices, ordinary Git failures, and late dirty/
shared/IME/replacement surface protection. Existing preview/drop/ref-lock/native-format/error,
status/partial staging/commit/signing/G01 remote and worker/save lifecycle regressions stay intact.
Add a causal UI regression before changing the missing route, complete README/architecture docs,
then run focused and full Git/UI tests, strict all-target Git/UI Clippy, workspace fmt/diff and
Snapshot27 scope proof. No measured hot path is targeted; unchanged performance gates need no
new run unless the implementation changes one. Final source/build/scripts are pinned before
final checks, actual executed test binaries are hashed, all failures are retained, and a frozen
report/diff/readback hands source/check ownership to root with canonical/plan boxes unchecked.


### Expanded G02 / Task27 worker evidence — independent scoped review pending

The missing completion branch now opens the existing Git sidebar/diff and first conflicted
source after a failed Apply/Pop with actual native unmerged status. The selected object/stack
is retained, the original Git error stays in Git/status, and existing Next Conflict plus undoable
Accept commands handle resolution. Prepared backend identity/stack validation and locked Drop,
configured signing, ordinary commits, partial staging, G01 remote policy/confirmations and all
shared Git-worker leases remain unchanged. Create/Drop do not enter the new conflict route.

Two preimplementation causal UI tests failed because conflict tools stayed closed. Both now
traverse actual disposable Apply/Pop conflicts, check exact stash OID retention, actual source
markers/caret/focus, failure reporting, palette Accept Both and Undo without writing source.
Additional checks cover a real index-lock failure without conflicts, a replacement picker/Git
selection, late shared dirty/read-only/IME sources, explicit untracked-toggle/patch/index restore,
forged display-index/object combinations, an older entry and a replaced root with unchanged
Git-directory identity. All original regressions and assertions remain intact. One intermediate
combined run exposed an invalid new selection_active guard (source focus intentionally clears
it) and a new fixture's deferred-open/palette ordering; both were corrected without weakening
acceptance. Both nonzero test logs and preliminary inspection diagnostics are retained.

Final source/build/fixture/asset and task-script pin preceded final checks: 373 inputs and five
scripts. Final focused19 Git/22 UI and full85 Git/1380 UI pass, with25 established UI ignores
and both doc-test targets clean. Actual final combined Git5b423177ed4563a2/UI fd79e6e0c5405c24
executables have matching immediate post-run hashes in both logs. Snapshot27 is unchanged and
all394 before blobs match accepted dfc9dfd; six scoped files,388 outside files and91 existing
test/helper functions are preserved. Final strict/fmt/diff, source-pin readback, task-local
patch/manifest and freeze/readback outcomes are recorded in task-27-report.md and raw logs.

Native Git2.56.0 executed disposable repository/ref-lock/conflict/signing/local-remote fixtures
on macOS; UI behavior ran in GPUI TestPlatform. This does not claim a physical Orlo window or
release build. No measured hot path, performance workload, threshold or ignore changed, so no
extra release performance run is warranted. Root owns independent review and final global/
release/native/performance stages. Canonical G02 and all three Task27 plan boxes stay unchecked.

### Expanded Task27 / G02 acceptance

Independent scoped Spec Compliance and Task Quality review passes with no required fixes. Root
corroborated808 frozen source/evidence hashes,11 binary/tool paths,373 pre-check source inputs
and5 scripts,388 outside files and91 retained test/helpers. Final focused19 Git/22 UI and full85
Git/1380 UI(+25 existing ignores), strict affected all-target Clippy, workspace fmt and diff pass.
Both nonzero test runs remain retained. Canonical G02 and Task27 plan boxes are now complete;
final workspace/release/native/performance checks remain pending after all selected features.

## Expanded phase — G03 / Task28 behavior contract (before product edits, 2026-10-31)

Retain the existing Git sidebar, Tools > Git and palette worktree entries, cancellable local
readers, explicit existing-branch/new-branch-at-HEAD create form and confirmed Remove/Prune.
Creation keeps the current checkout/window; Worktrees > Open in New Window then uses the normal
`open_window`/`Workspace::open_path` route, preserving the original project, unsaved/shared text
and native composition. No setting or persisted state changes. Removal keeps the branch and
refuses main/current/dirty/untracked/locked worktrees without force, reset or implicit unlock.

The missing branch is native prunable state: retain Git's NUL-delimited porcelain annotation
and optional reason alongside missing/locked states, list it in the picker, and confirm only
the registrations Git marks prunable. Discovery explicitly uses the existing prune policy's
`--expire now`; native Git immediately expires eligible stale metadata, even future-dated
index metadata. Missing paths alone do not establish eligibility. Locked/live registrations
remain under native protection; existing folders with mismatched checkout/backlink identity
still require repair before any operation. Target/common-directory/registration/HEAD plus
project/root/selection/surface freshness and shared unsaved/IME/save/write leases stay intact.
Changed eligibility/locks/registrations while a prompt is pending must cause stale refusal.

Acceptance uses disposable repositories and child-local configuration only: linked discovery,
literal Unicode/newline paths and reasons, explicit new/existing branch creation followed by
normal-window opening, native checked-out-branch/delete protections, dirty/untracked/locked
removal refusal, native eligible-only stale pruning with cancellation and stale confirmation,
all-window unsaved/composition/retained-save/worker safeguards. Add a causal UI regression for
the missing native state/reason before runtime edits; directly test the extended backend field
afterward and retain every existing regression. Run full affected Git/UI suites and warranted
dependent checks, strict all-target Git/UI Clippy, workspace fmt and diff. No measured hot path
is targeted. Pin final source/build/fixture/asset/scripts before final checks, retain raw actual
commands/UTC/exits and failures, hash actual executed final binaries, prove Snapshot28/outside
scope preservation and freeze/read back the complete task delta before ownership returns to
root. Canonical G03 and all three plan boxes remain unchecked pending independent scoped review.

### Expanded G03 / Task28 worker evidence — independent scoped review pending

The first-phase lifecycle is retained. The missing native prunable state is now exposed as
`Worktree::prunable: Option<String>` from NUL-delimited porcelain, with discovery explicitly
matching Prune's existing `--expire now` policy. The list shows the native reason separately
from missing/locked. The wrapped confirmation names only native eligible registrations and
their reasons; an empty eligible set reports status without prompting. Prepared root/HEAD/
common-directory/registration/branch identity, normal new-window opening, safe native removal
and all shared unsaved/IME/save/write/worker protections remain unchanged. README and architecture
document this state, explicit create-then-open flow, safe errors and metadata-only pruning.

Two compiled preimplementation UI regressions failed because the list and prompt omitted the
native prunable reason; all nine existing lifecycle UI tests passed. Both assertions now pass.
Two new backend tests compare each entry's prunable field with native porcelain, prove immediate pruning
of ordinary and future-index stale metadata while retaining locked/live entries and branches,
and verify checked-out-branch create/switch/delete refusals without changing linked contents.
Additional actual UI tests create with both explicit branch choices, then open each normally
while preserving the original unsaved document/window, reject an already-used branch, and refuse
pruning after a new lock appears during confirmation. Locked-only state prompts for nothing.
All 22 original worktree tests/helpers remain byte-identical.

The final pre-check pin records 373 source/build/fixture/asset inputs and six task scripts at
2026-10-31T18:45:38.533822Z; each runner stage verifies membership/hashes before and after.
Final focused15 Git/15 UI and full87 Git/1384 UI tests pass, with25 established UI ignores and
both doc-test targets clean. Strict affected all-target Clippy passes; final workspace fmt/diff,
Snapshot28 preservation and freeze/readback are recorded in task-28-report.md and raw logs.
The actual combined Git5b423177ed4563a2/UI fd79e6e0c5405c24 executables match their immediate
post-run hashes in both final logs. Snapshot28 was never recaptured; its394 before blobs match
accepted6b99923, seven scoped product files changed and387 outside files are unchanged.

Three nonzero commands remain visible: an exploratory incorrect future-index expiry assumption,
a new-test pending_prompt API compile error and the two compiled causal behavior failures.
The first two were corrected and are not called causal RED. Native Git2.56.0 exercised fresh
disposable repositories and the actual expiry/lock/ref/file behavior; UI interactions ran in
GPUI TestPlatform. No physical app/native-alert layout, whole-workspace test/release build or
extra performance claim is made. No measured hot path, budget, workload or ignore changed.
Root owns independent scoped review and final workspace/release/native/performance gates;
canonical G03 and all three Task28 plan boxes remain unchecked until that review passes.

### Expanded Task28 / G03 acceptance

Independent scoped Spec Compliance and Task Quality review passes with no required fixes. Root
corroborated810 frozen source/evidence hashes,11 binary/tool paths and6 supplemental mappings,
373 pre-check source inputs and6 scripts,387 unchanged outside files and22 retained test/helpers.
Final focused15 Git/15 UI and full87 Git/1384 UI(+25 existing ignores), strict affected all-target
Clippy, workspace fmt/diff and native disposable eligibility checks pass. Three executed nonzero
runs and the separate pre-execution audit-wrapper rejection remain retained. Canonical G03 and
Task28 plan boxes are complete; final workspace/release/native/performance gates remain pending.


## Expanded phase — G04 / Task29 behavior contract (before product edits, 2026-10-31)

Retain the existing explicit Git sidebar, Tools > Git and bindable palette Commit Staged and
Amend HEAD actions, their cancellable preparation, two window-local mode drafts and native
`commit --file=-` worker. Ordinary commit remains staged-only and uses native configured author
identity. Amend requires existing HEAD, includes only the index and supports a message-only
amend without staging; native Git preserves the previous author and timestamp by default.
Add blank optional Author name/email override fields: only a complete explicitly entered pair
changes this commit's author, using exact validated native `--author` input. Blank fields never
override identity, including retained drafts reopened against a later HEAD. No global identity,
committer, signing configuration, hooks or process environment is changed.

Validate the complete subject/body and coauthor name/email inputs before launching Git. Reuse
existing Unicode and injection checks. Remove semantically identical coauthor entries from the
actual trailer block and new coauthor inputs, preserving the first spelling/order and unrelated
body, sign-offs, folded metadata and literal trailer-like prose. Use native read-only trailer
parsing without running configured trailer commands; retain native commit trailer policy.
Repository/root/symbolic HEAD/OID, project/surface freshness, worker-owned lease and save/write
protections remain unchanged. Errors keep exact draft fields and useful native diagnostics;
newer message or author edits made while Git runs survive completion. No new persisted setting.

Acceptance uses disposable local Git repositories/fixtures only: ordinary staged-only commits,
amend with and without staged changes, unborn HEAD, previous-author preservation despite a
different configured author, explicit Unicode author change and invalid/partial identities,
Unicode message/trailers, separated and folded duplicate coauthors, manual footer duplicates,
configured trailer separators/aliases/commands and unrelated-text preservation. Native hook and
signing failures must leave HEAD, index content and worktree bytes unchanged and preserve UI
drafts for retry. Existing G01/G02/G03, partial staging, source/shared/IME/save and lease tests
remain intact. Add compiled causal regressions for actual missing branches, update reachable UI
and docs, run focused/full Git/UI and strict all-target affected checks plus fmt/diff. No measured
hot path is targeted; performance gates run only if that changes. Pin final source/build/fixture/
asset/script inputs before final checks and verify before/after, record UTC/exit/raw failures and
actual final executed binary hashes, then freeze Snapshot29 delta/scope/report/readback. Root
owns independent scoped review and final workspace/native/release/performance gates. Snapshot29
was captured once at accepted f6b5322; never recapture. Canonical G04 and Task29 boxes stay open.

### G04 Task29 composition policy refinement (before runtime changes)

Read-only native experiments show `--parse` does not execute configured trailer commands and
recognizes configured aliases. Git's ordinary `--trailer` insertion normalizes unrelated footer
separators and can turn canonical coauthor input into a malformed key with a custom separator.
The controller explicitly selected read-only native footer/alias/separator recognition followed
by surgical deduplication and direct canonical `Co-authored-by: Name <email>` insertion within
the recognized footer. Native `commit --file=-` remains the sole commit engine with configured
hooks/signing/identity intact. Assisted coauthors intentionally bypass `trailer.*` insertion and
command rewriting; use manual message text or commit hooks for such policy. This supersedes
the historical native trailer insertion policy only for assisted coauthor composition.

### G04 Task29 self-review refinement (before continuation-span repair)

The first pinned candidate passed focused/full/strict/fmt/native/scope checks. Native inspection
then confirmed that whitespace-only lines before a `---` divider are separators, even when
indented; duplicate coauthor removal must retain those bytes rather than consume them as folded
value text. Preserve V1 source/scripts/logs/pin and verified-at-archive historical binary hashes,
add a compiled causal regression for exact separator/body/patch preservation, stop duplicate
continuation spans at blank lines, and rerun all final affected checks after a fresh amended
source pin. Snapshot29 remains the original once-captured baseline.

### Expanded G04 / Task29 worker evidence — independent scoped review pending

Existing staged-only ordinary commit, explicit staged/message-only amend, unborn refusal and
previous-author preservation were retained. The form now offers blank optional Author name/email
overrides; a complete explicit pair affects only this commit through native `--author`, with no
Git identity/config/committer change. Separate retained normal/amend drafts include both author
fields, and newer author edits made during a submitted commit survive completion. The existing
CommitDraft and commit_draft API remain compatible; the opt-in API is commit_draft_as/CommitAuthor.

Native read-only footer parsing recognizes configured separators and aliases without executing
trailer commands. Temporary markers map parsed items to original byte spans. Coauthors dedupe
across folded/manual footer entries and new fields, preserving first spelling/order, unrelated
body/sign-off/folded text, divider/patch text and whitespace-only separators. New coauthors use
canonical `Co-authored-by: Name <email>` lines. Assisted composition intentionally bypasses
trailer.* insertion/command rewriting; native commit hooks/signing remain active for policy.

Three initial compiled causal branches showed duplicate4vs1, configured-separator duplicate3vs1
with malformed native output, and reachable author controls3vs5. Later self-review reproduced
loss of a whitespace-only separator before `---` in a whole-message assertion, fixed by stopping
continuation spans at blank lines. All pass now. Five nonzero verification invocations remain;
two initial setup/expectation mistakes are identified separately. Superseded V1 source/scripts/
logs/artifacts have431 exact archive mappings and verified-at-archive historical binary hashes.
Snapshot29 was never recaptured. Its394 originals still match accepted f6b5322; current395-file
membership contains nine modified files and one new UI test file, with385 outside files and157
existing test/helper bodies byte-identical. Canonical G04 and all Task29 boxes remain unchecked.

The amended pre-check pin at2026-10-31T19:19:00.978731+00:00 binds374 source/build/fixture/asset
inputs and six scripts. Final focused13 Git/33 UI pass (ten commit-specific UI tests; other
composition cases are included by the filter). Final full94 Git/1387 UI pass with25 established
UI ignores and both doc-test targets clean. Strict Git/UI all-target Clippy, workspace fmt,
native parsing probes, diff and scope/source readback pass. Final combined Git5b423177ed4563a2
and UI fd79e6e0c5405c24 binary paths/hashes match both final logs; standalone Git b43 evidence is
separate. Real native Git2.56 ran disposable commit/amend/hook/signing fixtures; UI checks use
GPUI TestPlatform. No physical window, real signing-key success or release build is claimed.
No measured hot path/workload/budget/ignore changed, so no extra performance gate was warranted.
Root owns fresh independent Spec/Quality review and final whole-workspace/release/native/global
performance checks. Full report, raw UTC/exit logs, complete task diff, pins, preservation and
freeze/readback evidence are under `.superpowers/sdd/editor-expanded-features-plan/task-29-*`.

### G04 root fix round1 behavior contract

Independent review found two private composer defects: marking values can turn URL prose into
a native footer, and fallback insertion can place assisted coauthors below scissors or a
tab-delimited patch cutoff. Defaults, optional author controls, native commit hooks/signing,
prepared targets, UI drafts and public APIs remain as specified for G04. Empty assisted fields
must preserve literal URL/coauthor examples when native Git finds no footer. New canonical
coauthors must appear in the native-recognized message before every supported cutoff, leaving
each unrelated body, separator and tail byte intact under verbatim cleanup. Native recognition
must use unmodified input; span mapping must preserve its result and comment/cut boundaries.

Acceptance adds real disposable-repository regressions for URL/literal duplicate prose without
fields, URL-only body with a new visible coauthor, configured comment-prefix scissors, no-footer
default scissors and tabbed patch dividers, with exact Unicode message/tail equality and native
trailer queries. Failure must not silently rewrite message text. Existing author, duplicate,
custom separator, whitespace, hook/signing-failure and UI/source/worker coverage remains. The
root owns this localized repair, pins amended inputs before covering/final checks, preserves
the reviewed V2 archive and obtains a fresh scoped F1/F2 re-review before G04 acceptance.

G04 root round1 covering verification: five compiled causal failures reproduce both review
findings; all19 composition Git cases pass after repair, including configured real-footer
scissors with Unicode commentString. Final pinned focused19 Git/33 UI and full100 Git/1387 UI
(+25 established ignores), strict affected all-target Clippy, workspace fmt/diff and six native
prefix/scissors probes pass. Source374 and scripts5 remain pinned. Scope retains390 outside
files and169 previous test/helper bodies; the exact1275-mapping reviewed V2 archive includes
actual binaries. This remains pending fresh scoped F1/F2 re-review; G04 is unchecked.

### G04 root fix round2 behavior contract

Fresh review confirms F1/F2 repaired, but finds F3: suffix marking changes Git's exact old-style
Conflicts header, making a valid sign-off plus ignored conflict-note block fail the consistency
check. Preserve this native delimiter and its tab-prefixed notes during mapping. Ordinary and
amend forms, author defaults, canonical coauthor policy and all existing guards remain intact.
A real disposable-Git regression must prove both commit modes succeed with empty/new coauthors,
retain the exact Unicode note tail and expose the intended footer through native parsing.
Archive the reviewed round1 state before changes, obtain a compiled causal failure, apply the
localized correction, repin/check/freeze and obtain a fresh F3 scoped re-review. G04 stays open.

G04 root round2 covering verification: the new legacy-note regression produced a compiled
valid-message rejection before the localized repair; the ordinary non-ignored metadata case
already passed. All21 composition Git cases now pass, including both commit modes/field choices
and exact native-visible footer/tail assertions. Final pinned focused21 Git/33 UI and full102
Git/1387 UI(+25 established ignores), strict affected all-target Clippy, workspace fmt/diff and
native prefix probes pass. Source374/scripts5 stay pinned; scope retains390 outside files and
175 prior test/helpers. Exact1718-mapping fix1 archive includes actual executables. F3 remains
pending fresh scoped re-review; G04 and its plan boxes remain unchecked.

### Expanded Task29 / G04 acceptance

Initial scoped review approved the author/default/API/UI boundaries and required F1/F2 repairs.
Fresh round1 review confirms F1/F2 addressed; fresh round2 review confirms F3 addressed with
Spec Compliance and Task Quality passes and no new findings. Root corroborated1740 final frozen
mappings,11 actual binary/tool paths,374 pre-check source inputs and5 scripts,390 unchanged
round1 outside files and175 preserved test/helper bodies,1718 exact archived prior-round mappings
including executable bytes. Final focused21 Git/33 UI and full102 Git/1387 UI(+25 established
ignores), strict affected all-target Clippy, workspace fmt/diff and native checks pass. Every
causal failure, setup failure and superseded result remains retained. Canonical G04 and its
three plan boxes are complete; final workspace/release/native/performance gates remain pending.

## Expanded phase — T04 / Task30 behavior contract (before product edits, 2026-10-31)

Keep the existing one-tab-on-first-show, login-shell preferences and active-terminal/editor/
project/home cwd fallback. Existing `Groups<Entity<Terminal>>` remains the window-local owner;
each entity retains its own PTY, emulator, cwd, selection, search/review, errors and lifetime.
Show/hide, panel changes and tab switches preserve sessions. No live process or tab name is
persisted or resurrected. Profiles/startup-directory policy, integration and send-selection
changes belong to later tasks.

Add Rename Terminal through the existing window form, terminal header, Tools menu, command
palette and custom-bindable action. Names are trimmed Unicode text, at most 128 characters;
control characters are refused. Empty text clears the override and restores the existing cwd
label. Unnamed titles keep their current appearance. Rename pins a weak entity identity and
validates current tab membership, so cancellation or a closed/replaced/project-cleared target
cannot change another tab or retain a closed PTY. Tab switch/close clicks resolve their captured
session identity against current membership rather than a stale index. Reuse existing tab/header
geometry and process termination, with no terminal split or replacement session manager.

Acceptance: a compiled failing reachable-action regression proves the missing rename branch;
Unicode rename/clear/cancel/invalid/stale submission preserve source and neighbouring sessions.
Two controlled real PTYs prove exact independent input/output, cwd/search/error isolation and
active-only viewport resize/focus. Hide/show preserves identity and output; active/inactive/running
tab close and project/last-window shutdown leave survivors usable and no fixture process alive.
Preserve accepted T01 result-local selection/pins/retained close/28px status, T02 displayed cwd/
OSC7 link provenance and T03 exit cleanup/local Shift drag. Run full terminal/UI and warranted
affected checks, strict all-target Clippy, fmt/diff and relevant existing serial performance gates
if a measured hot path changes. Snapshot30 was captured once at accepted 56338a1; never recapture.
Pin final source/build/fixture/asset/script membership before final checks, retain raw UTC command/
exit evidence and actual executed-binary hashes, then freeze/read back the complete task delta.
Canonical T04 and all three Task30 plan boxes remain unchecked pending root's independent review.

Task30 lifecycle refinement after compiled real-PTY failures: a closed tab or accepted last-window
removal can retain its entity/controller through another UI operation, leaving the PTY alive.
Explicit close will release the existing `PtySession` through its normal Drop/termination path;
accepted native-window removal and project clearing will release all of that window's sessions.
This happens after save/recovery/unsaved-close admission, never on a pending or cancelled request.
Retained form/view references cannot prolong a closed process; other windows keep their sessions.

### Expanded Task30 / T04 worker evidence — independent scoped review pending

Reused independent PTY entities and workspace Groups. Added window-local Unicode rename/clear
through the existing header/form/Tools/palette/custom-action routes, weak target plus current
membership validation, stable-identity tab click/close, full-name tooltips and bounded labels.
Explicit tab close, accepted command/native window close, project clear and registered app quit
now release the existing PTY Drop/process-tree cleanup even with retained controller/view refs.
Output/emulator state is preserved; no RIS, replacement manager, dependency, schema, profile,
integration, send-selection, split or process-restoration change was made.

Compiled causal failures prove missing rename, retained tab/window/native/app-quit PTY leaks and
the introduced long-label overlap/inactive-close focus defects. Both latter defects were fixed
with bounded existing widgets and the established mouse-default prevention. Retained raw setup
failures distinguish pre-paint dispatch, virtual debounce timing and clean-load versus dirty-edit
test setup. Mutation checks prove the live membership guard and removed-but-retained refusal;
exact source bytes were restored. All real-PTY process checks keep their five-second bound.

V1 full47 terminal/1399 UI and focused40 passes are historical: strict Clippy then found two
redundant new String conversions. Before edits,435 exact V1 mappings archived all396 product
files,375 inputs,6 scripts, logs/artifacts and both actual executed test binary bytes. V1 pin/logs
remain unchanged. The narrow conversion removal and one additional retained-target regression
precede the explicit V2 pin (375 inputs/6 scripts,2026-10-31T20:57:00 UTC).

Current pinned V2 focused41 terminal UI and full47 terminal/1400 UI tests pass, with1 terminal
and25 UI established ignores unchanged; strict affected all-target Clippy and workspace fmt
check pass. Real independent PTYs prove literal Unicode bytes/output, cwd/search/error isolation,
active-only resize/focus, hide/show, rename/reorder, active/inactive/running close, cancelled
close, held source-save close, other-window survival and accepted last-window/app shutdown.
Accepted T01 retained/pinned selection/fixed28px status, T02 displayed cwd/OSC7 links and T03
mouse/Shift-drag/exit cleanup regressions pass unchanged. Final diff/scope/freeze/readback and
actual command/binary evidence are recorded in task-30-report.md and its artifacts. Snapshot30
remains the once-captured395-file baseline, with396 current product files,14 modified/1 added,
381 outside byte-identical files and51 retained terminal regression/helper bodies. Canonical
T04 and all three Task30 plan boxes remain unchecked; root owns fresh scoped review and final
whole-workspace/release/physical-native/global performance gates.


### Expanded T04 — accepted after independent review

T04 is complete for the expanded acceptance criteria. Rename is reachable from the terminal
header, Tools, command palette and bindable action. Names are window-local Unicode presentation,
trimmed and bounded to128 scalars; empty input restores the default label. Independent existing
PTYs retain separate output, cwd, search, errors and lifetime; hiding preserves them. Stable
entity actions and weak/membership validation reject stale tab clicks and rename submissions.
Explicit admitted tab/project/window/app cleanup now releases processes even when views remain
retained, using the existing PTY Drop path. Cancelled closes and pending saves keep sessions alive.

Root independently corroborated1261 frozen mappings,16 actual binary/tool paths,375 source and6
verification inputs pinned before final checks,30 raw commands and2 finalization records,435 exact
historical V1 archive mappings including executable bytes,381 unchanged outside files and51
retained terminal test/helper bodies. Final41 focused,47 terminal and1400 UI tests pass, with
established1/25 ignores unchanged; strict affected all-target Clippy, workspace formatting, diff
and preservation checks pass. Failed setup/causal/mutation/Clippy runs remain distinguished from
final V2 evidence. Fresh scoped review passed Spec Compliance and Task Quality with no new
Critical, Important or Minor findings. Canonical T04 and all three Task30 plan boxes are checked
only after that review. Existing measured search/editor/emulator paths were unchanged; no
matching header/lifecycle performance gate exists. Physical AppKit QA and final whole-workspace,
release and global performance checks remain pending. The selected checklist now has30/64
accepted entries. Root commits are authorized by the latest user instruction.


## Expanded phase — T05 / Task31 behavior contract (before product edits, 2026-10-31)

Extend the existing global terminal preferences with bounded named profiles (shell executable,
literal argument array, optional absolute startup directory, 0–100000 retained rows) and an
optional default profile. Absent profiles/default selection preserves the existing shell/arguments/
scrollback fields and `$SHELL` then `/bin/zsh`, `-l -i`, 10000-row defaults. Profiles are launch
snapshots: New Terminal uses the configured default; New with Profile opens the existing chooser
and creates a separate entity/PTY. Neither settings reload nor profile selection kills/restarts
an existing PTY. Use the existing tabs, direct CommandBuilder argv and child-local environment;
no new session/process engine, split, resurrection, shell integration or send-selection feature.

Validate profile names, executable/argument strings, absolute startup directories and retention
at the settings boundary. Unknown default references warn and keep normal defaults. A missing
executable produces the existing visible startup error. A missing/unusable startup directory
falls back to the existing active-terminal/file/project/home/root policy; arguments with spaces,
quotes, dollar signs and empty strings remain separate literal argv. Profile-picker dispatch
rechecks current settings; removed profiles fail visibly and never launch another profile.

Retention changes use an explicit per-session Set Scrollback action through the existing window
form and weak target/current membership checks. It updates only retention, preserves PTY/modes/
current screen/cwd and immediately discards oldest retained rows above the new cap (zero drops
history; visible screen rows remain). Increasing the limit cannot restore discarded output.
The installed vt100 0.16.2 lacks a capacity setter and does not handle CSI3J; set_scrollback only
moves the viewport. A narrow capacity extension in that actual grid owner is under scope review,
never a reset or state serialization/replay. On changed retention, invalidate obsolete search
workers/selected/retained/command snapshots and selection/link coordinates; fresh queries use the
new layout epoch. A reduction clears bounded command archives and in-flight command captures,
so archived output cannot retain trimmed ranges. Unchanged retention preserves T01 pinned views.

Acceptance: compiled causal settings/profile UI and retention regressions precede fixes. Use
controlled real/fake PTYs and disposable directories to prove literal spaced/empty argv, missing
executable, cwd fallback, independent profiles, old/default startup and zero/small limits/live
trimming including alternate screen and stale results. Preserve all accepted T01 fixed28px status/
pins, T02 displayed OSC7/provenance, T03 mouse/noRIS/Shift selection, T04 naming/stable tab IDs/
admitted close/released retained PTYs and Git/editor/LSP/save/IME behavior. Run full affected
config/terminal/UI and dependent suites, strict affected all-target Clippy, fmt/diff and exact
relevant serial performance gates if their measured path changes; never weaken budgets. Pin
actual source/build/fixture/assets and task/shared runners before final checks, keep raw commands/
cwd/UTC/exits and actual executed test binary hashes, freeze/read back and release ownership.
Snapshot31 is the once-captured accepted893cc80 baseline; never recapture it. Canonical T05 and
all three Task31 plan boxes remain unchecked until root's independent review/acceptance.


Task31 retention scope ruling (2026-10-31): root approved vendoring the exact already-locked
MIT vt1000.16.2 source because it has neither a capacity setter nor CSI3J support. Exclude it
from the ten-crate workspace; expose only its primary-grid cap/drop/clamp operation, preserving
alternate capacity0, modes/cursor/parser state and newest rows. No new dependency/version/engine
or terminal replay is permitted. The two-method local extension carries an upgrade-maintenance
cost documented with original-file hashes and crate checksum in vendor/vt100. The published
installed package supplies ten Rust modules and one library doctest, with no external test files;
all available original source/docs/license are retained. Orlo directly tests native invariants.
Explicit per-session live retention, rather than an automatic settings observer, keeps profile
changes new-session-only and preserves independent PTY configuration/lifetimes.


### Expanded Task31 / T05 worker evidence — independent scoped review pending

Named profiles/default selection, startup-directory fallback and the header/Tools/palette/bindable
New with Profile route now reuse the existing direct argv builder and separate terminal entities.
The explicit session-local Set Scrollback form updates paired native history grids in place;
unchanged limits preserve pinned output, changed limits cancel old searches and discard stale
selection/retained/command views, and reductions clear command archives/captures. A changed or
removed profile and a removed-but-retained form target are refused. Existing process identity,
noRIS modes/cursor/decoder state, directory provenance and admitted close remain intact.

Causal compiled failures cover denied profile settings, legacy-vs-profile real launch, missing
catalog routes, fixed native retention (with an explicit no-op API scaffold), and an existing but
inaccessible startup directory. Four additional compiled guard mutations independently fail for
search invalidation, live form membership, profile freshness and layout epochs; exact source and
actual executed mutation binary bytes are retained and restored before the final pin. Test setup
OsStr/reference and wrong dismiss-handler compilation errors are recorded separately from causal
proof. Initial formatting exposed dependency inheritance; matching upstream max_width78 restores
all original source bytes except the approved two methods, without disabling formatting.

Strict affected preliminary checks and the expanded real/fake-PTY cases pass before finalization.
Final actual commands, exact selected test counts, dependency/source/membership preservation,
serial retained-search gate, pin and freeze/readback are reported in task-31-report.md with raw
UTC/exit logs and executed-binary hashes. The source pin precedes final checks; no historical
snapshot or canonical entry is changed. Root owns fresh independent review, physical AppKit QA
and final whole-workspace/release/global performance acceptance. T05 and all three Task31 plan
boxes remain unchecked.


Final Task31 checks additionally pin a child-local default-shell fixture that forwards the
existing `-l -i` argv to `/bin/zsh -f`; inherited ordinary/default terminal tests therefore avoid
user startup files. Explicit integration tests keep their existing disposable child HOME/ZDOTDIR
and still exercise actual hooks. The command runner itself uses `/bin/zsh -f -c`; no process-wide
environment is changed. The wrapper, all task Python scripts and both shared runners are pinned
with complete product/docs/source membership before final checks. Snapshot31 scope reads396
baseline and415 current products,375→391 non-Markdown inputs,20 modified/19 added files and376
outside byte-identical files. The unchanged package graph and all ten workspace members are
proved separately from the approved dependency source metadata change.


### Expanded T05 — accepted after independent review

T05 is complete for the expanded criteria. Named global profiles/default selection retain literal
argv and optional absolute startup directories with normal fallback; missing executables report
startup errors. Legacy/default $SHELL then /bin/zsh, -l -i and10000 retained rows remain. Profile
changes affect new sessions. Explicit per-session Set Scrollback changes retention in place,
including zero; same-limit requests preserve pins, reductions discard obsolete search/selection/
command output, and the current screen/modes/decoder/cwd/PTY survive. Weak membership and full
profile equality reject stale forms/choices. Header, Tools, palette and bindable actions reach
these existing owners. No dependency version or package graph changed. The exact locked MIT
vt1000.16.2 source is locally maintained for two capacity methods because the existing public
API cannot trim history safely; upstream source/license/checksums and upgrade cost are documented.
Ten product workspace crates remain.

Root independently checked884 frozen mappings,29 actual executable/tool paths,415 full products,
391 source inputs and8 verification helpers pinned before final checks,396 baseline files,
376 outside bytes,112 retained regression/helper bodies,775 unchanged package versions/edges,
exact two-method upstream byte delta, four causal mutation archives with executable bytes,39
raw command records and2 finalizations. Final focused50 UI/3config/5retention and seven affected
suites2668 passes (77config742editor89project179syntax53terminal1409UI119workspace) pass with
established3/1/25 ignores unchanged, as do strict all-target Clippy, formatting, diff and scope.
The exact serial retained-search release gate passes21.414083ms against unchanged250ms. Fresh
Spec Compliance and Task Quality review approved with no Critical, Important or Minor findings.
Canonical T05 and all three Task31 boxes were checked only after review. Physical AppKit and
final whole-workspace/release/global performance gates remain pending. Selected acceptance is31/64.

### Expanded phase — T06 / Task32 behavior contract

Task32 reuses existing OSC133 parsing, bounded command/output archives, directory provenance,
previous/next review, output copy and one fixed28px status toolbar. The missing launch admission
will become `terminal.shell_integration: false` by default in global preferences. Only an explicit
true enables Orlo's launch-local zsh hooks for new ordinary/profile sessions. Existing
`TerminalSession::new/from_command` remain callable and launch without injection; an additive
explicit constructor admits integration. Settings changes never restart an existing PTY.
TerminalProfile's literal argv/cwd/retention fields and chooser freshness remain unchanged.

The supported injector remains zsh, executable basename `zsh`, with interactive `-i`, `-li` or
`-il` and only the existing login/interactive flags (`-l`, `-i`, `-li`, `-il`). Other executables,
noninteractive or custom flags use the ordinary command unchanged. Installation failure reports
the existing nonfatal integration status and still launches the plain shell. No shell startup
configuration is written; existing user startup files, ZDOTDIR changes, login argv and hooks must
keep their native behavior. Owned temporary files are released through TerminalSession close/
Drop, including an explicitly admitted T04 tab/window close with retained UI references.

Genuine supported OSC133 A/B/C/D markers are parsed regardless of Orlo injection. No-marker
terminals keep ordinary input/output, selection, search, mouse, links and copy; no prompt text
is scraped. A/B/C delimit prompt/input/output, D optionally supplies status; interrupted/missing
status is unknown, exit finalizes an executing command, malformed/partial markers cannot invent
success. Native clear-screen controls affect active captured output; RIS aborts unfinished
capture. Completed independent archives retain at most32 records/1MiB text plus bounded path
provenance; per-capture scrollback and byte bounds evict old output. Reduced live retention
clears archives/captures through accepted T05; same-capacity requests preserve T01 pins.

Existing Previous/Next Command and Copy Command Output remain available. Add an explicit Copy
Last Output catalog/action route that always copies the newest completed output, including while
reviewing an older command. Review copy continues to copy the displayed command. Both exclude
prompt/input and only write the clipboard; navigation/copy never send input or evaluate output.
No records leave clipboard/live state alone. Review uses immutable bounded command snapshots;
archive eviction cannot redirect an already displayed copy to a different record.

Acceptance: compiled causal default launch/settings/catalog regressions precede fixes; replay
success/failure/interrupt, optional/bad status, byte-split/partial/malformed controls, clear/RIS,
archive eviction and zero/small retention. Controlled real zsh child HOME/ZDOTDIR fixtures prove
startup order/argv/hooks/status/cwd, unchanged user files, unsupported fallback and owned cleanup.
Real GPUI/PTY actions prove previous/next/live, displayed and last-output copy, prompt exclusion,
no PTY input and no-marker usability. Preserve all accepted T01–T05/editor/Git/LSP/save/IME state.
Run full affected terminal/config/UI and dependent suites, focused checks, strict affected
all-target Clippy/fmt/diff and relevant exact serial release gates without weakened budgets.
Before final checks pin complete product/source/build/fixture/assets and every task/shared runner;
retain literal commands/cwd/actual environment/UTC/exits and actual Cargo Running binary hashes.
Freeze/read back snapshot32 delta, membership/outside bytes/retained tests/dependency graph and
evidence. Snapshot32 is already captured once after accepted89efb32; never recapture it.
Canonical T06 and all three Task32 boxes remain unchecked pending root's independent review.

### Expanded Task32 / T06 worker evidence — independent scoped review pending

Global `terminal.shell_integration` now defaults to false and reaches only new ordinary/profile
launches through an additive explicit backend constructor. Existing constructor signatures stay
compatible and now intentionally require opt-in for injection. Existing integration regressions
explicitly opt in with every original assertion retained. Zsh startup adapter, OSC133 parser/
bounded capture/provenance, fixed28px rendering, T01 pins, T03 mouse and T04/T05 lifetime/profile/
retention owners are preserved. Genuine marker streams still work without injection. The
additive bindable Tools/palette Copy Last Output reads the newest completed record while existing
review copy keeps its immutable displayed output, including after live archive eviction.

Compiled causal REDs cover automatic default injection, the absent explicit/default-off setting,
missing Copy Last Output catalog route and an enabled new-profile launch that did not receive
integration. Final behavior verification covers exact PTY input isolation/clipboard/no evaluation,
no-marker operation, success/failure/interrupt/optional status, byte-split/partial/malformed markers,
clear/RIS, UTF8 byte caps/eviction and retained review. Controlled real zsh children prove startup
order and login flags, user hooks, changed/absent ZDOTDIR, status0/1/130 and unchanged fixture
configuration files; normal/failed launch and admitted retained-tab close remove owned resources.

Intermediate test-setup failures remain recorded: an argv Vec/reference comparison did not
compile, and an overly exact new ZDOTDIR-output assertion encountered native zsh prompt-spacing
rows. Neither is claimed as product causal proof. Corrected pre-pin verification passes62terminal
tests (one existing performance ignore) and55terminal UI tests. Snapshot32 still has415products/
391non-Markdown inputs;16modified files,399outside byte-identical,132retained regression/helper
bodies with only three necessary explicit-integration call migrations. Cargo manifests/lock,
775packages/edges, ten workspace members and the entire approved vt100 vendor are unchanged.

Complete product/source/build/fixtures/assets/docs and all task/shared runners are pinned before
final covering checks. Literal commands, cwd, relevant actual child environment, UTC/exit results
and actual Cargo Running binary paths/hashes are retained in task-32 logs/journal. Final affected
config/terminal/UI/dependent suites, focused/strict/fmt/diff/scope and the unchanged exact serial
terminal release gate, self-review, freeze/readback and native/global limits are reported in
task-32-report.md. Root owns independent review and final physical AppKit/workspace/release/global
performance acceptance; canonical T06 and all three Task32 plan boxes remain unchecked.


### Expanded T06 — accepted after independent review

T06 is complete for the expanded criteria. Global terminal.shell_integration is explicitly opt-in,
false by default, and used only by new ordinary/profile launches. Existing constructor signatures
remain default-off, with an additive explicit constructor. Existing eligible interactive zsh hooks,
OSC133 parsing, bounded output/status archives, previous/next/live navigation and retained copy are
reused. Genuine markers still work when injection is off, and unmarked terminals work normally.
Copy Last Output is separately reachable through Tools/palette/binding and copies newest completed
output without navigation, PTY input, evaluation, prompts or command input. Startup files, hooks,
argv, ZDOTDIR and owned-resource cleanup are preserved; no user shell configuration is written.

Root independently corroborated874 frozen hashes,30 actual binary/tool paths,415 products/docs,
391 source inputs and7 runners pinned before all checks,27 raw commands and2 finalizations,
399 unchanged outside files,132 retained regression/helper bodies and3 necessary explicit-true
fixture migrations with assertions preserved. All775 package versions/edges,ten workspace crates,
vendor/parser/capture/hook/render/Profile bodies remain unchanged. Final62terminal/4config/55UI
focused and2683 affected-suite passes (78config742editor89project179syntax62terminal1414UI119workspace)
pass with established3/1/25 ignores unchanged. Strict all-target Clippy, formatting, diff and scope
pass; exact serial retained-search gate passes13.095125ms against unchanged250ms. Four compiled
causal REDs, two setup failures and an early zero-terminal-test filter remain honestly distinguished.
Fresh scoped Spec Compliance and Task Quality review approved with no Critical/Important/Minor
findings; only then canonical T06 and three Task32 steps were checked. Physical AppKit and final
whole-workspace/release/global performance checks remain pending. Selected acceptance is32/64.


## Expanded phase — T07 / Task33 behavior contract (before product edits, 2026-10-31)

Retain the existing Tools/palette/custom-bindable SendSelectionToTerminal and
ExecuteSelectionInTerminal action identities, labeling the latter Send and Run. Both read the
active source view without editing it, including read-only/shared sources. Capture only nonempty
physical ranges, sort a copy into document order, extract exact rope text and join with one LF.
Empty carets and virtual-only ranges contribute nothing; none means a status message, no source
padding, whole-document fallback, terminal creation or input. Keep every source selection,
primary, goal, rectangular/virtual metadata, selection generation, Undo/Redo, save point and dirty
state. Pending composition in any shared source view and an unmapped shared revision refuse
before focus/launch/write, without committing composition or catching up implicitly.

Send uses the existing terminal paste protocol, including line-ending normalization and escape
filtering. It adds no newline. Multiline input also follows ordinary unbracketed paste behavior;
embedded newlines can therefore execute in a target without bracketed paste. Send and Run adds
exactly one CR execution byte after any closing bracketed-paste frame, even when the selected
text already ends with a newline. Encode and admit that as one bounded existing PTY payload.
Do not split the execution newline into another queue operation or spawn a shell-filter process.

Target only the chosen active session, show it if hidden, and create the first session through
the existing T05 profile/cwd/argv and T06 default-off integration constructor when absent. A dead,
closed, oversized or busy PTY reports the existing clear error and is never silently replaced.
Retained command/search review and terminal selection/scroll clear only after successful input
admission; refused input leaves their snapshot intact. Existing input APIs remain compatible;
any returning admission API is additive. No dependency, setting, hook, capture/parser, render or
vendor change is planned. Preserve T01 fixed28px/pins, T02 displayed OSC7, T03 Shift/mouse/noRIS,
T04 stable session IDs/admitted cleanup and T05 paired live retention/same-capacity no-op.

Acceptance will use controlled real raw PTYs to assert exact Unicode/multiline/multiselection
bytes for bracketed and unbracketed Send/Run, document ordering, omitted empty/virtual ranges,
trailing-newline Run behavior and no extra Send byte. Cover hidden/absent/two chosen sessions,
exited/closed and oversized refusal, retained review, read-only shared source/Undo/Redo/savepoint
preservation and existing IME/revision regressions. Add compiled causal REDs for missing branches,
then the smallest complete implementation and user docs. Final affected suites and strict
all-target checks, fmt/diff/scope use pinned actual product/source/build/fixture/asset/task/shared
runner inputs; retain literal commands/cwd/effective child env/UTC/exits and actual Cargo Running
executable hashes. Snapshot33 already exists once after accepted b2fd881 and must not be recaptured.
Canonical T07 and its three plan steps remain unchecked pending root's independent scoped review.


### Expanded Task33 / T07 worker implementation — independent scoped review pending

The existing Send Selection and ExecuteSelectionInTerminal identities now present explicit
Send Selection / Send and Run labels in Tools and the palette. Physical ranges are copied,
sorted into document order and joined without empty-caret/virtual-only separators or source
changes. Plain Send follows the existing paste protocol for multiline input and adds nothing;
Run adds one CR after any bracketed-paste frame, even after a selected trailing newline. One
bounded PTY payload preserves ordinary encoding and input-capacity behavior. Successful queue
admission alone clears terminal scroll/selection and UI write-path retained/search/command
review; dead/closed/busy/oversized/empty/stale/IME refusals preserve review and chosen identity.
Existing profile defaults, literal argv/cwd, default-off integration and all earlier owners remain.

Compiled causal regressions reproduce unbracketed multiline refusal, incorrect physical range
order/virtual separators, virtual-only PTY creation, failed-write review/selection clearing and
missing explicit Run labeling. A frame-normalized interim ordering PASS and setup import/panel/
resize failures are retained separately and are not claimed as causal proof. Real raw PTYs verify
exact Unicode/multiline/multiselection bytes and newline/framing behavior, selected trailing
newlines under both protocols, hidden/absent/two chosen sessions, dead/closed/busy/oversize and
shared IME/revision refusals. Read-only shared selections, primary/goals/virtual/rectangular state,
version/save point/dirty identity and still-usable source Undo/Redo are checked. Pre-pin focused
checks pass64 terminal (+1 established release ignore) and62 terminal UI tests.

Final source pin, actual commands/effective child environments/UTC/exits, final affected suites,
strict checks, exact serial terminal release gate, snapshot membership/outside-byte/retained
regression proof and freeze/readback are recorded in task-33-report.md and its raw task-local
artifacts. Snapshot33 remains the original once-captured accepted b2fd881 baseline. No source or
verification input may change silently after the final pin. Canonical T07 and all three Task33
plan boxes remain unchecked; root owns independent scoped review and final physical AppKit,
whole-workspace/app/release/global performance gates.


### Expanded T07 — accepted after independent review

T07 is complete for expanded acceptance. Stable existing Send/Run action identities expose
explicit labels, copy physical nonempty selections in document order and LF-join without changing
source/selection/history. Empty/virtual-only selections do not launch/write; shared IME/revision
guards and read-only source support remain. Existing paste rules handle Unicode/multiline/CRLF,
bracketed framing and control validation. Plain Send appends nothing; Run appends one CR after
framing even after selected trailing newline. The complete1MiB encoded payload is queued once.
Unbracketed embedded newlines can execute according to the target paste protocol, as documented.
Chosen sessions are reused/shown or created through existing defaults; dead/busy/oversized input
refuses without replacing PTYs or clearing retained review. Backend/UI state clears only after
successful queue admission. No source edit, separate shell filter, new queue or engine was added.

Root independently corroborated874 frozen mappings,29 actual binary/tool paths,416 product/docs,
392 source inputs and8 verification helpers pinned before all checks,25 raw commands and2
finalizations,405 outside files,148 retained helper/test bodies and the one necessary multiline
assertion migration. Graph775/ten members/vendor/key-paste/render/T02–T06 UI owners are unchanged.
Final2692 affected passes (78config742editor89project179syntax64terminal1421UI119workspace),
focused64terminal/4config/62UI, strict all-target Clippy, formatting, diff and scope all pass,
with existing3/1/25 ignores unchanged. Exact serial retained-search gate passes12.9385ms against
unchanged250ms. Seven causal assertion occurrences across six tests, fixture failures and the
noncausal frame-normalized ordering pass remain distinguished in evidence. Fresh scoped Spec
Compliance and Task Quality review approved with no defects; only then canonical T07 and three
Task33 steps were checked. Physical AppKit and final whole-workspace/release/global performance
gates remain pending. Selected acceptance is33/64; all seven terminal audit items are accepted.


## Expanded phase — N03 / Task34 behavior contract (before product edits, 2026-10-31)

Inspection found no N03 operations in the existing Codec/Convert/catalog/pipeline interfaces.
Reuse `regex_syntax::escape` (the installed syntax used by Orlo's actual Pattern engines),
`serde_json` string serialization/deserialization, and the existing atomic range edit API.
No dependency, preference, default shortcut, persistence schema or interpreter is needed.
Add `regex_literal` (encode only) and `json_string` Codec names; Regex Literal decode is an
explicit unsupported-direction error. Decode Text Escapes is an explicit recordable editor
action and pipeline operation, avoiding an invented reverse text-escape encoder. Native
Edit > Convert and the shared palette expose exactly Escape Regex, Quote JSON String,
Unquote JSON String and Decode Text Escapes. Existing Convert JSON arguments remain bindable.

Quote JSON String includes surrounding double quotes; Unquote accepts exactly one JSON string
(with ordinary JSON surrounding whitespace), rejects non-string JSON/trailing tokens/invalid
escapes, and uses serde_json rather than coercion. Decode Text Escapes accepts literal Unicode
and controls plus exactly `\\`, `\"`, `\'`, `\n`, `\r`, `\t`, `\b`, `\f`, `\xHH`, `\uHHHH`
and `\UHHHHHHHH`. Hex digits are ASCII; xHH is U+00HH, not UTF-8 bytes. A u high surrogate
requires an immediately adjacent u low-surrogate escape; isolated/reversed surrogates,
unknown/trailing/incomplete escapes and U non-scalars fail. Decode runs once, without eval.

Every N03 editor operation catches up shared source first, requires nonempty physical selection
ranges and ignores empty/pure-virtual targets without whole-document fallback or padding.
Precompute all replacements through immutable current source before entering a single existing
atomic undoable range edit. Invalid later targets leave text, complete selection state,
selection generation/history, document version/dirty/save identity and Undo/Redo unchanged.
No-op conversions also create no edit. Preserve backward/forward, primary and rectangular/virtual
origins through existing range mapping. Read-only file overwrite protection remains distinct
from immutable viewers (normal source edits stay allowed); immutable viewers and Follow refuse.
Successful explicit transforms keep the existing editor convention of committing marked text
and mapping shared views/snippets. Failed preflight must retain marked text. Work is synchronous,
so there is no delayed result: shared changes are caught up before reading and no await occurs
between preflight and apply. N01 URI and N02 HTML behavior remains separate and unchanged.

Acceptance includes compiled causal RED for missing catalog/Codec/pipeline branches; semantic
literal regex matching using actual Orlo Pattern (both fast/fancy where supported), JSON
quotes/newlines/emoji roundtrip/string-only failures, Unicode/control/malformed text-escape
tables, mixed multi-target atomic failures and one-step Undo/Redo. GPUI verifies menu/palette,
custom binding/macro reconstruction, physical-only targets, backward/virtual origins, shared
catch-up, read-only/Follow/viewer and IME behavior. Full affected editor/UI/dependent suites,
strict all-target affected Clippy, fmt/diff/scope and exact snapshot/membership/graph preservation
are required. No measured hot input/render/search owner changes are planned, so no unrelated
terminal performance gate is claimed. Canonical N03 and the three Task34 plan steps remain
unchecked for root-owned independent review and final workspace/native/release gates.

### Expanded Task34 / N03 worker implementation — independent scoped review pending

The existing conversion and atomic range-edit owners now expose the four named literal
operations. Regex Literal and JSON String reuse installed regex-syntax/serde_json; the strict
text-escape decoder is shared by its recordable unit action and pipeline operation. Unsupported
Regex Literal decode is absent from menus and rejected when a pipeline loads. No dependency,
settings, graph, terminal, Git, LSP, source-save, input-method or measured hot-path owner changed.

Five compiled editor regressions and the shared palette regression failed on the genuinely
missing named branches before implementation. Current pre-pin focused evidence passes five
editor and ten UI tests, including actual regex-engine semantics, Unicode/control/malformed
tables, mixed physical/backward/rectangular/virtual targets, exact failure generation/save/marked
state and preserved redo, shared-view stale catch-up, read-only/Follow/viewer behavior, native
menu actions, custom binding/macro reconstruction and pipeline UI reachability. New code keeps
the existing synchronous explicit-edit composition and existing pipeline target/checkpoint
conventions. URI/HTML implementation and accepted tests remain intact. Initial test-harness
compile mistakes and strict test-only single-range lint findings are retained honestly in
the task-local logs; no ignores or warning suppressions were added.

Task-local final commands, exact source/script pin, snapshot membership/preservation, actual
executed binary paths/hashes, all failures and readback are recorded in
`.superpowers/sdd/editor-expanded-features-plan/task-34-report.md` and adjacent artifacts.
Canonical N03 and all three Task34 plan boxes remain unchecked pending independent scoped
review. Physical native interaction and final workspace/release/global performance gates
remain root-owned; the worker's native evidence is the actual GPUI menu/action harness.

Additional pre-pin coverage verifies snippet mirrors and subsequent field navigation through
the same literal range edit. Its first fixture expected only the primary occurrence; inspection
confirmed existing snippet selections include mirrors, so the regression now asserts both the
unchanged pre-edit mirror selections and both transformed selections. The corrected snippet
case passes; strict affected all-target Clippy also passes before final pin. This was a test
expectation correction, not a product behavior change.


### Expanded N03 — accepted after independent review

N03 is complete. Escape Regex uses existing regex-syntax literal escaping and has no reverse
operation. Quote/Unquote JSON String use existing serde_json string-only semantics without
coercion. Decode Text Escapes is one shared strict scalar decoder/action/pipeline step for
backslash/quotes, n/r/t/b/f, xHH=U+00HH, adjacent valid uHHHH surrogate pairs and UHHHHHHHH scalars;
unknown, incomplete, isolated/reversed or out-of-range forms fail. URI/HTML remain separate.
All requested menu/palette/binding/macro/pipeline routes and dialect documentation are complete.
Immutable physical-target preflight computes every result before history or edit; empty/no-change
is a no-op and failure preserves source, history, selections and composition. One existing range
batch preserves shared/backward/primary/rectangular/virtual/snippet behavior. Existing pipeline
scope, document newline normalization, file-read-only versus immutable-viewer/Follow admission
and successful IME commit conventions remain unchanged. No dependency or engine was added.

Root corroborated875 frozen mappings,27 actual binary/tool paths,416 products/docs392 source7
runners pinned before checks,25 raw commands/two finalizations,406 outside files,3337 retained
regression/helper bodies, exact inherited attributes and one additive four-menu-label expectation.
Final2708 affected tests pass (78config747editor89project179syntax64terminal1432UI119workspace),
with29 established ignores unchanged; focused N03 5editor/11UI, URI5/5 and HTML5/6 pass. Strict
all-target Clippy across affected editor/UI/workspace/syntax plus app compile, formatting, diff
and preservation pass. Existing775-package graph/ten workspace members/vendor/URI/HTML/terminal/
Git/LSP/save/IME owners remain unchanged. No measured hot owner changed; no unrelated performance
gate is claimed. Fresh scoped Spec Compliance and Task Quality review approved with no findings;
only then canonical N03 and three Task34 steps were checked. Physical AppKit and final workspace/
release/global performance remain pending. Selected acceptance is34/64.


## Expanded phase — N05 / Task35 behavior contract (before product edits, 2026-10-31)

Inspection: existing typed Motion supports words/subwords and display geometry; paragraph
expansion already works and remains byte-identical. N05's four motions and Select/Delete Sentence
are missing. Extend Motion with paragraph_previous/paragraph_next and sentence_previous/
sentence_next; reuse Move's select flag, existing selection history, edit batch, shared source
mapping and recordable action catalog. Add Edit > Prose and matching palette rows, with no new
default keys, preference, persistent state, service, dependency or linguistic parser.

Both directions use one ordered boundary set: 0, paragraph/sentence starts, and EOF. Previous
chooses the greatest strictly smaller boundary, Next the least strictly greater; endpoints clamp.
From an interior position they choose the neighboring boundary. Paragraph starts are physical
line starts of nonblank runs, including indentation, after any whitespace-only blank-line run.
Leading blanks are crossed in one step; blank-line runs introduce no intermediate paragraphs.
Sentence punctuation is .?! followed by any stacked closing quotes/brackets and then whitespace
or EOF. Closing suffixes include ASCII quotes and )]}, Unicode right quotes/angle quotes and
paired closing brackets. Abbreviations have no exception: Dr. followed by a space splits.
Sentence starts follow all separator whitespace; leading whitespace belongs to the first
sentence and trailing whitespace to the last. A final unpunctuated remainder is one sentence.
Blank lines alone do not terminate an unpunctuated sentence. Endpoints always cover whole
extended graphemes, including combining marks, emoji ZWJ sequences and CRLF pairs.

Select Sentence expands each physical selection to every sentence interval it touches, keeping
its direction, primary and rectangular origin. A caret exactly at a start selects that sentence;
a caret in a separator selects the preceding sentence; EOF selects the last sentence. A selection
ending exactly at a sentence start excludes that following sentence. Repetition is idempotent.
Delete Sentence uses those same intervals, including following whitespace, and merges overlapping
requests through the existing edit batch into one undo step. Empty documents and pure virtual
ranges delete nothing; virtual targets add no padding. Physical ranges with virtual endpoints
use only their physical part. Undo restores original selections/primary/virtual/rect origins;
Redo reapplies. Motions clear head virtual space/goals, preserve extension anchors and rectangular
origin, normalize duplicates, and never edit source or source undo history. Fold targets use
existing directional Rows::skip; wrap/filler geometry never defines prose boundaries.

Synchronous commands read the current shared revision: existing selection catch-up applies
before motion/select and existing change admission before deletion. No asynchronous result or
new cache can go stale. Selection/motion work in immutable Viewer and Follow views. Deletion
uses edit_text: immutable/Follow refuse before clearing marked text or touching history;
filesystem-read-only working copies remain editable. Admitted operations keep existing explicit
selection/edit IME-commit behavior, snippets and overtype conventions.

Acceptance: compiled causal REDs before missing branches, full-prose all positions in both
directions, exact boundaries/leading and trailing whitespace/blank runs/EOF/no final punctuation,
abbreviations/stacked ASCII and Unicode quotes/brackets/combining/ZWJ/CRLF; multicarets, reversed
and extended anchors, goals/virtual/rect/primary, duplicate overlap, shared revisions and one-step
Undo/Redo. GPUI verifies all ten shared menu/palette commands, custom keymap, macro recording and
reconstruction/replay, locks, IME, wrapped/folded motion and shared same-update catch-up. Full
affected suites and strict all-target checks plus exact serial release bench_typing and the find-panel keystroke gate
retain every existing workload and budget. Pin all product/source/build/fixture/asset/runner
inputs before final checks, preserve every prior regression/helper and inherited attributes,
record all commands/failures/actual Running binaries, then scope/freeze/readback. Snapshot35 is
already captured once; canonical N05 and its three plan steps remain unchecked for root review.

Task35 controller approved the exact interval/boundary contract above. Clarification: an ordinary
zero-length physical caret selects/deletes its containing sentence; only an empty document or
purely virtual deletion target is a no-op. Previous motion from a virtual head first drops its
gap at the physical line end, matching existing word/subword motion. This was verified by a
new compiled failing regression before extending that guard to the prose Previous variants.

### Expanded Task35 / N05 worker implementation — independent scoped review pending

The complete source/UI slice now uses one boundary table per operation, with no whole-document
string clone per caret. Both motions and extending variants reuse typed Motion/Move and the
ordinary row/fold path. Select/Delete Sentence share exactly the same half-open interval lookup;
delete reuses edit_with's duplicate/overlap union and original-selection undo snapshot. Existing
paragraph expansion, codecs (including accepted N03 atomic preflight), input/save/shared history
engines, all seven terminal features, Git/LSP services and dependency graph remain unchanged.
All ten requested commands are in shared Edit > Prose/palette catalogs and the ordinary custom
binding/macro catalog, without changing any default shortcut.

Pre-pin verification passes 12 editor prose regressions and seven GPUI prose regressions,
including real installed custom keys, recording/reconstruction/replay of all ten actions, actual
native-menu dispatch, same-update shared catch-up, wrapped/folded source movement, admitted IME
commit and immutable/Follow refusal before composition mutation. The expanded Unicode fixture
initially miscounted scalar offsets; the failed assertion and corrected fixture are retained.
The first macro replay test called a private helper; its setup-only compile failure is retained,
and the test now uses existing ReplayMacro. A further compiled RED caught Previous-from-virtual
skipping the physical line end; the narrow existing virtual-motion guard now covers prose too.
No old test/helper, ignore, lint attribute, performance workload or budget was weakened.

Final all-product/source/build/fixture/asset/runner pin, full affected suites, strict affected
all-target Clippy (including dependent app compile), focused retained regressions, formatting,
diff/scope and the exact serial release typing/find-input gates follow. Exact commands/results,
failures, UTC/environment/cwd and actual Cargo Running binary variants are recorded adjacent to
`.superpowers/sdd/editor-expanded-features-plan/task-35-report.md`. Canonical N05 and the three
Task35 plan steps remain unchecked. Physical AppKit and final workspace/release/global performance
checks remain root-owned; worker native evidence uses the actual GPUI menu/action harness.

The first exact preservation pass also caught an accidental old subword fixture change caused
by a broad numeric replacement while fixing a new Unicode expectation. The entire pre-existing
motion-test prefix was restored verbatim from Snapshot35, retaining every new prose test.
Corrected preservation passes: 416 products/392 source inputs, 14 modified and no membership
change, 402 outside files byte-identical, all 3354 previous regression/helper bodies and exact
inherited attributes retained, with unchanged default-key and paragraph-expansion bodies.
That failed proof remains in the pre-pin evidence; no final pin existed during this correction.


### Expanded N05 — accepted after independent review

N05 is complete. Previous/Next Paragraph and Sentence and selection variants use one source-
ordered start table per explicit operation, with strict symmetric steps and0/EOF clamps. Blank
physical lines separate paragraphs, retaining indentation. Sentences use the documented .?!,
closing quote/bracket and following whitespace/end heuristic, without abbreviation exceptions
or blank-line-only splits. Sentence intervals own separator whitespace; EOF chooses the final
interval and final unpunctuated text is retained. Select/Delete Sentence share half-open touched
intervals; selection keeps direction/primary/rectangular origin and deletion unions overlaps into
one existing edit/Undo batch. Real carets work; empty documents and pure virtual deletion are no-
ops. Graphemes, combining tails, ZWJ/CRLF, virtual first-step-to-EOL, shared revisions, Viewer/Follow,
IME, snippets/overtype, folds and old paragraph expansion are preserved. All ten shared menu,
palette, binding and macro routes work; no default key, dependency or service was added.

Root independently corroborated880 frozen mappings,28 actual binary/tool paths,416 products/docs,
392 source inputs and7 runners pinned before checks,29 raw commands/two finalizations,402 outside
files,3354 exact prior helpers and four whole old test-file prefixes. Final2727 affected tests,
focused12editor/7UI and retained subword/syntax-selection/N03/URI/HTML pass with29 inherited ignores
unchanged. Strict affected all-target Clippy including app compile, fmt/diff/scope pass. Both exact
serial release gates pass unchanged budgets: typing0.003/0.095/1.625/15.960ms against1/3/20/30ms;
find-input37.057ms against300ms. The preserver caught and restored an accidental pre-pin old fixture
edit; no prior assertions were weakened. An environmental ENOSPC UI compile and missing-journal
append are preserved exactly; root removed only regenerable incremental caches after safety checks,
recovered the actual failed record and retried on unchanged V1 source. All recovery metadata is
retained. A root-only benchmark-path metadata lookup was corrected before final corroboration;
no workload/test/source changed. Fresh scoped review approved Spec Compliance and Task Quality
with no findings; only then canonical N05 and three Task35 steps were checked. Physical AppKit
and final whole-workspace/release/global performance gates remain pending. Acceptance is35/64.


### Expanded Task 36 / N06 implementation contract (2026-10-31)

Add distinct recordable `editor::TransposeWords`, shared Edit command entry/native Text menu,
palette/custom binding/macro reconstruction and real ReplayMacro route; no default binding.
Preserve existing Transpose (graphemes/selection rotation), Word/Subword and N05 prose behavior.
Use existing alphanumeric/underscore word classification and `word_range_at` grapheme extension;
combining tails remain attached, emoji/punctuation/whitespace between words stay separators.
Prepare one ordered source-word table for the explicit operation, without whole-document text
clones per caret. At word start or inside a word choose that word and the following word; at
word end or anywhere in punctuation/whitespace choose the left and right words. Exact EOF
chooses the final two words; trailing separators before EOF have no right partner and do nothing.
Leading separators have no left partner and do nothing. Nonempty selections use their head;
virtual heads do nothing, including pure virtual selections; no padding or document fallback.
Missing partners and identical-word swaps are no-ops. Existing normalized duplicate selections
are one caret, but distinct carets sharing any word or overlapping full swap spans reject the
entire operation before history/IME/text/selection mutation, even when another target is valid.
Precompute every source range and replacement string first; apply two word-only replacements
per admitted pair through ONE existing `replace_ranges` batch. Never rewrite separators.
Keep original selection endpoints through normal minimal-change range mapping: shared prefixes/
suffixes and insertion boundaries use the existing mapper. Preserve primary/orientation/
rectangular origin and unaffected virtual gaps, clear goals only on admitted edit as usual.
One Undo restores original text and complete original selections, one Redo repeats the batch.
UI catches up shared revisions before preparation; Viewer/Follow refuses before composition
or history changes; filesystem-read-only source buffers retain the existing editable policy.
Accepted edit commits IME and uses existing shared/snippet/overtype paths; rejected/no-op
operations preserve IME, selection generation, saved identity and Undo/Redo.
Regression acceptance includes alpha, beta -> beta, alpha; caret boundaries/EOF/trailing/none;
Unicode combining/ZWJ emoji/tab/newline/CRLF separators; disjoint and shared-word multicarets;
atomic rejection, original history/primary/reverse/virtual origins, shared catchup/Undo, snippet
mirrors/overtype; native menu/palette/default-key preservation/custom binding/actual macro replay.
All canonical N06 and plan boxes remain unchecked until controller independent review. Root
retains final workspace/native/release/global performance gates. No other feature is included.


N06 pre-final checkpoint: immutable `transpose_word_edits` builds source word ranges with the
existing classifier/`word_range_at`; `transpose_words` and the UI consume its prepared batch.
All spans are checked before replacement texts are admitted; two word replacements leave the
separator outside every source edit. UI preflights before `edit_text`. The new shared command
entry reaches native Text menu/palette; catalog declaration drives recording/reconstruction.
The compiled missing-action regression failed at runtime before implementation; five model and
six UI regressions passed during development, including actual public ReplayMacro dispatch.
Development failures and fixes remain in Task36 command logs; final pinned checks and full
results are recorded in the task report, without checking the canonical/plan boxes here.

Measured-path decision: inspected `benches/editing/mod.rs::bench_typing` (direct model
`type_text`/`backspace`/`undo`/`redo`) and the 10M-match UI gate (`type_in` -> `Editor::edit` ->
model `insert`, followed by existing presentation). These owners, cursor/word/selection,
range-edit/history/native input/IME and both exact benchmark bodies are byte-identical to
Snapshot36. Only the explicit new cold action/preparation and its catalog/menu consumers were
added. No measured shared input/motion path changed, so there is no new task-local hot gate;
root retains the unchanged serial release/global performance suite. Scope preservation also
checks all old regression/helper bodies and whole prefixes of the four appended test files.


N06 V3 pre-final checkpoint: self-review added a compiled causal byte-preservation regression
for combining marks attached to whitespace/punctuation separators, with combining marks inside
both swapped words. It failed semantically against V2; the narrow new consumer starts each word
at its first classified scalar's grapheme boundary while retaining `word_range_at`'s end. Shared
word/classifier/grapheme helpers remain unchanged. The regression checks start/inside/separator/
EOF carets, exact edit spans, valid grapheme endpoints, one Undo and Redo. Exact V1/V2 archives
and causal RED source/executable bytes are retained in task evidence before the corresponding
changes. An unchanged Git fixture had one local socket WouldBlock in V2's ordinary full run;
the exact focused retry and full nine-library serial retry passed. No Git assertion or source
was changed. For V3, controller-approved final verification runs complete eight non-Git library
suites with ordinary scheduling, then the complete Git suite serially in a separate required
log, all under the same V3 source/script pin. This scheduling is verification, not a Git fix.
Root's guarded regenerable incremental-cache maintenance occurred only after confirmed NOJOBS;
source/evidence/deps/actual binaries were retained and its storage record is included in freeze.


N06 V4 pre-final checkpoint: the controller authorized two fixture-only setup corrections after
archiving exact V3 source/scripts/logs and all executed final binary bytes. The T07 busy child
now signals PRIMING, waits for one byte of a MAX_INPUT priming write, signals READY, then never
reads again before the unchanged queue-fill/refusal assertions. The Git loopback fixture now
explicitly puts the accepted socket into blocking mode before the identical one-second timeout.
A disposable raw-accept macOS probe demonstrated inherited O_NONBLOCK and immediate OS35 despite
that timeout; clearing the flag reads the actual payload. Both root proof JSONs, the probe,
both original failing broad runs, all retry logs and exact earlier archives remain evidence.
Every prior assertion/deadline/timeout is preserved. Preservation admits precisely these two
setup-only helper migrations. The existing inventory proves 3,372 helper bodies exactly unchanged
plus one counted T07 migration; the Git fixture is outside that legacy inventory and is proved
separately as the entire old file plus one setup line. The controller's initial 3,371 estimate
was corrected before pinning; no inventory was altered to meet that estimate. All four old
test-file prefixes, default keys and shared runtime owners remain unchanged.

V4 uses explicit CARGO_INCREMENTAL=0 on every Cargo command and records the effective child
environment; this limits build-cache growth without changing test workloads or assertions.
Complete eight non-Git library suites run with ordinary scheduling, followed by the complete
Git suite serially, plus exact focused checks of both corrected fixtures and the twelve N06
checks. Final phases use immutable `-final-v4-attempt-N` log names. A phase's latest attempt
must pass; all attempts must retain identical command/environment, and any further attempt
requires an explicit controller ruling tied to the previous raw-log hash. The runner never
automatically retries, relocates or overwrites a log. All failures stay in the final manifest
and command journal. Unexpected failure stops work for diagnosis/controller ruling. Strict
all-target affected checks include app; final formatting/diff/preservation/freeze/readback and
full report are required. Canonical N06 and its three plan boxes remain unchecked for review.

### N06 root fix round 1 — separator grapheme ownership

A word candidate that lies inside a preceding separator grapheme cannot start a word, even
when the scalar has Unicode's Alphabetic property. Skip the complete existing cluster before
considering another candidate; leave the shared word classifier and grapheme helpers unchanged.
The focused byte/span/Undo regression covers U+0345 and spacing marks attached to spaces,
punctuation and emoji, plus the original combining/ZWJ/control cases. Standalone Alphabetic
marks after control boundaries retain the existing word classification. The literal review
case reproduced before the fix and passes afterward; full V6 checks and scoped re-review remain.

### N06 root fix round 2 — legitimate word prefixes

A grapheme can begin with Unicode Prepend scalars before its first letter or number. Preserve
that complete word rather than skipping its base. The new consumer reuses existing word
continuation classification and the same grapheme segmenter to distinguish a joining prefix
from a true separator in the cluster prefix. Separator-owned marks still stay outside edits.
A separate regression covers the exact U+0600 review case, repeated/numeric prefixes, prefix
plus marks, and prefixes combined with actual space/punctuation/emoji/control separators,
including exact spans and one-step Undo/Redo. All earlier N06 regressions remain unchanged.
The review case reproduced before the fix and passes afterward; full V7 checks and scoped
re-review remain pending. No shared word/grapheme helper or dependency changed.

### N06 accepted — expanded feature 36 of 64

Transpose Words is available in Edit > Text, the command palette, custom bindings and recorded
macros. It keeps existing word/continuation classification, complete graphemes and exact
separator bytes, including Alphabetic combining marks and legitimate Prepend word prefixes.
Disjoint carets share one normal range-edit batch and Undo snapshot; overlapping word/swap
spans reject atomically. Missing partners, unchanged swaps and virtual heads are no-ops.
Selection endpoints follow ordinary minimal-change mapping. No default shortcut, setting,
dependency or persistent state was added; existing Transpose remains unchanged.

Final V7 verification passed: 2,851 tests across all nine libraries with 29 existing ignored;
focused seven model/six UI regressions and the exact T07/Git fixtures; retained motion51,
N03codecs16 and transpose16. Commands used locked dependencies and the recorded shared Cargo
target with incremental caching disabled. Strict all-target Clippy for editor/UI/workspace/
syntax/Git/app, formatting, diff and preservation passed. Root independently corroborated
5,650 frozen mappings, 29 actual executable/tool paths, pre-check inputs and raw command/UTC/
exit records. A fresh scoped review and two scoped fix reviews closed both Unicode findings
with compiled failing regressions and no new findings. Both fixture setup corrections retained
every old assertion, deadline and timeout. Historical failed attempts and the qualified build
artifact disappearance/reconstruction are retained; final evidence uses fresh verified source.
Physical macOS integration, final workspace/release and global serial performance remain for
the final combined verification after all selected features.

### N07 implementation contract — existing surrounds

Change Surround and Delete Surround are explicit, recordable editor actions, reached from
Edit > Text and the palette through existing forms, or directly from custom bindings/macros.
Targets are `(`, `[`, `{`, `'` and `"` (closing bracket aliases accepted); Change additionally
requires a replacement target. Count defaults to 1 and must be a positive whole number. It
chooses the nth enclosing pair of the requested target, innermost first, rather than removing
n pairs. No default shortcut or persistent setting is added.

A physical caret on either delimiter, immediately inside it, or immediately after its closer
can target that pair. A nonempty selection must fit wholly inside the pair or include either
or both delimiters; reverse direction and all selection provenance are retained through normal
range mapping. Pure virtual targets are skipped; no padding is materialized. Matching reuses
existing pair/bracket classification, with a cold explicit-command scan: balanced mixed brackets,
quoted spans shielding brackets, single/double quotes shielding each other, and odd backslash
parity escaping delimiters. Quotes may span lines; no language/parser or comment inference.
A mismatched closer invalidates currently open bracket candidates; an unmatched opener/quote
is never edited. Unmatched targets/counts and unchanged replacements are no-ops. Distinct valid
nonoverlapping targets may still run when another target is unmatched. Identical pair targets
are deduplicated; distinct nested/crossing full-pair spans reject the entire batch atomically.
Only delimiter scalars are replaced/deleted; interior bytes (including escapes, CRLF, combining
characters and emoji) are never rewritten. Replacement does not escape/reformat the interior.

Preparation is immutable and completes before IME/history admission. Direct actions catch up
shared revisions first; forms pin document identity/version and complete SelState plus selection
generation and refuse stale submission. Viewer and Follow refuse before mutation; filesystem
read-only buffers retain existing editable-buffer policy. Successful batches use existing source
range/snippet/view mapping and one Undo/Redo step; rejected/no-op commands retain composition,
text, selection/history and saved state. Ordinary wrapping, empty-pair backspace, overtype,
word transposition, syntax selection and all earlier interfaces remain unchanged.

Acceptance includes `(value)` -> `[value]`, count2 removing only the outer pair of `((value))`,
quoted/escaped and malformed/unmatched input, Unicode/reverse/multi-carets, identical targets,
nested conflicts, exact delimiter-only edits, source/display/shared mapping, IME/protection,
form validation/staleness, native-menu/palette/custom-binding and actual ReplayMacro routes.
Canonical N07 and the three plan boxes stay unchecked until root verification and scoped review.

Existing `typing::pair_of` supports exactly round/square/curly brackets and single/double quotes;
backticks and additional quote pairs are absent. N07 keeps that exact vocabulary.

Form capture settles the previous command's pending selection checkpoint before pinning its
monotonic generation, as syntax selection already does. Later ordinary frames then preserve the
unchanged source snapshot; moving away and back still invalidates it. Direct edit preparation
remains immutable and does not checkpoint rejected or unchanged commands.

N07 quote/boundary clarification before the matching fix: only complete quoted spans shield
brackets. A quote glued to a preceding word character or the same quote uses automatic typing's
existing non-opener rule, so contractions such as `don't (value)` keep their bracket target.
An unmatched non-target quote is transparent to later balanced brackets; the unmatched quote
itself remains a no-op. At adjacent pairs a caret exactly on the next opener belongs to that
pair and its actual enclosing ancestors; a previous sibling never satisfies count 2.

N07 accepted after independent Spec Compliance PASS / Task Quality Approved, no product findings.
Root rechecked all901 frozen mappings/28 tools and416 current products before this metadata update;
392 source inputs and7 runners remain byte-identical. Full2866 passed/29 old ignored, focused8+7,
retained53+31/16/16/71, strict six all-target/fmt/diff/preservation passed. Review's sole Minor is
inherited Cargo future-incompatibility output (block0.1.6/proc-macro-error2 2.0.1), retained rather
than suppressed. Physical installed-menu/final combined workspace/release/global performance
checks remain explicitly root-final work, not a scoped product gap. Exact report/review/raw/freeze
and root corroboration are retained in task-37-* evidence. Canonical37/64 selected accepted.


### Expanded Task38 / N08 — pre-edit behavior contract

Selection > Invert Selections computes the physical document-relative complement of the union
of selected ranges; empty/virtual-only selections occupy no source. Split Selections by Regex,
Keep Selections Matching Regex and Remove Matching Selections open existing forms and resolve
typed recordable editor actions, also available to custom bindings/macros. No default keys,
settings, persistence or dependencies. Regex defaults case-sensitive (optional Ignore case);
inline flags work. Empty pattern is a valid zero-width regex, unlike an empty Find query.
Predicates search each complete selected physical string independently (anchors/lookaround
see that string alone), accepting any match; they never search the whole document implicitly.
Split uses the same per-selection context, non-overlapping Pattern matches, excluding delimiters
and empty pieces. Zero-width matches divide at Unicode scalar boundaries with Pattern's existing
progress rules. A no-delimiter split retains the full original selection. Retained predicates
keep exact direction, goals, virtual endpoints, rectangular origin and primary; split pieces
keep direction/origin and original outer virtual endpoints where still present. Overlap uses
the existing normalizer. Primary follows its retained original or last split child; if removed,
the nearest preceding surviving original (else first). Inversion creates forward physical
ranges with the first as primary. Empty results retain one bounded physical caret at the old
primary head. Source bytes/version/saved identity/dirty/text Undo/Redo never change.

All planning/errors finish before selection history changes; Pattern syntax, 10MiB engine size,
1,000,000 backtracking limit and one existing 2s UI deadline apply, failures discard all results.
Selection changes checkpoint through existing N16 history/generation. Viewer/Follow and
filesystem read-only remain selectable, matching existing selection-only commands. Active IME refuses without unmarking; forms
also refuse any shared-view composition, revision lag, source/selection changes and away/back
generation changes after capture. Forms settle preceding checkpoints before capture.

Acceptance: abcd/b => a + cd; a,b,c/comma => a/b/c; reversed/overlap, empty, zero-width Unicode,
full-selection predicates and virtual provenance; selection undo/redo restores originals; real
source redo branch and saved identity survive; typed JSON/custom bindings/public ReplayMacro,
native menu/palette/forms, invalid/runaway/stale/IME/Viewer/Follow/shared catchup are covered.

Task38 pre-pin compatibility amendment: root confirmed that Viewer/Follow must allow N08,
matching existing selection-only commands (the initial provisional refusal was removed before
final checks). Native menu/palette entries use cold workspace wrappers, and custom-command /
ReplayMacro execution adds only a N08-type admission branch before existing execute_command
logic, using existing all-window open_editors to refuse shared IME. Editor catalog handlers
independently refuse local IME and use ViewSelect; no old typing/search/input/history/normalizer
body changes. Runtime search errors use the editor status; invalid syntax remains in the form.

Task38 CRLF pre-correction amendment (root ruling after V1 self-review): a split whose ANY
resulting physical endpoint falls between CR and LF refuses the ENTIRE operation with a visible
error, preserving all selections/history/source, including unrelated targets. Matching the full
CRLF delimiter is supported. Empty and zero-width patterns follow the same rule. This preserves
the existing off_cr_lf invariant without re-including delimiters. The compiled test-only CRLF
regression failed against the archived, still-bugged V1 backend before the narrow cold guard.

N08 accepted after fresh independent Spec Compliance PASS / Task Quality Approved, no product
findings. Full2886 passed/29 old ignored, focused10+10, retained31/31/8+7, strict six all-target
including app/fmt/diff/preservation and exact serial typing/complete-find/UI10M gates all passed.
Root rechecked all1446 frozen mappings/29 tools and416 current products BEFORE this metadata
update;392 source inputs+11 runners stay identical. V1 was historical/unaccepted due CRLF and
899 exact archived mappings/actual binaries preceded the6-line cold V2 fix. Compiled CRLF RED
and current whole-batch refusal/valid-CRLF/Unicode/history tests close it. Canonical38/64 accepted.
Fixture qualification: benchmarks' definitions/hot bodies/budgets are unchanged, while generated
100k-line and repeated big_file bytes change. Frozen report/proof mistakenly labels big_file as
1000 lines; actual benches/mod.rs uses100000. Root independently reconstructs both generators
and verifies the current106867566-byte fixture; root-task38-fixture-correction.json resolves the
wording while original frozen evidence stays intact. No historical per-invocation fixture SHA
capture or unrun check is claimed. Inherited Cargo future-incompatibility notices remain visible.
Physical installed-menu and final combined workspace/release/global gates are root-final work.


### Task39 / N09 implementation contract — before product edits

Adjust existing whole physical selected values, never generate a sequence or infer a token at a
caret. Empty/virtual-only targets are skipped without padding; no nonempty targets is a no-op.
Add recordable typed `editor::AdjustValues { step }` (default +1), Edit > Text Increment Values,
Decrement Values and Adjust Values… signed-step form, shared palette entries, custom binding and
actual macro replay. The form defaults to 1, settles the existing selection checkpoint before
capturing buffer/version/full SelState/generation, and refuses source or selection away/back
changes. Use the existing pure pipeline Op `adjust_values` on its entire per-selection/document
input, with existing scope/admission/atomic application; no line splitting or new engine.

`ValueStep(i128)` uses decimal JSON STRINGS for every explicit step, including small values;
reject JSON numbers/floats rather than silently round/narrow through serde_json::Value. The default
is 1. Values are parsed directly from source text with checked signed i128 arithmetic. Optional
leading +/- precedes optional exact 0x/0X/0b/0B/0o/0O prefix (otherwise decimal). ASCII digits,
underscores strictly between digits, no whitespace/floats/suffixes. Hex alphabet must be uniformly
upper/lower; mixed case is unsupported. Retain the exact prefix, input hex alphabet case (prefix
case when it has no letters), minimum digit width and underscore boundaries measured from the
right; growth expands the leftmost group, without inventing new underscores. Preserve explicit +
on nonnegative results; - appears only for negative results; crossing zero drops a prior minus.
Negative zero becomes unsigned zero on nonzero adjustment to zero; zero step validates all targets
and preserves their exact strings. Signed i128 MIN/MAX supported in every radix with checked
magnitude parsing (including MIN); overflow rejects the entire operation.

Strict date YYYY-MM-DD uses proleptic Gregorian years 0001..9999, days as the step. Strict time
HH:MM or HH:MM:SS uses minutes, wraps modulo 24 hours, retains the original seconds field.
Date-times are YYYY-MM-DDTHH:MM[:SS], optionally followed by uppercase Z or ±HH:MM (offset hours
00..23, minutes 00..59); minutes move local calendar fields and preserve the zone bytes and seconds.
Uppercase T/Z only; no spaces, fractional seconds, leap seconds, compact zones or other ISO forms.
Hours 00..23/minutes and seconds 00..59; Gregorian leap/century rules and month lengths validate
before arithmetic. Dates/date-times reject results outside 0001..9999; huge i128 steps never narrow.
No new dependency/settings/default keys and the old i64 Column Editor Sequence remains unchanged.

Immutable preparation validates ALL original selected values and produces all replacements before
native IME/history/editing. Viewer/Follow refuse untouched; filesystem-read-only retains the normal
in-memory editing policy. Shared-view composition refuses, local successful composition follows
normal edit commit; failures/no-ops preserve marked input, source/version/saved identity, full
selection provenance/generation and real redo. Catch up shared revision before preparing; use
one existing replace_ranges batch/one Undo/Redo with snippets/overtype/shared mapping unchanged.

Acceptance: literal 009→010, 0x0f→0x10, negative/zero-crossing/sign/prefix/case/padding/irregular
groups; all-radix MIN/MAX and huge signed step serde/record/replay; invalid later mixed/Unicode
selection atomicity; leap1900/2000/2100/month/year and positive/negative date/time edges/seconds/
zones/invalid complete forms; native/palette/form/default/stale/invalid-step/binding/actual
Command::build/public ReplayMacro; shared catch-up/IME, snippets, provenance and existing numbering.
Compiled causal REDs precede implementation. Full affected suites+Git serial/strict six all-targets
including app/fmt/diff/preservation, then the same unchanged serial release typing+COMPLETE find+
UI10M find gates because generated EDITING_MODEL fixtures change. Pin all inputs/scripts before
final checks and retain actual binary COW copies; leave canonical N09 and plan boxes unchecked.


N09 pre-pin root ruling (after compiled initial REDs): mixed-case hexadecimal remains a supported
whole hex integer. Preserve case at aligned source alphabetic digit positions; newly introduced
letters follow the first source alphabetic digit's case, or prefix case if no source letters.
This supersedes only the uniform-case restriction above. Exact prefix/padding/underscore policies,
zero-step exact strings and all other contract clauses remain. Compiled mixed-case RED precedes
this correction; literal carry/growth/signed/grouped cases cover the ruling.


N09 pipeline pre-pin owner amendment: the prior pipeline UI entered Editor::edit before validation.
A compiled causal IME regression proves that a failed AdjustValues pipeline cleared marked state.
Only pipelines containing AdjustValues now prepare all outputs first using a detached duplicate
view and the EXISTING transform_targets/transform_texts, complete Pipeline::run operation order
and LF rules. Successful prepared outputs feed unchanged transform_selections (including its
forward/widened target selection and final empty-line behavior), preserving those semantics that
replace_ranges would change. Whole-document open-buffer file runs similarly prepare before the
unchanged transform_document application. No-op/error bypass Editor::edit. Existing shared-view
admission and file-run admission/cancellation/leases remain; purevirtual-only scoped inputs skip.
Pipelines without AdjustValues retain their original path. This is an additive preflight, not a
replacement pipeline, transform, transaction or selection engine.

N09 review fix1 behavior contract: actual typed native/keybinding AdjustValues actions bubble
to the workspace for the existing all-window shared-composition check before borrowing the
target editor. The action still targets only a focused document editor; global bindings cannot
edit a hidden source from another control. Editor::perform_action keeps the existing guarded,
synchronous JSON/macro path, and native editor capture keeps one resolved recorded command.
No new wrapper action, registry, deferred execution, default key or persistence is introduced.
The exact reviewed V1 (931 archive mappings, actual11 final binaries, four causal copies and
large fixture) was preserved before edits. A compiled actual-keystroke/peer-native-composition
regression failed on V1 before this fix; the same test also checks direct native dispatch,
full source/selection/marked/save identity and a genuine source redo after refusal. Additional
focus/recording coverage and all covering V2 checks precede fresh scoped re-review. N09 remains
unchecked until that review; inherited dependency notices remain visible.

N09 accepted after ROOT fix1/5 and fresh scoped re-review: I1 native binding shared-IME bypass
addressed, Spec PASS/Task Quality Approved/no new findings. V2 full2909/29oldignored/focus9+14,
strictsixalltargets inclapp/fmt/diff/preservation and unchanged3 serial release gates passed;
root proof1496 mappings29tools/416products392source13runners/3427oldhelpers/11actualCOWbinaries/
931reviewedV1archive520COW/causal8/report9-chain. This acceptance follows actual pre-metadata
readback of every frozen current product and report link; no source/script change through metadata.
Canonical39/64 and this task's three plan steps now checked. Step+1/string serde/no new keys,
settings or dependencies; existing numbering and complete pipeline semantics retained. Final
physical/native/workspace/release/global verification and older N08 native-binding follow-up remain.

N08 native-binding follow-up — short behavior specification before edits

Defaults and entry points remain unchanged: Invert/Split/Keep/Remove selection sets, their
catalog identities, regex options, menus/forms/JSON/macros and user keybindings. Native typed
actions must reach the existing Workspace all-window selection_set_admission BEFORE target
Editor borrowing/catch-up/history. Require the actual source editor to be focused, including
context-free custom bindings; report shared/local composition refusal through existing status.
Reuse N09's explicit Workspace ownership pattern only for these four actions. No general
action registry, wrapper actions, async/deferred macro path, model/parser/regex rewrite, new
setting/dependency/default key or persistence. Selection-only operations remain permitted in
Viewer/Follow/filesystem-read-only documents. All source bytes/save identity/source redo stay
unchanged, and refusal preserves full SelState, generation and both views' native marked state.
Acceptance: installed actual keys AND typed native dispatch for all four actions against a
natively composing shared Unicode peer with reverse/rectangular/virtual selections and a real
source redo; global-focus refusal; successful native actions in ordinary/Viewer/Follow/readonly
modes, literal selections and one recorded command per key. Retain existing selection UndoRedo,
regex/CRLF, forms/staleness and replay coverage; rerun full affected/strict/fmt/diff and exact
serial relevant release gates. Record causal RED before production repair, final input pins,
actual binaries/raw exits and independent scoped fix review before acceptance/commit. The
clean accepted N09 state416products392source/11 immutable binary copies was preserved first.

N08 native binding follow-up accepted after ROOT fix1/5 + fresh scoped SpecPASS/QualityApproved,
no new findings. All4 actual keys+all4 typed native routes now refuse shared/local composition
before target Editor borrow; context-free focus, capture once, read-only/Viewer/Follow selection
access and selection Undo remain. Full2911+29oldignored/focus10+12/retainedN0923+40/27/15,
strict6alltargets/fmt/diff/preservation3462helpers/3exactserialreleasegates passed; root proof907
mappings29tools/416products392source6runners/11current+11priorCOW/causal8/format-only identity/
9-linkcorrectedV1b report+3retainedfailedmetadatarefs. All416products rechecked BEFORE this
intentional metadata update;392source/6scripts stable through acceptance. Count remains39/64;
no extra feature checkbox or Task40 snapshot. N08 native-entry follow-up is closed. Final
physical/native/workspace/release-app/global gates remain root-owned after all64.


N10 — short behavior specification before product edits

Align on Pattern… defaults to regex `=` and capture index 0 (the whole match), with optional
Ignore case off. Every ordinal match column aligns its chosen capture start across the relevant
source lines; leading spaces/tabs in the capture are existing padding, so the insertion anchor
is its first other character (the capture end for whitespace-only captures). Unmatched optional
captures and empty captures are skipped without changing the line or shifting later ordinals.
Any nonempty physical selections scope their covered source lines once, with an end at the next
line's start excluding that line. Ordinary carets use the document, like line transforms; pure
virtual-only scope never falls back. Align Selected Fields uses only nonempty same-line physical
ranges, sorted into ordinal columns on each line, and appends spaces inside each selected field
until its right edge reaches the common display column. Empty/virtual-only ranges stay untouched.
Both commands keep original text, delimiters, tabs and line endings byte-exact except inserted
spaces. Each later field is measured on detached row text after earlier padding, using existing
grapheme/tab display widths. Repeated execution is a no-op; selection replacements include their
padding. All bounds, same-line, overlaps, grapheme and CRLF insertion boundaries validate before
any edit/history/IME mutation. Errors and no-ops preserve source/save/SelState/primary/goals/
rectangular/virtual/generation and real source redo. Accepted edits use existing replace_ranges
for one batch UndoRedo, with shared-view mapping. Pattern compile/capture-index validation uses
the shared Pattern dialect and actual group count. No new dependency, setting or default key.
Reachable shared Edit/Columns menu and palette, pattern form, typed JSON/macros and actual custom
keys use the current owners; typed native actions bubble to Workspace for focused-source and
all-window composition admission before borrowing Editor. Viewer/Follow refuses source editing;
filesystem read-only keeps ordinary editable-buffer behavior. Forms pin buffer/version/complete
selection/generation and reject stale submissions or later composition. Tests cover key=value,
selected capture, two columns, tabs/CJK/combining/supplementary, empty/unmatched captures, malformed
regex/index, overlap, split grapheme/CRLF, idempotence, one UndoRedo, native keys/shared IME/focus/
record once and form staleness/public replay. Canonical N10 and plan steps remain unchecked.

N10 selected-field causal check found that splice trims unchanged prefixes, so ordinary
replace_ranges leaves a selection endpoint before newly appended spaces. The additive selected
application uses the unchanged batch/Undo and existing map_sels(after=true) for physical fields,
ordinary mapping for empty/virtual ranges, then existing normalization; it introduces no mapper
or history engine. Original reverse/rectangular/primary/virtual provenance is retained. The
compiled regression demonstrated repeated growth before this endpoint-affinity correction.

N10 padding normalization (ROOT-approved before final pin): a whitespace-only chosen capture
anchors at the end of its complete adjacent horizontal space/tab run, even when the regex
consumes only one space. Multiple matches naming that same padding run occupy one normalized
column; this includes full whitespace/zero-width padding matches whose chosen capture is
missing/empty. Distinct nonpadding missing/empty matches retain slots. Coincident valid physical
anchors are one boundary. Literal `a( )` and `(=)| ` regressions failed on compiled pre-repair
code and are retained with actual binary bytes; this avoids repeatedly growing padding or
shifting later capture columns. The selected application uses actual logged splice changes,
original full selections/primary and existing true/false mapper affinities, then normal CRLF/
goal clearing and normalization. Measurement now streams each original row segment once from
its running display column; no repeated row insertion or prefix scan per regex field.

N10 final padding-boundary clarification: a padding match whose selected capture is missing
still names its run-end boundary. A later selected capture at that exact boundary fills the
same normalized column, rather than creating a new slot. Distinct nonpadding missing/empty
matches still retain slots. Compiled `(=)|.` on `日==`/`x==` exposed the previously unfilled
coincident slot; its causal source and actual executable were preserved before correction.
The simpler broad `(.)` fixture passed before that correction and is covering GREEN only.

N10 topology ruling (ROOT-approved, before final pin): a candidate must be settled under the
same compiled pattern and ordinary mapped source scope. Preflight applies the candidate only
to a detached rope/view with existing replace_ranges and re-plans; further required padding
refuses the entire real edit with “The pattern changes its field columns after padding; include
optional whitespace around the capture”. The literal `a(=)` two-column regression failed on
compiled pre-guard code; `a\s*(=)` is the accepted stable equivalent. No cached last-result state,
pattern rewrite, general regex/transform engine or real history mutation is introduced. The
same shared Pattern and one shared two-second deadline bound both planning passes together.

N10 V2 authorized menu-fixture migration: the first pinned V1 full affected run passed all new
model/UI cases but the old exact Columns label list omitted the two new authorized entries.
ROOT approved adding ONLY those two expected literals in that old menu test, retaining every
old item/order/assertion and delimiter default/selection/status check. The exact V1 pin/products/
scripts/logs, all six actually executed full-run debug binaries, seven retained actual debug
binaries and eleven prior accepted N08 executable copies were preserved in a 1361-mapping
archive before any edit. V1 release gates had not started and are not claimed. V2 preservation
reverses only the two literals in that exact function to reconstruct the original entire old
function/prefix; all other old tests remain literal. Fresh V2 pin/full checks and all three
unchanged serial release gates are required. Snapshot40 was not recaptured.

N10 review repair contract (ROOT, round1): ordinary carets keep document-wide fallback even
beside virtual-only selections; exclusively virtual-only scope remains a no-op. Detached
fixed-point preflight now carries the existing value-only snippet session and its pending
change log, then invokes existing sync_snippet before replanning. It refuses an unstable
mirror result before any real edit, preserving source, full SelState, generation, save identity,
snippet and redo. Two Clone derives permit a detached value copy; global duplicate, snippet
mapping, history and publication semantics remain unchanged. Acceptance regressions use the
real identical-source snippet expansion (so existing real redo survives), both published and
pending logs, subsequent live mirrored typing/UndoRedo, and mixed ordinary/rectangular virtual
selections with one UndoRedo. All three compiled regressions failed against reviewed V2 and
passed after the narrow fix; actual RED source/log/binary retained in private evidence. No
new dependency, setting or default key. Canonical N10 remains unchecked until fresh verification
and scoped review.

N10 accepted after ROOT fix round1 and fresh scoped re-review: I1/I2 ADDRESSED, no new
Critical/Important findings. Final2937passed29priorignored, focused18model8UI, retainedcolumns55/
snippets30/N0923/N0822/N0715, strict6alltargetinclapp/fmt/diff/preservation and same3serial
release gates PASS. Main2998mappings29test/tools11actualCOWbinaries;416products392source24runners
unchanged from finalpin. Exact reviewedV2 archived beforefix; compiled3RED+418source/log/binary
mappings retained;9linkreport/readback and independentROOTv3b proof corroborated. Canonical40/64
now accepted; all3Task40steps checked. No new dependency/settings/defaultkeys. Historical audit
unchanged; final combined native/workspace/release/global gates remain ROOT-owned.


### Task 41 / N11 pre-edit behavior contract

Defaults and entry: the current-buffer Find/Replace panel starts with Any. An explicit syntax
control cycles Any, Comments, Strings, Comments or Strings; a separate Include/Exclude control
inverts the selected class. Any always means ordinary unrestricted syntax eligibility. These
transient panel choices compose with regex, case, whole word, wrap, preserve case and the original
selection scope; Find Next/Previous/All, Replace/Replace All and document extraction share them.
Find in Files retains its existing independent options.

Classification: use the current highlighter's bundled parser scopes and authoritative fallback
grammar stacks, with the existing comment-before-string precedence. Only the match START owns
eligibility, even if a regex crosses scopes/lines or enters/leaves interpolation. Newline/EOF
positions use the current highlighter's last-character/empty-line scope convention. Prepare one
bounded cancellable background classification per exact document/rope/version/grammar identity,
reuse it across queries, and leave the Any hot path unchanged. Plain Text, over-16-MiB sources,
lines beyond the grammar's exact coloring bound, parse failure, timeout and cancelled/pending or
stale results provide no eligible matches and visibly say syntax search is unavailable/preparing.
Never substitute Any. No dependency, parser, search engine, setting or default key is added.

Application: a replacement rechecks the exact eligible source snapshot, current query/options/
selection scope, shared revision, immutable Viewer/Follow flags and all-window composition before
Editor mutation. Refusal preserves source, marked text and Undo/Redo. Successful replacements
use the existing replacer/edit/history path once. Source or grammar changes invalidate classification
and cancel old work; late completion cannot publish into another panel/source.

Acceptance: retain compiled causal RED for the absent reachable syntax control and filtered
replacement; test code/comments/normal+raw strings, interpolation/Unicode, include/exclude masks,
start-only cross-scope/multiline/zero-width matching, all option and selection composition,
Replace Next/All Undo/Redo, changed-source/grammar/unsupported/cancel and shared-IME/read-only
refusals. Preserve every old test/helper/assertion, benchmark/default-key/dependency/workspace
input. Full affected and strict checks precede exact unchanged serial search/input/highlight/render
performance gates; derive actual included-source fixtures before final checks and read back actual
100-MiB gate input. Canonical N11 and its three plan boxes remain unchecked for root review.


N11 implementation choices resolved before final pin: the mask lives beside the existing search
model, while FindPanel owns transient mask/exclusion and the exact source snapshot. The ordinary
FindOpts/default keys and cached engine matches remain unchanged. Highlighter supplies a fresh
owner token, existing Tree-sitter source revision and strict parser/grammar-scope classification;
no approximate painting fallback authorizes replacement. CSS lexical context walks from line zero.
JS fixtures independently confirm template text String and interpolation identifier Code; fallback
grammars keep their own enclosing-stack/comment precedence. Source edits cancel immediately,
clear eligibility and debounce 300 ms; bounded worker results pin view/document/version/rope and
grammar identity and cannot restore a closed/replaced panel. Two-second classification, 16 MiB,
20,000-byte line and one-million-run caps are explicit unavailable states. Missing classification,
including exclusions, returns no eligibility; the panel retains a visible reason even with
highlighting disabled. New replacement admission reuses all-window composition and Viewer/Follow
checks before Editor mutation, with the existing atomic replacer and real Undo/Redo.

Compiled pre-feature GPUI RED for the absent reachable syntax eligibility control is retained with
all exact products, logs and the independently copied actual Cargo Running test executable. The
same real control/comment-only Replace All regression passed after implementation. Retained setup
failures (Rust quoting/type/borrow and hand-counted new-fixture offsets) are not causal evidence.
Debounced UI tests explicitly advance GPUI's virtual clock through the existing settle helper;
pre-followup exact source/logs/three actual executables are retained. Focused verification before
pin passed 2 model, 4 syntax and 9 UI regressions, including all four actual control choices,
include/exclude, multiline and zero-width starts, selection/options, stale source/grammar/closed
panel, native Replace All keys with local/other-window composition and preserved real redo.
Strict six-package all-target checks including app passed before pin; all 3487 old regression/
helper bodies, existing assertions/default keys/bench definitions/budgets/ignore flags and locked
775-package/ten-member graph are preserved. Full pinned checks and ten unchanged serial release
gates follow; canonical N11 and all three plan steps remain unchecked for ROOT's scoped review.

### N11 ROOT review repair contract — round 1

A valid syntax-filtered query must remain usable by Find All, extraction, bookmarking and
persistent highlights when only the replacement template is invalid. Keep the replacement
error visible while the panel remains open and continue to refuse replacement. Guard the
shared search consumers on current classification and matcher readiness/failure, using the
existing engines and source identity checks. Unsupported, stale or pending classification
must keep its visible refusal; ordinary Any behavior stays unchanged. Acceptance uses actual
Rust comment classification, regex query `hit`, invalid replacement `$1`, literal eligible
source ranges, nonmutating consumer results, unchanged source/save state, and real redo.
Preserve every V1 test/helper, including stale/unsupported/IME cases. Two new compiled causal
regressions must fail on the reviewed V1 before the narrow fix and pass afterward. No new
setting, action, dependency or persistence format. Canonical N11 remains unchecked until fresh
verification and one scoped re-review.

N11 repair causal result: both new GPUI regressions compiled and failed on the reviewed V1
(Find All kept its panel open; bookmarking returned no eligible lines). Exact 416 product
blobs, raw log and actual UI executable are retained as 418 mappings before the logic fix.
The same tests pass after carrying explicit classification/matcher readiness through the
existing shared search-consumer seam. They check literal comment ranges, Find All selections,
bookmarks, persistent highlights, extraction clipboard bytes, unchanged source/save identity,
retained replacement error/refusal and real redo. Ordinary Any matching and all V1 tests are
retained. The minor ROOT summary field error is separately preserved and qualified; the new
proof must serialize the verified fixture dictionary without reusing a later hash-loop variable.

### N11 accepted — selected feature 41 of 64

ROOT fix round 1 and its fresh scoped review close I1 (query readiness independent from
replacement errors) and E1 (explicit correction of the retained proof summary). All 416 final
V3 product bytes equal the reviewed V2. Final 2,954 tests passed with 29 prior ignores;
focused 2 model/4 syntax/11 UI, strict six packages/all targets including app, retained search/
highlight/N10/snippet/column/N09/N08/N07 checks, formatting/diff/preservation and ten unchanged
serial release gates passed. Exact source/scripts/logs, 12 current executable COW copies,
3499 main mappings/30 tool paths and a nine-link completed report chain are corroborated.
The failed V2 metadata freeze and both exact before-edit archives remain retained; V3 only
corrected its historical reference and reran all checks on identical products. Final source
392 and 24 actual runners remained stable. Canonical N11 and all three plan steps are checked.
No new dependency/settings/default keys/schema; final combined native/workspace/release/global
gates remain ROOT-owned after all selected features.


### Task 42 / N12 pre-edit behavior contract

Default Find in Files, `<previous results>` (whole matching files), textual Find Results and
the results browser's path/text filter keep their existing behavior. Add an explicit
**Previous Matching Lines** Where choice, spelled `<previous matching lines>`. A new query
uses the chosen structured results' physical hit lines, including the entire lines touched
by multiline hits, independently of inclusion checkboxes, context rows and browser filtering.
Adjacent hit lines merge; a complete new match must fit one merged interval and cannot cross
a gap. Regex anchors, lookaround and whole-word edges still see the full source. As with
existing selection scopes, a zero-width match at an allowed interval's end is included.

The choice retains source path/identity/version, exact open-buffer rope and merged character
intervals. Closed files retain their actual decoded disk stamp/hash/codec/BOM/newline evidence.
Query/options/filter edits and Refresh of the new query reuse that choice rather than narrowing
it again. Selecting the explicit choice again captures the currently displayed completed
result set; an empty result set refuses visibly, and mixing this restricted scope with other
source tokens/paths refuses rather than broadening. Include/exclude globs may further filter
its files. In-progress results must finish or be cancelled before choosing their lines.

Changed, missing, closed, rebound, reinterpreted or write-overlapped sources produce a visible
per-file stale/Refresh explanation and no matches/replacements for that source. Unchanged
sources remain searchable. Refresh reruns the new query against the retained source snapshot;
refresh the original source query and explicitly select its lines again to renew stale bounds.
Open unsaved/shared buffers are searched as captured, aliases cannot substitute another source,
and closed-file reads use the retained codec and existing read-admission tickets. New results
revalidate source/tickets when admitted on the UI thread; normal replacement preflight remains
mandatory. No new mutating native action, dependency, persistence setting, result pages (N13)
or iterative narrowing (N14) is introduced. A replacement-only template error does not prevent
a valid nonmutating Find.

Acceptance covers two-query line/file/browser differences; multiline complete-fit, adjacent
lines and disconnected gaps; Unicode/CRLF, end-position empty matches, full-source lookaround/
word context; unsaved/shared/aliased buffers, legacy decoded disk sources, deletion/rebinding/
source edits before and during delivery, write-ticket overlap, cancellation and read-only/IME
replacement refusal. Existing helpers/assertions/default keys/budgets/ignores and locked graph
remain intact. Canonical N12 and its three plan steps stay unchecked for ROOT review.

N12 causal proof: the first UI test initially needed its own virtual-clock search-drain wait;
that harness failure and actual binary are retained separately. The corrected compiled RED
then failed at the intended assertion (zero second-query matches instead of one), and passed
after implementation. A second compiled causal failure caught canonical collection losing an
explicit symlink's original route: rebinding that alias incorrectly retained two eligible files.
The exact source/log/actual COW executable were retained before repair. Captures now keep the
original typed route from `shown`, so alias rebinding yields a visible stale per-file result.
Four editor and eight UI focused regressions passed before final pin.

The existing matcher supplies bounded complete matches; no regex engine or plugin was added.
`ResultSet.matching_lines` holds each run's own source hit lines and `previous_lines` pins the
chosen prior set independently. Search publishes only current source/ticket results; Apply
rechecks those identities and all-window native composition. No new mutating action or default
binding is needed: the existing Where chooser, Find/Replace/Refresh/Apply routes reach N12.

Performance selection inspects the unchanged bench definitions: search/files.rs is part of
`benches::RUST`, so actual concat/100000-line/repeated 100-MiB fixture bytes are derived before
final checks and read back after release gates. The exact serial typing, COMPLETE find and
UI 10M find gates are required. The changed file-search owner also requires the 99-MiB
single-line Find in Files gate. The seven existing syntax/render fixture gates cover the changed
included Rust source's classification/render work. Index, disk polling, saving and loading
implementations remain unchanged; no new index/filesystem owner or measured hot loop is added.
Canonical N12 and all three plan steps remain unchecked pending ROOT's independent review.

N12 lifetime self-review added a ninth UI regression: explicitly choose source lines, edit the
source, refresh/publish the original query, then run the new restricted query. It compiled and
failed causally (two matches instead of stale/zero); exact source/log/actual COW binary were
archived before repair. The chosen snapshot now belongs to transient `Workspace.previous_lines`,
while each `ResultSet.previous_lines` pins that run's scope for Refresh/Apply. Source result
publication, panel replacement and unrelated queries cannot renew it. Only an explicit chooser
selection replaces it. All nine focused UI cases passed after this narrow owner correction.
Final pin/check/freeze evidence follows in the task report; acceptance remains ROOT-owned.


### N12 accepted — 2026-11-01

Explicit Previous Matching Lines now retains immutable source identity/version/rope or disk codec/stamp/hash and full physical hit-line bounds. Complete new matches fit one merged interval; stale sources visibly refuse and replacement retains existing checked admission. The original choice survives new queries and Refresh publication. All 30 final phases passed (2,967 tests, 29 prior ignores, 4 editor/9 UI focused, strict six packages, eleven serial release gates); ROOT independently corroborated 2,216 mappings/29 actual tools/11 executable copies and report9, and scoped spec/quality review approved. Causal source/log/actual-binary archives and all preliminary failures remain. The resumed readback reached the last once-only proof guard; its exit and retrieved original proof are explicitly reconciled, with no overwritten proof or reconstructed output claim. No new dependency/default/persistence. N12 is the 42nd of 64 selected entries; final combined native/workspace/release/global performance checks remain.


### Task 43 / N13 — pre-edit behavior contract (2026-11-01)

- Keep the existing active `Workspace.result_set` and all matcher, N11 syntax, N12 frozen-line,
  replacement/source/version and append-only textual Find Results engines. Add transient
  per-window page storage with stable monotonic page IDs; no persistence or new dependency.
- Each submitted Find/Replace creates a page, retaining query, replacement, Where/options,
  source versions, hit-line basis, counts/status, inclusion/collapse state, browser filter,
  row selection and review note. Switching restores the chosen page without rerunning it.
  A page owns its exact N12 basis; Refresh always uses that basis, even after a different
  explicit window line choice. Ordinary new queries retain the existing explicit-choice rule.
- At most eight pages. Creating a ninth retires the oldest inactive non-running page by
  creation ID. Admission happens before cancellation/eviction; if no eligible slot exists,
  refuse visibly and preserve the active/running pages. Switching keeps a running search
  alive; starting another run cancels the prior run with retained partial hits. Closing its
  page cancels it and removes it permanently. Refresh cancels the prior single search and
  replaces only the selected page's contents, preserving its ID.
- Every drain/progress/completion/cancel carries page plus request identity. Late drains may
  neither publish into another page nor resurrect a removed one. Page navigation invalidates
  pending browser jumps. Apply remains selected-page only with existing stale/source/write
  checks; while Apply runs, switching/closing/starting/refreshing refuse visibly, and its
  callbacks additionally verify the captured page ID. Cancel Apply keeps its existing
  next-file boundary. No new mutating editor action/default key is introduced.
- Reachable browser page buttons switch and close exact IDs, including inactive pages;
  Refresh and existing Search Results commands act on the selected page. Closing the final
  structured page hides the browser without closing or clearing textual Find Results.
- Causal proof: two real searches must retain both independent results rather than overwrite.
  Acceptance also covers refresh/switch/options/checkboxes/filter, selected Apply/undo/stale,
  eight/ninth retention and no-eligible refusal, cancel/close-running/late delivery/window
  isolation, exact N12 bases and old text-result navigation. Preserve old tests/helpers,
  source guards, all keys, budgets, ignored flags, assets and 775-package/10-member graph.


### N13 accepted — 2026-11-01

Up to eight transient per-window structured pages retain their own query/options/resolved scope/status/checkboxes/filter/review/source facts and N12 frozen line basis. Stable page/request identity isolates publication, cancellation and selected-page Apply; switch/close/Refresh are reachable browser controls, while textual Find Results retain append/navigation behavior. V1 exposed five old asynchronous navigation regressions; exact source/log/actual binaries were archived before replacing synthetic filter edits with retained per-page filter entities and current-identity subscriptions/focus handoff. All 31 final V2 phases passed (2,977 tests, 29 prior ignores, 10 N13 and 4+9 retained N12, strict six packages and eleven serial release gates). ROOT independently corroborated 2,287 mappings/29 actual tools/11 current executable copies, failed V1 and causal archives, actual fixture and report9; scoped spec/quality review approved. No new dependency/default/persistence. N13 is the 43rd of 64 selected entries; final combined native/workspace/release/global checks remain.


### Task 44 / N14 — pre-edit behavior contract (2026-11-01)

- Add view-local Narrow to Selection, Narrow to Fold, Show Matching Lines… and Clear Filter
  through the existing action catalog, shared menu/palette and custom bindings. No default
  keys, dependencies, document rewrite or new projection/search engine. Selection narrowing
  keeps the union of nonempty physical ranges (virtual-only/empty selection refuses visibly).
  Fold narrowing uses an active containing fold, otherwise the current syntax/indent region;
  it includes the region's header line. Multiple targets keep their union.
- The first activation snapshots every prior fold, including nested/inline folds. Replacing
  the filter keeps that original snapshot. While active, ordinary folds are temporarily open
  and exclusion ranges use existing Folds/Rows placeholders; Clear restores the valid snapshot
  mapped with ordinary fold edit semantics. Explicit fold commands clear the filter first.
  Session and closed-view persistence capture only that mapped prior fold state; the filter,
  regex, context and its exclusions are transient and never restored.
- Regex is case-sensitive by default, supports the existing optional Ignore case control and
  existing bounded matcher, and matches the complete immutable source (including multiline
  and lookaround context). Show every physical line touched by a match plus a validated
  nonnegative whole context count, default 0, clipped/merged at document edges. Invalid regex
  or context leaves the existing filter unchanged and explains the error. No matches keeps
  one whole-source placeholder and a bounded caret at 0; an empty document stays empty.
- Focus ranges follow edits with left-sticky starts and right-sticky ends, so edits at either
  edge and replacement text remain in focus. Source edits from any shared view map the saved
  folds; regex requests recompute against the new source version. Obsolete requests are
  cancelled and identity/version/request checks reject late replies. While recomputing, show
  the current source without old exclusions. Failure clears the filter and restores saved
  folds with a visible error, rather than presenting stale matching lines.
- Activation moves hidden selection endpoints to the nearest visible source endpoint,
  preserving direction/primary/rectangle and valid virtual gaps. Ordinary navigation or source
  selection into hidden text clears the filter and reveals its destination. Copy, edits,
  undo/redo, Find and shared documents retain physical source coordinates/full source identity.
  Filter/clear never create text history, dirty or save-metadata changes; real edits retain
  existing undo/redo, snippet, overtype and virtual-space behavior. Read-only/Viewer/Follow
  allow these presentation operations; local/shared IME refuses before native Editor borrowing.
- Native typed listeners belong to Workspace and check actual editor focus plus all-window
  composition admission before borrowing the target. JSON/macros stay synchronous and native
  capture records once. The matching-lines form pins document/version/full selection and
  generation; stale source/away-back refuses without publishing a new filter.
- Acceptance: compiled causal failure must show baseline text remains unfiltered through a
  real reachable command; retain exact source/log/actual executable before repair. Cover exact
  source/folds/text Undo and real Redo continuity on clear; in-focus/shared/hidden source edits,
  nested/inline folds, Unicode/tab/wrap/virtual space, multiline regex/context/no-match/empty,
  obsolete replies/cancellation, native key/shared composition/wrong focus/macro and transient
  persistence. Preserve literal old helpers/tests/keys/bench budgets/ignores and locked graph.


N14 implementation detail settled before the complete slice: every matching-lines scan uses
existing background work, including small documents. This gives one publication seam with
request/document/version and delayed shared-composition checks. Local active composition
suspends filter exclusions without moving marked endpoints; a reply encountering local/shared
composition defers publication until a subsequent frame/action. Native entry admission remains
at Workspace before borrowing Editor; its OpenViews callback must not reread a leased Workspace.


N14 causal proof now also covers a self-review catch: virtual-only selections initially became
an empty physical focus. The compiled test failed at the intended assertion; exact source,
log, runner and actual Cargo Running binary were retained in a second 422-map COW archive
before filtering targets by physical span. Four model and fifteen UI cases then passed. The
preliminary compiler errors and the visible-boundary fixture correction remain logged honestly.
New shared views already start without folds/filter/tasks; an added regression confirms that
behavior and independent cancellation. Native bindings and delayed local/shared composition
exercise actual entry and reply lifecycles without rereading a borrowed Workspace.

Final verification will pin all product/source/build/assets, exact task/shared runner bytes and
derived concat/100000-line/repeated-big-file fixture inputs before complete covering checks.
The unchanged serial release typing, COMPLETE find, UI10M, seven render/highlight/fold/scroll
and single-line 99-MiB file-search gates cover affected hot owners and generated fixture bytes.
No benchmark definition, budget or ignore is changed. The actual temporary big-file bytes will
be read back after the gates. Canonical N14 and all three plan steps remain unchecked for ROOT.


N14's final self-review obtained two further compiled causal failures (both exact source/log/
actual executable retained before repair): an away/back selection could receive an old regex
reply, and revealing a hidden source destination inside a saved fold moved it to that fold's
start. Matching requests now pin full selection state plus settled monotonic generation;
changed selections restore prior folds with destinations revealed and refuse visibly. Hidden
navigation uses that same revealing restore, preserving the physical destination. Explicit Clear
still restores the mapped prior folds exactly. Fold Level clears filters before its background
scan. Regex scans inherit the existing 60-second background deadline and backtracking limits.


### N14 accepted — 2026-11-01

View-local physical focus ranges and existing fold placeholders implement Narrow to Selection/Fold, bounded asynchronous Show Matching Lines (context0/case-sensitive defaults) and Clear Filter. Prior nested/inline folds map through edits and alone persist; hidden navigation reveals its source destination, and filtering preserves full source/history/save identity. Native admission and deferred replies protect local/shared composition and full selection-generation freshness; pure virtual selections refuse. All 39 actual V3 checks passed (2,999 tests/29 prior ignores, 4 model+18 UI, strict6, actual rendered output and 14 serial release gates). ROOT corroborated 3,297 mappings/29 tools/11 COW executables/report9, corrected only its own shared-helper lookup through accepted retained blobs, and preserved the original private failure. Superseded verifier selector/reference issues and interrupted V2 remain qualified, with no fabricated child completion or overwritten evidence. Scoped spec/quality review approved. Fields/aligned comparisons refuse visibly; filters are transient. No dependency/default/schema change. N14 is the 44th of 64 selected entries; final combined native/workspace/release/global checks remain.


### Task 45 / N15 — pre-edit behavior contract (2026-11-01)

- Add Move to Next/Previous/Parent Fold, their Select to variants, and Select Fold through
  one shared Fold Navigation menu/palette family and the action/JSON/custom-key/macro catalog.
  No default keys, settings, dependencies or persisted history. Reuse current valid ordinary
  collapsed ranges (N14's saved prior ranges while filtered) plus current syntax candidates;
  unavailable syntax falls back to one full indentation-region scan. A valid empty syntax
  index remains authoritative for discovered regions; manual/inline ranges remain candidates.
- Multiline ranges include the header from its physical line start through the fold end;
  inline selection folds keep their exact source span. Exact duplicates collapse; distinct
  ranges sharing a header remain for strict containment. Headers are unique source offsets
  for next/previous: strictly greater/less than each caret head, source ordered, without wrap.
  At a header, next/previous skip that header; in an interior, previous reaches its header.
  EOF can move backward; forward at the end and backward at the start keep that selection.
- Current means the smallest range containing the head, half-open, except a range ending at
  document EOF also contains EOF. Parent is the nearest strictly enclosing range around that
  current range (one equal boundary is allowed, equality is not). No current/parent/destination
  leaves the affected selection unchanged. Select Fold selects current including its header,
  retaining direction; Select to variants retain the existing anchor and extend the head.
  Per-caret results normalize with existing primary-selection semantics. Moved heads lose
  virtual gaps, retained anchors keep valid gaps; no padding or text edit is introduced.
- Catch up shared edits before reading candidates. Resolve every physical destination before
  any N14 restore/clamping, then use existing selection edit/reveal/scroll and selection undo.
  A hidden N14 destination clears the transient filter via its revealing restore; visible
  destinations keep it. Dummy exclusions never become fold candidates. Ordinary fold state
  changes only to reveal selection endpoints, preserving unrelated nested/inline folds.
- Native listeners admit focused document views and all-window local/shared composition before
  borrowing Editor; direct model/UI dispatch also refuses local marked text. Read-only,
  Viewer and Follow allow these selection commands. Fields/aligned views refuse visibly.
  Commands are synchronous: no stale result or new background job. Current highlighter
  identity/version validation and ordinary Folds edit mapping supply candidate freshness.
- Acceptance: retain a meaningful compiled causal failure plus exact source/log/actual Running
  binary before repair. Cover nested/sibling/inline and open/collapsed candidates, duplicate
  headers, strict parent/no parent, boundaries/no wrap/EOF, minimal reveal and N14 saved folds,
  source Unicode/tab/wrap/virtual space, multiple carets/primary/shared views, selection undo
  and real source Redo, read-only/Viewer/Follow, native keys/focus/IME and JSON/macro capture.
  Preserve old regression/helper bodies/defaults/benchmarks/ignores/locked graph. Run full
  affected/focused/retained coverage, strict affected all-target Clippy including app, fmt/diff,
  and unchanged serial input/find/fold/scroll/highlight/wrap gates with derived fixture proof.
  Canonical N15 and all Task45 plan boxes remain unchecked for ROOT acceptance.


N15 self-review added a second compiled causal failure for a valid manual range hiding only
its header's newline: it initially behaved like an inline span and excluded the visible header.
Exact source/log/actual Running executable were retained in an independent COW archive before
repair. Multiline detection now uses the range end's physical line, matching Folds' joined-line
semantics even when the end is the next line's start. Native/JSON/macro, local/shared IME,
Viewer/Follow/read-only, N14 saved-fold reveal and selection/source history cases pass in the
preliminary focused run. The final source/runner pin, complete checks and all fourteen serial
release gates will supply immutable actual evidence; acceptance remains unchecked for ROOT.


N15 V2 intentionally repairs one product issue found after V1 pin: normalizing different raw
multiline fold ranges to one header/end erased their strict parent. All V1 ordinary checks,
fourteen serial gates and actual fixture readback finished without interruption, then all
1,785 V1 source/runner/log/artifact mappings and eleven actual Running executables were retained
and independently read back before adding the new regression. That compiled assertion failed
with None versus Some(0); exact failing source/log/actual binary were retained before repair.
Traversal now keeps raw char-range identity/containment separately from presentation headers
and selected spans. Only exact raw duplicates collapse; parent remains strictly enclosing even
when its header destination is shared. The V2 pin and full repeated coverage/same fourteen
unchanged gates will cover this intentional source repair; canonical/plan remain unchecked.


### N15 review fix I1 — behavior contract before repair

Native fold traversal must settle the existing N14 focus after catching up shared edits, before capturing current selections and resolving source destinations. A peer edit and native action in one event must preserve the exact next-fold offset or current-fold span; hidden destinations clear narrowing, while visible destinations retain it. This refresh follows existing synchronous presentation and stale-result admission, changes no source bytes/save point/history, and introduces no option or asynchronous navigation. Acceptance: the actual installed native workspace route dispatches Move, Select to Next and Select Fold immediately after a shared-view Unicode append without an intervening frame. The new compiled causal test failed with caret4 instead of5; exact source, runners, raw log and actual executable were retained before repair in root-task45-native-causal-red-v3 (451 mappings). Full prior N15/N14/folding/source-undo coverage and unchanged serial performance gates remain required.


### N15 accepted — 2026-11-01

Seven catalog entries expose fold header motions, anchor-extending variants and current-fold selection. Traversal retains raw range identity for strict containment while deriving source header/selection spans separately, using ordinary manual/inline folds and valid syntax or indentation candidates. It computes physical targets before N14 restore and minimally reveals them, preserving text, source history, primary/virtual selection semantics and native IME admission. All 42 final V3 phases passed (3,010 tests/29 prior ignores, focused2+9, strict6, actual selector audit and 14 serial release gates). ROOT corroborated 5,376 mappings/29 tools/11 current COW executables/report9 plus four compiled causal states, exact V1 pre-repair and reviewed V2 source/binary archives; one old-log index omission explicitly qualified. Native I1 ordering repair reproduced before fix and approved in scoped re-review; exact source targets survive shared edits before a frame. Earlier assertions/UI/old runners retained; no false concat-workload change claim. Scoped spec/quality review approved. Newly observed debug UI compact-unwind warning remains documented with concrete ROOT final workspace-debug/optimized-application follow-up; no suppression or profile/coverage change. Fields/aligned comparisons refuse; no dependency/default/schema change. N15 is the 45th of 64 selected entries; final combined checks remain.


### N20 — behavior contract before implementation

- Defaults and entry: Goto > Bookmarks > Find Bookmark Across Open Files… and the command
  palette expose `workspace::GotoBookmark`, also available to JSON/custom bindings. No new
  default key or setting. Existing local F2/Shift-F2, view-owned marks and session persistence
  remain unchanged. The picker enumerates every open window, including untitled documents;
  one row per live document/physical bookmarked line retains a valid bookmarked owning view.
- Rows show filename, one-based source line and a bounded Unicode source preview. Filename
  and preview both filter through the existing picker. Highlighting a row does not mutate a
  source view. Explicit confirmation catches up and validates document identity, version,
  line and bookmark membership, then reveals and activates that existing owning view.
- Stale/failure: text/bookmark/tab changes refresh the live list; confirmation also refreshes
  synchronously. Changed text or query invalidates a previously displayed confirmation. A
  closed preferred owner may fall back to another live bookmarked view; no owner means no
  jump, retained entity or disk reopen. All-window shared composition admission precedes
  borrowing the destination Editor, and deferred cross-window activation revalidates there.
- Acceptance: compiled causal RED with exact source/log/actual executable retained before
  repair; two files, shared views/another window, Unicode filename/text filters, bounded
  preview, edits/deletion mapping, changed query, closed preferred/all owners, N14 narrowing
  and folds, Viewer/Follow/read-only, IME and command reachability. Full affected/focused and
  retained navigation checks, strict all-target Clippy including app, unchanged serial typing,
  complete find/UI10M and relevant picker/index gates, immutable input/main/report seals.
  Canonical N20 and all three Task46 plan checkboxes remain unchecked for ROOT review.

N20 preliminary verification passes all 12 focused cases, including timed live refresh,
shared-view marks across windows, stale text/query/removal, owner fallback and deferred close,
Unicode/bounded previews, native menu/custom binding/JSON entry, IME refusal, source Redo and
selection undo, and N14/ordinary-fold reveal in read-only/Viewer/Follow views. The compiled
missing-command RED retains exact source/log/actual executable in task-46-causal-red-v1. A
separate prepin 8/9 diagnostic marked the short EOF line instead of the long preview fixture;
its raw failure remains, and the setup now explicitly marks line 1. Strict prepin also found
five unnecessary mutable test-context references; those are corrected without suppression.
The final immutable pin/check/main/report chain follows. Only new-kind picker branches and a
dedicated source-navigation method are added; existing renderer/fold/index implementations
remain unchanged. Exact serial typing, complete find (including fuzzy20k/goto100k), and UI10M
are the relevant unchanged gates; derive current concat fixtures from actual included owners.
Canonical N20 and all three Task46 plan boxes remain unchecked for ROOT acceptance.


### N20 review I1 — origin-focus contract before repair

After a validated bookmark confirmation disposes its picker, restore the originating window's active editor focus before dispatching local/deferred remote navigation. This mirrors ordinary QuickPanel confirmation: returning after a successful remote jump must accept typing immediately, and failed deferred delivery (closed/stale/composing destination) must leave the origin usable. Invalid or composition-refused confirmation retains its picker/current focus; admission order stays unchanged. No source text/history/default/persistence behavior changes. Acceptance uses two real GPUI-window cases: close the remote owner after confirmation before delivery, then focus/Unicode typing in origin; and complete a remote jump, reactivate origin, then type without an extra click while remote selection/text remain intact. New compiled causal regression failed on actual missing source focus; exact435-map source/runners/log/binary with original+retained inode/device metadata preserved before repair. ReviewedV1 all1479maps/report9/currentbinaries retained first; fresh full checks/same3serial gates/scoped re-review required.


### N20 accepted — 2026-11-01

All-open-window Find Bookmark picker (`workspace::GotoBookmark`) uses ID-only rows, existing view-owned marks/edit mapping, document+line dedup, bounded Unicode preview, query/timer refresh and synchronous stale confirmation. Closed owners fall back only to another valid bookmarked owner; cross-window deferred navigation revalidates source/IME before Editor borrow. Existing local commands/persistence, readonly/ViewerFollow navigation, exact N14/ordinary fold reveal, selection Undo and real source Redo remain. No default key/option/dependency/global store/schema. Actual full3,024 tests/29 established ignores, focused14, strict6APPalltargets, actualselectors,3570oldhelpers,31ordinary+3exactserial+actualfixture35phases passed. ROOT corroborated 2,555+29 mappings/11currentCOW/originalcausal424/newfocus435/reviewedV11479/report9, fresh scoped I1 spec/quality re-review approved; origin focus restoration precedes remote dispatch and two new actual typing cases pass. Original causal numeric inode unavailable explicitly qualified; current final copies have complete inode proof. N15 debug compact-unwind and older dependency notices retained, not suppressed or called repaired. N20 is the46thof64selected; final combined native/workspace/releaseAPP/global checks remain.


### N21 — behavior contract before implementation

- Goto menu and palette expose Open Selected Path/Address and Open Selected URL; both are
  JSON/custom-bindable workspace actions with no default key, setting or persistence. Exactly
  one nonempty physical selection is required; empty, virtual-only and competing selections
  refuse visibly. Selecting, copying and ordinary mouse behavior remain unchanged.
- Preserve complete spaces/Unicode/extensionless filenames and literal terminal punctuation.
  Optional matching single/double quotes and positive one-based :line[:column] are declarative;
  unmatched quotes, zero/overflow/malformed numeric locations and unsupported schemes refuse.
  Relative paths use the named source parent, or exactly one project root for an untitled
  source. Missing/nonregular targets, no base and ambiguous roots never create a buffer.
  Local file URLs reuse the existing terminal decoder; only Open Selected URL launches
  HTTP(S), synchronously through GPUI's existing external URL API. No shell execution.
- File reads pin the originating live document/version/full selection/selection generation,
  source path, roots, active view, focus and navigation/project generations. Revalidate before
  applying a loaded target; newer selections/edits/tab/project/focus/IME/cancelled navigation
  discard late work. All-window composition admission precedes borrowing source/destination.
  Reuse canonical aliases, live document ownership, remembered F01 codec and source clamping;
  catch up destination before computing exact coordinates and minimally reveal N14/folds.
  Failure preserves source text, selections, save identity, history and usable origin focus.
- Acceptance covers relative/absolute/quoted/Unicode/spaces/extensionless/line+column/clamp,
  untitled/no-base/multiple roots/missing/invalid selections and locations, alias/live-owner/
  remembered codec, IME and stale asynchronous races, explicit-only recorded HTTP(S), native
  menu/palette/custom JSON reachability and retained result/terminal links and copy/navigation.
  Retain compiled causal RED source/log/actual executable before repair. Full affected/focused
  checks, strict all-target app Clippy, fmt/diff/preservation and exact serial typing/complete
  find/UI10M gates follow immutable pin and actual fixture derivation/readback. Canonical N21
  and all three Task47 plan boxes remain unchecked for ROOT review.


N21 preliminary checks pass all 15 focused cases, including actual HTTP(S) TestPlatform
records, native/custom JSON reachability, nine stale-source scenarios, post-read disk
change/deletion, dirty live alias reuse after await, shared destination composition, remembered
Windows-1252, exact N14 source reveal with target Redo, and actual typing after large-file
Cancel or composition-refused delivery. The original missing-action causal RED retains
427 exact source/runner/log/actual-executable mappings with numeric original+retained inode/
device metadata. Prepin compile diagnostics (test-only API/schema assumptions) and one
strict unnecessary-lazy-evaluation diagnostic remain separate from that causal failure; no
suppressions or coverage reductions. Current Snapshot47 has 3,585 old literal helpers.
Only cold explicit command/parser/loading branches and a dedicated exact-source reveal
method changed; ordinary mouse/result/terminal detection, renderer, search and project index
remain byte-identical. Relevant unchanged serial gates are typing, COMPLETE find including
fuzzy20k/goto100k, and UI10M; derive the actual current concat/100000-line/big-file bytes
before immutable pin, and read back the actual emitted file after all gates. Canonical N21
and all three Task47 plan boxes remain unchecked pending ROOT review.

N21 review repair contract (I1, before repair): an existing selected path resolves by the same priority as ordinary opening: active-group view, first other local view, then the registry's most recently indexed live document. Admit composition on that exact document across all windows before attaching or focusing any destination. Same-path Save As collisions remain supported. Refusal retains the source tab, focus, selection, history and both owners' text/marked state; an unrelated composing same-path document must not block an already-local clear owner. Acceptance adds real Save As collision GPUI actions for both registry-owner refusal with origin typing/Undo and local-owner priority; all previous N21 assertions and default bindings remain unchanged.
The I1 repair also refuses a busy registry-only target immediately: ordinary opening could otherwise wait for a writer and adopt another owner after the admission snapshot. Already-local views need no disk read and remain navigable during a reservation. The two regression routes cover both busy behaviors and prove that dropping the reservation does not deliver a refused action later. Initial regression setup was corrected to finish window activation before invoking the action, because activation invalidates pending navigation by design; both initial timeout observations and the later direct wrong-active-tab failure are preserved with their exact source and binaries.


### N21 accepted — 2026-11-01

Two explicit selected-path/address and HTTP(S)-URL commands share the existing catalog/menu/palette/JSON/keymap. Physical one-selection parsing preserves full filenames/quotes/Unicode/punctuation, strict locations/bases/refusals; the cold async orchestrator reuses canonical ownership/Loaded decoding/hints/write leases/attachment and validates source/context/target before delivery. Live dirty aliases are reconciled after await, source/target IME precedes mutable Editor use, exact locations precede N14 restore, and errors/large-file Cancel retain origin typing/history. All3041 tests/29 prior ignores/focus17/strictALL10alltargetsAPP/3585oldhelpers/43phases3serial/actualfixture/report9 passed. ROOT independent V4 proof corroborated 8,136+32 mappings/14currentCOW/causal427/1316V1+2673V2archives; fresh spec/quality review approved. V1/V2/V3 products identical; V4 fixes actual same-path destination admission with two additional action regressions; retained selection uses same cases serial plus explicit warmup, full parallel UI unchanged. Earlier real T06 history/teardown failure and parser readiness observation NOT repaired by passing repeats; REQUIRED separate ROOT follow-ups after this commit BEFORETask48N23. No new dependency/default/schema/mouse behavior. N21 is47thof64selected; final combined native/workspace/optimized-app/global checks remain.

### T06 follow-up contract — private launch resources and shell history (before edit)

Shell integration remains opt-in, zsh-only and launch-local, with literal supported arguments and unchanged user startup files/hooks/ZDOTDIR behavior. Its private directory must live until the PTY worker has terminated and reaped the child, then be removed after normal exit, close or Drop even while a view retains review output. Close remains asynchronous and idempotent; spawn failures clean up immediately and cleanup errors are reported. Correct only the system history default redirected through Orlo's private ZDOTDIR: restore its logical user directory before reading user .zshrc, preserving custom HISTFILE choices. No user configuration file or real user history is test data.

Acceptance: deterministic ordinary-versus-integrated HISTFILE comparisons in disposable Unicode/space HOME fixtures with unset/explicit/profile-changed ZDOTDIR and custom history; persisted history on normal exit; automatic private cleanup after child exit while TerminalSession/review remains alive; observed bounded child-reap/cleanup synchronization for existing close/Drop assertions, repeated close, failed spawn, unsupported/norcs flags and existing hooks/status/interrupt tests. Retain the original failed teardown evidence and exact new compiled causal source/log/binary before repair. Run full terminal and parallel UI suites, strict affected all-target checks, formatting and diff checks; no rendering/input/search/index hot path is being changed. Update user/architecture notes and the accepted T06 evidence only after verification/review. This repair does not increment the selected-feature count and precedes Task48.
The first repaired run passed cleanup/lifetime and the unset-directory history case, then exposed a native baseline limitation: installed macOS zsh 5.9 corrupts inherited Unicode ZDOTDIR bytes during plain interactive startup (also reproduced without Orlo across C, C.UTF-8 and en_US.UTF-8; raw hex retained). The fixture now uses an ASCII inherited profile directory with Unicode HOME for ordinary comparisons, retains a profile-assigned Unicode directory case, and adds a fifth direct explicit-Unicode-directory expectation. No lossy decoding is used as a test oracle. Both initial causal failures and this separate first-repair observation retain exact source/log/binary/fixture copies. The history guard also treats an unset HISTFILE safely and preserves readonly/custom nondefault values.
The first full parallel UI run retained one failure in its existing profile/close test: it asserted private-directory absence synchronously after Close Terminal Tab. The complete pinned attempt (all products/runners/actual terminal+UI binaries/logs and report9 presence/absence) was preserved before editing. The test fixture now reports a complete PID, verifies it is alive before close, and observes PID reaping plus directory absence before the same original cleanup and sibling-live assertions. Production close remains nonblocking; no assertion or suite parallelism was removed. This is the eighth intended file in the follow-up.


### T06 history/lifetime follow-up accepted — 2026-11-01

Actual final9checks/1640fullpassed26existingignored/focus10/native5historycases/PTY6/terminalUI62/strictworkspaceALLtargetsAPP/fmt/diff/preservation, actualmain2226+15/current2COW/report9 and firstROOTcorroboration passed; fresh scoped SpecPASS/QualityApproved. The existing PTY worker owns launch resources until child reaping, with asynchronous close/Drop and retained review; the proxy restores only its private default history target, preserving custom history/config bytes. Two causal native regressions and all430+429+1729retained mappings explain actual failures and UI readiness repair. All17runners/source395 remain stable through metadata, all3606oldhelper/assertion checks preserved with explicit test-only normalization and9originalUIassertions. No new dependency/default/schema/hot input/render/search/index code. Selected acceptance remains47/64; this repairs acceptedT06. Required parser readiness follow-up still precedes Task48/N23; final combined native/workspace/release/global gates remain.

### Parser warmup readiness follow-up contract — before edit

Background-open warmup is best effort: the existing worker may return partial work when the unchanged 10 ms parse slice ends, urgent work arrives, or cancellation fires. Attachment promises a usable editor, not a complete syntax/folding snapshot. The existing explicit syntax-selection action handles a cold current tree by scheduling ordinary highlighting and asking for retry without moving selections. Defaults, commands, cancellation, stale-result guards, parser identity, undo and all production logic remain unchanged.

This follow-up corrects the test readiness assumption; it does not force unbounded urgent parsing or claim an exact predicate for the historical uninstrumented failure. First add a deterministic readiness proof against unchanged production: queue urgent work behind file-open-style warmup, prove queries remain cold, then use ordinary cooperative retry and adoption to obtain the exact Unicode range/folds while preserving the receiver identity. Keep the existing warm-tree reuse proof. Only after that proof, make the background-open UI regression observe both current named-range and folding readiness through the existing normal background job before all original exact assertions. No fixed delay, assertion removal, ignored test, budget change or full-suite serialization.

Acceptance: deterministic interrupted warmup/normal retry and existing detected-warm adoption; full parallel syntax/UI and retained syntax-selection/shared-view/stale/undo routes; strict workspace all-target Clippy, fmt/diff and preservation. Verify production portions and benchmark include bytes are unchanged; no performance gate is claimed for a test/readiness/documentation-only change, and the final documented global release/performance suite remains mandatory. Document the contract, retain the original None observation and exact new proof source/log/binary, then obtain scoped independent review before updating N16 evidence and committing. Selected count stays47/64; no Task48/Snapshot48 yet.


### N16 parser readiness follow-up accepted — 2026-11-01

Best-effort warmup/current-query readiness is now deterministic coverage; normal cooperative retry retains exact Unicode/fold results and parser identity. UI background-open regression waits for both current queries through existing normal work, preserving every original assertion; production prefix/runtime files/defaults/10ms/cancellation unchanged. Actual9checks/1758fullpassed28oldignores/focus5+16/exact1/urgent4/strictworkspacealltargetsAPP/fmt/diff/preservation/fixturebytes, main3980+16/current3COW/report9/FIRSTROOTproof/freshSpecPASSQualityApproved. Readiness427/historical2673 exactsource/binaries preserved; no retrospective failed-moment predicate or production parser repair claim. All3611old helper/assertion fragments preserved;41939510+fixture1 stable. Selected47/64unchanged. Both mandatory pre-Task48 follow-ups are now verified; after this authorized commit capture Snapshot48ONCE and proceed to N23. Final global/native/release/performance and broad final review remain.


### Task 48 / N23 pre-edit behavior contract (2026-11-01)

Defaults: viewer protection is a transient, default-off flag on the actual shared Document; fresh/reopened/session-restored documents do not inherit it. Disk read_only, immutable input_enabled and Follow remain independent. Entry: an explicit default-unbound workspace catalog action, native menu/palette/JSON/custom keys and persistent visible viewer indicator. Central model mutation admission combines only Follow and viewer, preserving generic selection/navigation/copy APIs and the working-copy semantics of read_only.

Transitions refuse while any view of this exact document has native composition, preserving marked text and showing a useful message. Accepted transitions end typing coalescing and publish version-only changes without touching dirty/saved identity, rope, selections or undo/redo. This invalidates asynchronous captures even across an on/off round trip. All targets are admitted before any multi-document edit. Same-path distinct documents do not share the flag or composition admission.

Viewer blocks source changes through typing/paste/deletion/IME/undo/redo/transforms/replace/pipelines/LSP, reload and serialization metadata changes. Read/navigation/selection/copy/filter presentation remain available. Explicit saving of already dirty text remains allowed under existing disk checks; both formatter and cleanup/encoding/BOM policies skip mutation while protected. Close/save decisions continue using the existing dirty/saved identity. Follow ingestion remains a trusted separate path but must not bypass viewer protection.

Acceptance: dirty Unicode buffer across shared views resists actual actions/native input/IME and model APIs, retains dirty text/history and supports reading; toggle-off restores editing only under existing protections. Exercise composition in another window and same-path distinct owner, on/off stale captures, atomic multi-target LSP refusal, pipeline result admission, explicit dirty save with format/cleanup configured, native menu/palette/key/JSON reachability, visible mode and transient reopen/restore. Retain all old assertions and unchanged serial typing/find/render budgets.

N23 visibility decision: native window title and rendered tab labels also show `[VIEWER]`, so hiding the status bar does not hide the mode. The native menu keeps a static Toggle Viewer Mode action; it has no installed dynamic label/check state to become stale. Workspace owns toggle admission and never borrows an Editor while mutating its document.


### N23 accepted — 2026-11-01

Document-wide transient default-off viewer protection now shares central source admission across all views, with explicit catalog/menu/palette/JSON/custom-key toggle, status Unlock and hidden-bar title/tab indicators. Composition refusal uses actual document identity; version-only transitions preserve dirty/saved/history state and invalidate old async captures. Reading/selection/copy and explicit protected dirty saves remain; formatter/cleanup/serialization mutation skips and all-target LSP/pipeline/replacement admission preserve protections. Actual full3056passed29oldignored/focus12/strictworkspaceALLtargetsAPP/29phases13serial/actualfixture/preservation3612/report9 and FIRSTROOT4612+28/current10COW proof passed; fresh SpecPASS/QualityApproved, no Critical/Important. All419/395/15+fixture1 stable through acceptance. Eight exact pre-repair/setup archives retained, two precisely qualified production-function scanner substitutions preserve all old assertions. No new dependency/default binding/persistence. Selected acceptance48/64; final physical/native/workspace/optimized-app/global gates and broad review remain.


### Task49 / N25 — mixed line endings

The behavior/architecture contract was recorded before product edits and clarified during
implementation. Disk-only decoding recognizes CRLF, LF and every lone CR before normalized
LF source is built. One l/w/r ASCII kind per LF boundary lives in a second shared Rope;
ordinary non-newline typing leaves it unchanged and history/save/copy snapshots share it.
Minimal edits remove only deleted boundaries; inserted LF uses the preferred ending.
Existing boundaries survive line edits, case conversion, sorting, joins/splits, shared Undo,
encoding/BOM conversion and immutable pending saves/copies. No-final-newline remains absent.

Preferred ending keeps the existing majority/tie rule, then existing repository/explicit
precedence. Automatic preferences never flatten a present map. Document::from_text remains
the explicit legacy logical-text API; new/runtime source documents use from_source/uniform
maps and physical files use decoded maps. Old recovery absence materializes a uniform map
from its serialized legacy default, preserving initial bytes and later boundary provenance.
Present invalid metadata fails admission locally, retaining the damaged session privately and
keeping valid peers. Maps validate kind/count against draft source and are bounded to 100 MiB.

SetLineEnding now explicitly converts every boundary, including literal CR/CRLF introduced
by arbitrary text operations. Actual conversion is one dirty shared Undo step, including
metadata-only conversion with identical logical source. Choosing an already uniform current
ending records explicit preference provenance as one Undo step/version-only notification,
without changing rope, map, SavedPoint or dirty state; repeated already-explicit choice is a
true no-op. Undo/Redo restore the exact corresponding preference/override/map and source.
Viewer/Follow, immutable-view and actual shared-document native composition admission apply;
inspection/navigation remain available. The JSON/custom-key route validates LF/CRLF/CR.

Inspect Line Endings shows physical LF/CRLF/CR counts, final-newline state and wrapping source
navigation. Statistics/checksums use physical serialization. Closed-file search/replacement,
pipelines, comparison, Revert and Follow share file-aware normalization/maps; ordinary
clipboard/transform normalization retains its prior literal-CR rules. File History is text-only:
surviving destination boundaries remain and inserted ones use its preference. F07 rescue is
Restore as New Untitled with a new default uniform map, not original format recovery.

Validation: 24 focused regressions pass (15 editor, 1 project, 7 UI, 1 workspace), including
31 supported codec/BOM combinations, exact mixed bytes, new untitled/preference changes,
copy/save races, literal CR conversion, broad transforms, recovery corruption, history,
comparison, protected routes and real custom native keys/shared IME. The earlier affected
8-package run passed 2971 tests/29 original ignores; final affected all-target strict/fmt/diff
and nine applicable exact serial release gates passed. Cold same-ending provenance repair
retains the unchanged-hot-path gate results. Existing assertions and budgets remain.
Independent review found two history interactions; causal regressions now cover peer typing across a provenance choice and explicit conversion Redo after automatic preference/save. Both fixes passed scoped re-review. N25 is accepted; workspace checkpoint Task51 and all final native/release/global gates remain.

### Task50 / N28 implementation contract (before product edits, 2026-11-01)

Defaults: per-window transient explorer history, at most 32 total folder locations including
current; no persistence/settings/dependencies. Existing multi-root tree, cached rows/listings,
expanded state and editor jump history keep their owners. Navigation selects/reveals folders
without collapsing other branches or changing roots. Initial browsing seeds the applicable root;
Back/Forward skip missing or replaced directory identities, a new visit truncates forward history,
and adjacent duplicate visits do not consume slots. Known Orlo moves remap recorded paths.
Parent uses the current folder; at an outer project root it visibly refuses and retains roots.

Entry: sidebar folder activation, Back/Forward/Parent controls and actions, menu/palette/JSON/custom
keys; active-file Path Breadcrumbs opens existing Choices widgets for ancestors and their immediate
children. Child files use ordinary opening, folders use ordinary tree reveal. Reads/identity checks
and enumeration run on existing background executors, never in paint. Source/version/path,
project/navigation request and picker/focus context guard delivery and child activation; stale
work does nothing, missing/replaced targets report a useful refusal. Reading/navigation stays
available in viewer mode; no source edit or IME mutation is introduced.

Acceptance: real nested/two-root browsing, Back/Forward/Parent, branch/bound32/duplicates,
deleted/moved/replaced history, ancestor-child opening, stale delivery, preserved roots/expansion/
dirty source/editor jump history, and actual GPUI keyboard/focus plus catalog/menu/palette/JSON/
custom-key reachability. Focused affected-package tests and strict all-target checks, fmt/diff;
existing performance gates only if an actual hot path changes. Canonical N28 remains unchecked
for ROOT review and acceptance.


Task50 / N28 implementation and focused verification finished (2026-11-01), pending ROOT review.
Explorer location history and identity checks extend project/explorer and the existing window
SidebarState; Back/Forward/Parent preserve multi-root expansion and editor jumps. The active-file
ancestor/child Choices menu shares one source context, reads/validates on workers and reuses normal
open/tree reveal. Defaults remain transient, bounded at 32 total locations, with no new settings,
dependencies or source mutations. README/action IDs and architecture notes are updated.

Actual results: four new backend and nine new GPUI regressions pass; complete project backend
94/94, connected fileops10/roots6/goto3, affected project/UI/app strict all-target Clippy, fmt and
diff checks pass. Existing serial release UI keystroke gate: 40.543 ms against unchanged 300 ms.
The final cold source-context sharing change was followed by nine GPUI regressions, strict checks
and fmt; it did not change the render/input/poll paths measured by that gate. Causal missing-action
and two history-edge failures plus the two corrected compiler errors remain in the concise report:
`.superpowers/sdd/editor-expanded-features-plan/task-50-report.md`. Canonical N28 stays unchecked;
ROOT owns independent review/commit and the next full-workspace checkpoint after Task51.

Task50 / N28 review I2 repair contract (before repair, 2026-11-01): breadcrumb file activation
must retain its captured file identity/kind and source/picker/focus request through ordinary-open
admission, background reads and large-file prompts. An active write lease may refuse with a retry
message instead of waiting. Missing, replaced, directory or stale-context targets must leave the
source, roots and picker intact; ordinary unguarded missing-file/directory opening keeps its rules.
Reuse existing loading/attachment/codec/live-document ownership; add causal post-validation lease
races, deferred-read/prompt and source/focus regressions. No hot rendering/input/index work changes.

Task50 / N28 I2 repair verification (2026-11-01): breadcrumb file identity now survives ordinary
opening, background reads and large prompts in a concrete guard; busy write leases refuse/retry.
The existing loader checks the opened file handle's identity/kind and guarded read stamps, then
source/surface/focus/lease checks precede attachment. Errors retain the source/picker, while live
dirty owners, codec hints, native composition and ordinary missing-file/directory rules remain.
Actual causal 0/3 failures and initial 13-pass/1-fail foreground-focus regression are retained in
the same report; final N28 19GPUI+5backend, project95, focusedIO2, ordinary-opener/reservation cases,
selected-target16, roots6/fileops10/goto3, strict affected all-targets, fmt/diff pass. No hot UI path
changed and no unchanged performance gate was repeated. ROOT's I1 history fixes remain intact;
canonical N28 stays unchecked for one scoped combined I1+I2 re-review. Details and literal commands:
`.superpowers/sdd/editor-expanded-features-plan/task-50-report.md`.

N28 accepted after scoped re-review: both Important findings addressed, Spec PASS/Quality APPROVE, no new findings. Root-removal pruning and project reset preserve surviving history; guarded child opening rejects stale identities through read/prompt/lease transitions. Actual final results above are retained; full workspace checkpoint follows N29/Task51 and final native/global gates remain.


### Task51 / N29 behavior contract (before product edits, 2026-11-01)

Global presentation settings `explorer_sort` (`"lexical"` default, `"natural"` opt-in)
and `explorer_sticky_ancestors` (false default) belong to config's application settings,
not file scopes or document/session state. Natural order keeps folders first, compares
ASCII digit runs by significant length and digits without integer parsing, and resolves
numeric equality by fewer leading zeros, then case-insensitive/name-byte ties. Existing
lexical lowercase order remains the default. Changing sort reorders existing cached listings;
new/refresh/location listings use the selected policy without new foreground filesystem work.

Sticky ancestors decorate the existing uniform list using the actual current viewport and
cached parent row indices. Only ancestors above the first visible tree row are pinned; the
nearest ancestors fit when the viewport cannot hold the full chain, always leaving at least
one content row. OPEN FILES/headings and multi-root boundaries remain in the existing list.
Pinned rows retain their real root/path identity, selection, folder-toggle/context/focus and
mark actions through the shared row renderer. They do not enter the flattened row list or
change logical indices; keyboard/reveal accounts for their occlusion, and the list's current
layout coordinates govern painting and pointer hit testing through scroll/resize.

Acceptance: causal regression for missing settings and sticky UI, natural numeric/huge/case/
zero/folder ordering and refresh, cache invalidation, multi-root/deep viewport limits, real
GPUI pointer/context/keyboard/focus, resize and reveal. Preserve N28 history/identity guards,
all existing assertions and serial performance budgets. No dependencies or replacement tree.
ROOT owns independent review, canonical acceptance and the next workspace checkpoint.


Task51 / N29 implementation and focused verification finished (2026-11-01), pending ROOT review.
Global Settings/template exposes `explorer_sort` (`lexical` default / `natural`) and independent
`explorer_sticky_ancestors` (false default); project/scoped overrides reject both. Cached tree
listings use folder-first arbitrary-length ASCII numeric order, deterministic zero/case ties,
and cached parent indices for current-viewport pinned ancestor rows. Shared row actions, native
wheel passthrough, root identity and keyboard reveal retain the existing owners and N28 history.

Actual results: config79/project97; explorer13 including four new GPUI regressions; N28 nineteen,
fileops10/roots6/preferences22 with one original modeline gate ignored; affected all-target strict,
fmt/diff pass. Existing serial release UI keystroke gate: 36.909 ms / unchanged 300 ms budget,
optimized compilation 9m12s. Causal failures, literal commands, ownership and self-review are in
`.superpowers/sdd/editor-expanded-features-plan/task-51-report.md`. Physical AppKit/manual checks,
full-workspace checkpoint and independent review remain ROOT's work; canonical N29 stays unchecked.

N29 review repair contract: a reveal can leave the viewport beginning inside a deeper branch than the target. Sticky reveal reserves the viewport's maximum permitted pinned rows, guaranteeing one full content row; ordinary non-sticky reveal keeps its previous behavior. This conservative reservation may place a shallow target lower in the viewport. Reproduce deep-to-shallow keyboard navigation with trailing siblings, verify actual target bounds below the pinned stack and click the requested file. No new coordinate or scrolling engine.

N29 accepted after scoped repair re-review: I1 addressed, Spec PASS/Quality APPROVE, no new findings. Final eight N29/explorer14/N28 19 and affected strict/fmt/diff passed after the cold reveal repair; earlier checkpoint3,114/0/29 and serial UI36.909ms/300ms retained with exact scope. Lexical and sticky-off defaults remain; one content row is reserved beneath the maximum possible pinned stack for explicit reveal. Next workspace checkpoint follows Task55; all final native/global gates remain.

### Task52 / N30 behavior contract (before product edits, 2026-11-01)

Extend the existing Sort Lines by Pattern form and pipeline operation. Defaults stay lexical,
case-sensitive, forward, no header; omitted capture keeps capture 1 when present, otherwise
match 0. Explicit capture is a validated nonnegative group index (0 selects the full match);
a valid optional group that did not participate retains the existing empty-key behavior.
The legacy `numeric` flag keeps finite f64 comparison and lexical fallback exactly; it cannot
combine with a nonlexical `order`. New `order` values are `natural`, `locale`, and `length`.
Natural compares arbitrary-length ASCII integer chunks, with delayed fewer-leading-zero ties;
case folding remains opt-in. Reuse N29's exact algorithm without changing explorer ordering.
Locale uses Foundation with one current-macOS-locale snapshot per sort, including canonical
Unicode equality; length counts extended grapheme clusters of the original key. No dependency.

Stable ties remain in source order even in reverse. Unmatched lines always follow matched
lines in source order. Keep First Selected Line as Header excludes the first line of each
selected line block (the document when none selected); disjoint blocks sort independently,
touching blocks keep existing merging. Reuse existing transformation admission, atomic errors,
Undo/Redo, source selection handling, and N25 per-boundary LF/CRLF/CR identity at each ordinal
separator. Sorting does not normalize physical endings or add/remove the final newline.
Bad patterns/capture indices/options change no source, selections, history or metadata and the
form reports the error. Existing active-editor form target admission remains unchanged; the synchronous command reads
the current text and selections at submission, with no asynchronous result to become stale.

Acceptance regressions: natural v2-part9/10, huge chunks, case/zeros, current locale fixtures
with locale-independent expectations plus controlled native locale fixtures, combining/emoji
length ties, explicit/full/optional captures, invalid/negative captures, header/reverse/stable
unmatched behavior, disjoint selected blocks and one Undo/Redo with exact mixed ending bytes.
Exercise the real form and serialized pipeline; preserve legacy lexical/f64 and all old tests.
ROOT owns independent review, canonical checklist acceptance and workspace checkpoints.

Task52 / N30 implementation and affected verification finished (2026-11-01), pending ROOT review.
Existing SortOpts/form/pipeline now offer natural ASCII integer chunks, current macOS locale and
extended-grapheme length, validated capture indices and per-block header exclusion. Lexical and
finite-f64 numeric defaults/outputs, stable reverse ties, unmatched source order, selected-block
admission, mixed physical boundary bytes and shared Undo/Redo retain their existing owners.
The exact N29 comparator now lives in utils/text; native collation uses the existing Foundation
seam with one retained locale and one NSString per key, including NUL and full Unicode ranges.
No dependencies, replacement engines or new actions/settings were added.

Actual results: 14 N30 regressions; complete editor849/project97/UI1618/utils11 = 2,575 passed,
0 failed, 25 existing UI ignores; affected all-target strict Clippy, fmt and diff checks pass.
Initial missing-option/form failures and two corrected new test fixtures remain in the report:
`.superpowers/sdd/editor-expanded-features-plan/task-52-report.md`. Measured hot methods and
benchmark definitions/budgets/ignored flags were unchanged; no hot-body gate applies. Cold source
fixtures include these transform files, so no assertion of unchanged benchmark input bytes is
made. Independent review, canonical acceptance and final global/native gates remain ROOT's work.

N30 accepted after independent Spec compliant/Quality approved review with no findings. Fourteen regressions and affected suites2,575/0/25 plus strict/fmt/diff passed; native locale ownership, full UTF-16/NUL keys and unchanged N29 helper/legacy sort were reviewed. All source/selection/Undo/ending assertions retained. Next workspace checkpointTask55; final native/global gates remain.

## Expanded phase — N31 behavior contract (2026-11-01)

Before Task53 product edits: local and L01 server snippets share TextMate numeric choice
fields `${1|first,second|}`. Parse escaped commas, pipes and backslashes into owned literal
values; preserve empty/Unicode choices, nested surrounding fields, mirrors and transforms.
The first value is the inserted default. The existing per-view snippet session retains the
choices and supplies the active field's list independently of its file/provider lifetime.
The editor's existing popup keeps keyboard focus in the editor: Up/Down selects, Enter
confirms and keeps the field selected for typing, Escape dismisses only the list, Tab
confirms then visits the next field, and Shift-Tab dismisses then visits the previous field.
Revisiting a choice field opens its list again. Confirm uses the ordinary edit/session path
for every active insertion caret and its mirrors/transforms, in one Undo step; later typed
values and Undo/Redo retain existing snippet behavior.

Typing, native composition, incompatible selections/source/context, protected input, another
view's edit, scrolling, resizing and focus loss close the list. Provider shutdown alone does
not invalidate owned choices. Existing read-only/Follow/viewer, stale completion, shared-view,
IME, virtual-space and field-navigation admission stay with their current owners. Acceptance
covers escaped/malformed/empty values, nested fields, multicaret/mirror/transform/Unicode,
confirm/cancel/Tab/Shift-Tab, typed replacement, Undo/Redo, actual GPUI focus/keyboard routing,
local and L01 insertion, source/view/IME/protected closure and provider-independent ownership.
Run affected editor/UI suites, strict affected all-target Clippy, fmt/diff and unchanged serial
release typing, find and UI keystroke gates; no benchmark definition or budget changes.

N31 implementation evidence: 10 focused regressions cover local parsing, owned choices,
keyboard confirmation/cancellation/navigation, nested fields, multicarets, mirrors/transforms,
Unicode/empty values, Undo/Redo, IME, shared edits, protected/stale context closure and hidden
tab blur. Real fixture Format2 completion/resolve/additional edits and shared history retain
all alternatives after actual provider shutdown. Initial local-model/UI regressions and the
hidden-blur regression were observed RED before their fixes. Full affected editor/UI suites
passed 854 + 1,623 tests, with 25 existing UI ignores; strict affected all-target Clippy,
formatting and diff checks passed. Existing exact serial release typing/find/UI-keystroke
gates passed unchanged budgets (1,000-caret typing 1.594/20 ms, Undo/Redo 15.752/30 ms;
UI find-panel keystroke 36.524/300 ms). Embedded source fixture bytes change with snippet
command/tests; benchmark definitions, counts, budgets and ignore flags do not. Actual commands,
results, logs, self-review and limits: `.superpowers/sdd/editor-expanded-features-plan/task-53-report.md`.
Canonical N31 remains unchecked for ROOT's independent review and acceptance; no worker commit.

N31 review repair contract: an admitted completion/signature popup dismisses the active choice list while retaining ordinary snippet field navigation; provider-only cleanup keeps choices alive. Multiline choices anchor at the visible primary selection head, so an offscreen field start cannot leave invisible key interception. Causal GPUI regressions cover Ctrl-Space/server-row keys and commit, explicit signature overload keys, later field navigation, and a100-line default whose head is visible while its start is offscreen. No new popup or coordinate engine.

N31 accepted after both UI review repairs: final13N31/popup28/signature32/strictfmt/diff passed, scopedSpecMeets/QualityApprove/no findings, and post-repair serial UI10M35.864ms/300ms passed. Prior model typing/find gates retained; fullaffected2477/25 preceded the narrowUIrepair, explicitly documented. NativephysicalAppKit/OSIME and finalglobalgates remain; next workspacecheckpoint55.

## Task 54 / N32 — implementation contract (before edits)

- Add **Insert Date/Time…** to the existing Edit menu and palette, opening the existing form
  with local time selected and `%Y-%m-%dT%H:%M:%S%:z` as its ISO default. The recordable
  editor action validates only `%Y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%z`, `%:z`, `%%`;
  malformed directives fail before any edit. Literal Unicode/percent escaping is preserved.
- Capture one wall-clock instant per complete expansion/insertion. Native local conversion uses
  the offset at that instant; UTC uses zero offset. Existing SystemTime/libc facilities suffice,
  with no dependency, shell, command evaluation, random or UUID support. Tests inject the clock,
  zone conversion and clipboard; time failures leave plain insertion unchanged.
- Snapshot new named clock/clipboard/workspace values once at actual local or LSP snippet commit,
  never list discovery. Local date components are padded; Unix seconds are decimal. Preserve
  existing per-caret text/file/indent variables, defaults, transforms, owned choices and Undo.
  Empty/missing clipboard and project context resolve to empty strings. Raw clipboard text is data.
- Resolve project values from the current window index roots: deepest lexical containing root,
  matching existing explorer/language ownership; first root for untitled. WORKSPACE_NAME uses
  Root::label (custom label then folder name), WORKSPACE_FOLDER its path. Outside roots have empty
  workspace values and RELATIVE_FILEPATH falls back to filename; untitled relative path is empty.
- The form pins document/version/selections/generation and refuses stale submission. Ordinary
  document/view protections and shared-view IME admission remain; one insert replaces all
  selections in one Undo step. No asynchronous timestamp result or persistent new setting.
- Regressions cover local/UTC/default/offset transitions/escaping/invalid formats, one instant
  across carets, raw/empty clipboard, roots/untitled/outside paths, defaults/transforms/choices,
  actual form/menu/palette/local/LSP commit and normal Undo/protected/stale refusal. Focused
  affected suites and strict checks apply; hot gates only if a measured hot path changes.

N32 accepted after independent Spec compliant/Quality approved review, no findings. Thirteen focused and2,504affected tests passed with25existingignores, final strict/fmt/diff passed after a test-only lint correction. Local/UTC formatter and single commit-time runtime capture retain existing snippet/Undo/source protections; no new dependencies or ordinary hot-path work. Workspacecheckpoint followsTask55; final native/global gates remain.

### Task55 / N33 behavior contract (before product edits, 2026-11-01)

`selection_occurrences` defaults to false and resolves through ordinary global/project/file
preferences. In document views, exactly one nonempty, nonvirtual, single logical line selection
requests a view-local passive overlay. Matching uses the existing literal, case-sensitive,
non-overlapping search and its cancellable background deadline. The selected occurrence itself
is omitted. Unicode offsets remain character offsets; `aa` in `aaa` follows Find's existing
left-to-right non-overlapping rule.

Cache identity includes the actual document, immutable rope/version, exact selection and its
generation. Unsupported/empty/multiple/multiline selections, composition and disabling clear
and cancel promptly; changed source/selection/settings cannot adopt a delayed result, including
away-and-back selections and shared-document edits. Visible paint visits cached ranges through
the existing fold/wrap row mapping, without searching or copying the document per frame.
The overlay never changes Find ranges/cache, five manual styles, text, selections, Undo or
geometry. Read-only/viewer document views remain eligible. No new dependency or search engine.

Acceptance: causal opt-in regression, real Unicode/case/overlap results and visible rendering,
independent views/Find/manual styles, empty/multi/multiline/virtual/IME/disabled clearing,
controlled delayed completions after edits/away-back/shared changes and policy changes,
fold/wrap clipping, actual preferences reachability, affected tests/strict checks and unchanged
applicable serial search/render/highlight/scroll/typing gates. ROOT owns checkpoint55, review
and the sole canonical checklist acceptance.

Task55 bounded-work amendment (before admission edit): automatic queries admit at most 1,024
Unicode scalars (at most 4 KiB UTF-8); larger selections clear and schedule no job, before any
selected text copy. Find's limits and behavior stay unchanged. Existing long-row clipping already
precedes passive iteration. Add a 199,999-retained-match regression at near/far horizontal
viewports, counting binary-search and candidate visits, plus exact query-boundary/oversize proof.

Task55 / N33 implementation and focused verification finished (2026-11-01), pending ROOT review.
Config's opt-in resolves through ordinary file/project scopes. UI owns separate per-view ranges,
cancellable bounded literal search, exact document/rope/version/selection/generation/request
guards, and prompt clearing for unsupported/disabled/composing/stale contexts. Query admission
caps selected text at 1,024 scalars before copying/scheduling. Cached paint visits only clipped
visible source pieces and skips fold markers; Find, five manual styles, source, Undo, selections
and geometry retain their owners. README, user guide, settings template and architecture agree.

Thirteen new regressions include causal missing-setting/paint/query-bound failures, held real
background matcher completions through the production receiver, Unicode/case/non-overlap,
shared views/native input-handler IME/viewer/virtual/fold/wrap/settings behavior, cancellation,
failure caching and 199,999 retained matches with fewer than 1,024 visits per near/far frame.
Actual affected suites passed config80/editor856/UI1644 (2,580 total, 25 existing ignores) before
the final admission amendment; final amended-code focused12, affected all-target strict Clippy,
fmt/diff and all eight exact applicable serial release gates pass. Final UI10M45.083ms/300ms,
folded scroll3.178ms/8ms and comparison scroll2.573ms/10ms. All original workloads, assertions and
budgets remain. No dependencies, commits or canonical checklist changes. Actual commands,
failure logs, results and limits: `.superpowers/sdd/editor-expanded-features-plan/task-55-report.md`.
ROOT owns checkpoint55, independent review/acceptance and remaining native/global/final gates.

N33 accepted after independent Spec compliant/Quality approved review, no findings. Final workspacecheckpoint55 passed3,168/0/29 plusfmt/strictalltargets/diff; final eight serial gates passed with unchanged assertions/thresholds. Defaultoff/scoped,1,024scalar cap and visible-piece cachedpaint are documented; Find/manual/source/geometry state preserved. Nextcheckpoint59; all final native/global gates remain.


## Task56 / N38 — per-selection shell filters and predicates (pre-edit contract)

Extend the existing explicit Tools/palette Filter Through Shell and package `filter:true`
path. Capture actual document/view/version/path, complete selection state and selection
generation, and each physical nonempty character range. Process inputs in source order over
STDIN using the existing `/bin/sh -c` owner; never interpolate selected text into commands.
Ignore ordinary empty carets mixed with physical ranges; reject virtual endpoints, overlapping
ranges, and more than 4096 targets before launching. Preserve the single physical caret's
whole-document fallback only for Filter Through Shell/package filters. Output/API commands
retain their existing one-input contract. No new dependencies or execution framework.

Add Tools/palette Keep Selections Through Shell: exit 0 keeps that original range, exit 1
drops it, any other exit/signal/execution failure rejects the entire batch. It requires
physical nonempty ranges. Retain selection direction/primary where possible; an empty retained
set becomes one valid caret. Predicate completion changes selections through ordinary selection
history, never text or text Undo.

Run one child at a time, with the existing process-group kill/reap, one shared cancellation
flag, a 60-second whole-batch deadline and 4 MiB combined stdout/stderr budget. Gather all
successful UTF-8 replacement outputs before one `replace_ranges`/Undo edit. Failure, invalid
UTF-8, cancellation (including after worker completion), stale document/selection, closed
target, read-only/viewer/Follow/immutable source or native composition in any shared view
refuses without source/selection mutation or a text Undo step. Check admission both before spawn and
completion. Diagnostics identify the failed target and preserve stderr/exit information.

Acceptance: causal multicaret/virtual/away-back regressions; actual disposable subprocesses
for Unicode, source-order/env offsets, literal hostile STDIN, stderr/exit/signal/UTF-8/output
limits, batch timeout and cancellation/reap; UI atomic Undo/shared view, failure/cancel/stale
edit/selection/protection/IME and predicate selection Undo/Redo; native menu/palette/action/form
reachability and legacy one-caret behavior. Run affected config/editor/UI suites and strict
affected all-target Clippy/fmt/diff. No automatic editing/render/search hot path changes;
ROOT owns independent review and the sole unchecked canonical N38 entry.


Task56 / N38 implementation and focused verification finished (2026-11-01), pending ROOT
review. Existing explicit shell filters now capture independent physical ranges and exact
document/view/version/path/selection/generation identity, process them serially through the
existing cancellable kill/reap owner, and commit gathered UTF-8 replacements in one Undo step.
One physical caret retains the whole-document fallback. The new Tools/palette predicate
retains exit-0 selections, removes exit-1 selections, and rejects any other execution failure;
all-dropped results retain one valid caret with ordinary selection Undo/Redo and no text edit.
Pre-prompt/capture/commit guards protect read-only/viewer/Follow/immutable/shared-IME sources.
The batch shares 60 seconds and 4 MiB total stdout/stderr; 4096 selections bound admission.
Virtual endpoints, overlapping ranges and unsupported empty inputs refuse. No new dependency.

Fifteen new focused tests pass, including actual Unicode/literal-STDIN subprocesses, per-range
offsets/order, stderr/exit/signal failures, invalid UTF-8, aggregate limits/deadline, early exits
with 256 KiB unconsumed input, explicit cancellation with shell PID reaping, actual edit during
a held command, late cancellation, protected/shared/native-composition admission, predicate
selection history, package/API compatibility and natural typed-caret whole-document filter
plus one Undo. Full affected config84/editor858/UI1654 pass, with 25 established UI ignores;
that full UI run precedes only the last additional legacy-caret test, and final focused15
covers it. Strict affected all-target Clippy and formatting pass. Final diff verification is
recorded in the report. No hot editing/render/search path or performance workload changed;
no redundant performance gate was run. User guide/architecture, existing action catalog,
Tools menu and palette agree. Canonical N38 remains unchecked for ROOT independent review.
Actual literal commands, causal/failure logs and self-review/limits:
`.superpowers/sdd/editor-expanded-features-plan/task-56-report.md`.

N38 review repair contract: a batch whose shared deadline is exhausted refuses before spawning another shell and preserves its selection-indexed timeout error. The same start instant charges subprocess command/pipe setup and all earlier input work; single-input execution also rejects a zero/expired budget before spawn. A deterministic missing-cwd regression distinguishes timeout admission from an attempted spawn without timing-dependent side effects. Existing positive timeout/cancel/aggregate-output tests remain unchanged.

N38 accepted 2026-11-01 after scoped review: expired admission fixed before spawn; final16focused/fullconfig85/affectedstrict/fmt/diff pass, earlier full editor/UI coverage retained on unchanged code. Independent Spec pass/Quality approved, no findings. No hot-path gate applies.

### Expanded N39 / Task57 behavior contract (before edits)

Sync Terminal Folder is explicit and targets only the existing selected terminal: active file
parent, otherwise the workspace’s first project root. No launch, document-switch autosync,
setting or default shortcut is added. The command is exposed in Tools, the palette, JSON
actions and the terminal header. Existing opt-in supported zsh integration is required.

Every sent input invalidates local readiness. A fresh shell command and ZLE empty-line report
restore it; old OSC133 prompt markers alone never authorize a write. The existing PTY worker
rechecks the real foreground process group immediately before dispatch. A launch-local ZLE
widget rechecks its current BUFFER and queued input before accepting the shell-owner’s quoted
cd command from one bounded private request file. Refusals preserve typed input and review;
unsupported, closed, exited and foreground-job sessions refuse. No async target lookup can
retarget another session. The existing OSC7 owner alone changes cwd; pending requests wait for
the command’s actual reported folder and failures stay visible. Encoded control bytes in
OSC7 are admitted only for the exact pending sync target; ordinary T02 rejection stays intact.
New input, exit, close and failure clear that pending admission. Integration teardown owns the
request file through the existing child reaping lifecycle.

Causal real integrated-PTY tests cover spaces, quotes, newline and Unicode paths, pending
versus reported cwd, stale markers/partial input, foreground jobs, failed cd and exit. GPUI
tests cover active-file/project fallback, selected-session routing and catalog reachability.
Focused terminal/UI/config suites, strict affected all-target Clippy, formatting/diff checks
and the existing terminal snapshot/search gate retain their workloads and budgets. ROOT owns
independent review, the canonical checkbox, later native combined QA and workspace checkpoint59.

### Expanded N39 / Task57 worker evidence — independent review pending

Explicit Sync Terminal Folder now reaches the selected existing session from the header,
Tools, palette and JSON action catalog. Eight new regressions cover five real integrated-PTY
cases, strict versus pending-target OSC7 decoding, and two workspace/catalog paths. Final
affected suites: config85, terminal72+1ignored, UI1657+25ignored (1814passed/26existingignored).
Strict affected all-target Clippy, full formatting, zsh syntax and diff checks passed. The
unchanged serial release terminal snapshot/search gate passed at12.9015ms against250ms.
Actual commands/exits and all failed attempts are retained directly in task-57-*.log; see
.superpowers/sdd/editor-expanded-features-plan/task-57-report.md. The initial T02 malformed
OSC7 failure was repaired with exact pending-target admission, preserving its original
assertions. No canonical checkbox changed; ROOT owns independent review and later combined
native/workspace/release/global verification.

N39 review repair contract: a confirmed nonzero command result ends a failed sync even when strict OSC7 rejects the unchanged control-byte cwd; success still requires a fresh accepted matching cwd report. Foreground refusal revokes the private request before reporting failure so a later empty-prompt widget invocation cannot replay it. Add causal integrated-PTY combinations for both; preserve ordinary OSC7/clickable decoding and all prior assertions.

N39 accepted 2026-11-01: explicit selected-session folder sync with opt-in zsh, fresh empty-line/foreground/live-ZLE admission, input invalidation, consumed private request and real OSC7 confirmation. Causal repairs cover failed cd from a newline cwd and no replay after foreground refusal. Final focused10/fullterminal74/affectedstrict/fmt/diff pass; original affected1,814 and unchanged terminal gate12.9015ms/250ms retained. Independent scoped Spec/Quality pass, no open findings. Conservative readiness stays disabled after unconfirmed input until a fresh command completes; no autosync/default key/dependency added.

N40 review verification decision: cancellation also runs on ordinary document edits. Run the existing exact serial UI keystroke/10M-find gate with its unchanged300ms budget before acceptance; rendering/ranking/index/search-model hot bodies remain unchanged. Independent code review found no functional defect.

N40 accepted 2026-11-01: independent Spec meets/Quality code approval, final15focused/178language-owner and affected package/strict/fmt/diff pass. The required exact serial UI typing gate passed43.934ms/300ms, resolving R1 without changing reviewed code or thresholds. New destinations preserve semantic-only routing, picker/read-ticket freshness and range validation before visible navigation. No dependencies, settings or default keys added. Next N41 and workspacecheckpoint59.

### Task 59 / N41 — implementation contract (2026-11-01)

`inline_diagnostics` defaults false; file/project/language preferences may enable it.
`inline_diagnostics_severity` defaults `warning` and accepts `error`, `warning`,
`information`, `hint` (inclusive threshold). Reuse the current admitted diagnostic list:
cache one winner per physical start line when that list/source is installed, ordered by
severity, source offset, then original report order. Paint a muted, single-line, elided
message after that line's visible end, on its last soft-wrap row only. Hidden line ends,
filler rows, insufficient trailing width and IME composition paint no message. Clip to the
existing text viewport; do not add document text/rows, hit targets or measurement inputs.
The existing Goto > Language Server > Diagnostics picker exposes full admitted messages,
including diagnostics omitted by threshold or line priority; gutter/underlines stay intact.

Use existing path/version/route admission and a source-rope-bound cache. Edits/reload/shared
edits and removed/restarted routes must hide old winners immediately, without input work or
per-frame diagnostic sorting. Existing Session::drain accepts a versionless report for an
open URI; numeric versions must equal the live serial, and protocol identity changes retire
old clients. Preserve this policy: a versionless server report relies on live-session
admission and cannot prove the server's internal compute version. No stale admission widens.

Acceptance: causal config/default/scoped/malformed threshold tests; real GPUI priority/tie/
threshold/elision, wrap/fold/filler/horizontal clipping, stale/reload/shared/IME/protected-view
and copy/save/text/coordinate/selection/caret invariance; existing fixture numeric-version and
old-client tests plus all-open picker full-detail interaction. Keep passive occurrences, Find
and all manual styles. Run affected UI/config/syntax suites, strict affected all-target checks,
fmt/diff and existing exact serial paint/highlight/scroll/typing gates on final source.

Inline cache freshness also uses the existing language-service generation, so restart/settings
invalidation hides messages before transport work. The epoch-guarded publication reinstalls
winners even for identical reports and unchanged source. No shared input observer changes.

Task59 full-detail completion: the established Diagnostics picker previously stored the full
label but clipped it visually. Add only a diagnostic-mode selected-message area in that same
owner, wrapping within panel/window bounds with vertical scrolling. Keyboard list/query,
confirmation and Escape/focus restoration keep existing routes; no editor hit region is added.
Prove the long-message endpoint enters view after scrolling and selection/query changes update
and reset details. The eight passing gates cover unchanged editor paint/input/search source;
this later cold picker branch receives focused GPUI/owner and affected strict checks, with
ROOT checkpoint59 covering all final source under the updated evidence policy.

Task59 implementation/proof complete pending ROOT checkpoint59/review: focused config1/UI10,
full affected config86/syntax184/UI1682 before cold picker completion, eight unchanged exact
serial release gates (UI10M35.589/300ms), final language179/picker24 and affected strict checks
pass. Readable picker detail now proves full endpoint scrolling at 400×300, selection/query
updates, reset, actual Escape dismissal/focus restoration and Unicode navigation. Literal
commands/results/failures and precise pre/post cold-branch gate scope are in
`.superpowers/sdd/editor-expanded-features-plan/task-59-report.md`. Canonical checks stay with ROOT.

N41 review repair contract: the physical source line end itself must lie inside the text mask before applying virtual spacing/elision. Horizontal scrolling past a short line end hides its message; scrolling back restores it without rebuilding winner data or changing source/selection/caret geometry. Preserve the existing offscreen-right/visible-tail regression. Checkpoint59 before this repair passed3,220/0/29 plus workspacefmt/strictalltargets/diff; final focused/strict/hot-path gates will cover the repair.

N41 accepted 2026-11-01: independent scoped Spec/Quality approve after the causal offscreen-left-end repair. Finalfocused12/affectedstrict/fmt/diff and ALL eight exact serial gates pass on final paint source; UI44.489ms/300ms,folds3.237/8,comparison1.758/10. Workspacecheckpoint59 passed3,220/0/29 plusfmt/strict/diff before this small repair; final focused and performance proof covers it. Defaultoff/thresholdwarning; immutable epoch/source winners, no source geometry or hit targets, and readable wrapped/scrollable full picker detail. Versionless reports retain documented existing live-session limits. No dependencies or open findings. Next N43 local Git tags; checkpoint63 then all final gates after64.

## N43 behavior contract

Local Tags… and Create Local Tag… extend the Git sidebar, Tools > Git and command palette.
The local-only picker shows each tag's name, lightweight/annotated type, target object and
annotation message, and offers confirmed deletion of the selected tag. Creation first resolves
HEAD (default) or an explicitly entered commit revision on a worker, then opens the name and
optional message form with the pinned commit ID. An empty/whitespace-only message creates a
lightweight tag; a message creates an unsigned annotated tag. Tag commands explicitly disable
tag signing and editor invocation, regardless of tag.gpgSign/core.editor; ordinary configured
commit signing remains unchanged. No dependencies, remote operations, checkout or index/source
changes are needed.

Reuse prepared repository identity, existing Git tasks, loading pickers, forms and error/status
reporting. Validate names with Git, reject duplicates without force, reject missing/non-commit
revisions and unborn HEAD, and refuse changed project/repository/HEAD contexts. Create uses the
resolved object ID, never reinterpreting the revision after the form opens. Delete verifies the
selected raw ref object (including annotated tag identity) and uses atomic compare-and-delete;
a replacement ref survives even if it changes after validation. Symbolic tag refs are listed
but refused for deletion. Cancellation and stale/background errors leave local refs unchanged.

Acceptance uses disposable repositories for Unicode/lightweight/annotated listing and creation,
chosen commit resolution, configured signing/editor isolation, invalid/duplicate names, unborn
HEAD, missing/non-commit revisions, stale forms/repositories and deletion including a replaced
annotation at the same commit and a replacement at the atomic deletion boundary. GPUI covers
actual menu/catalog routes, picker details, target/name/message forms, confirmation/cancellation,
useful errors and stale UI. Run focused Git/UI tests and strict affected Clippy/fmt/diff checks;
no hot body or performance workload changes are planned. Canonical checklist stays for ROOT.

N43 no-remote completion: tag-specific Git processes disable implicit promisor-object fetches and transport protocols, including list/resolve/create metadata reads; missing objects fail locally. This does not alter explicitly invoked remote operations or ordinary commit signing. A disposable fake SSH service invokes only local upload-pack, proving the pre-fix implicit-contact failure and positive controls without real network/remotes.

N43 review repair contract: reference-only tag completion updates its task/status without unrelated status/index/diff reloads, on both success and error. A missing promisor index blob must not start transport after actual create/error/delete UI submission; a local native-Git positive control proves the fixture. Local Tags also renders type/target/multiline message in a tag-specific bounded, wrapped, scrollable selected-detail surface, retaining existing picker actions and diagnostic behavior. Actual narrow-window scroll/focus and metadata rendering regressions cover it.

N43 accepted: both review findings closed by independent scoped Spec/Quality review. Final Git UI96/0, tags8/0, picker24/0, diagnostic11/0 plus affected strict/fmt/diff pass; backend110/0 remains unchanged. No new dependency/hot editor body. Next U01, with existing native Character Viewer already manually verified; checkpoint63 and all final gates remain.

### U01 implementation contract (Task 61, review pending)

Defaults remain ordinary native text/IME input. Edit/menu/palette expose Insert Unicode
Character (a single trimmed hexadecimal scalar, optional U+ prefix), Insert Literal Next
Key, and the existing native Character Viewer through GPUI's show_character_palette.
Scalar validation accepts supplementary and combining characters; malformed hex, surrogates,
and values above U+10FFFF fail before any edit. The explicit insertion replaces every
selection atomically, without auto-pairs, as its own Undo step. The form captures document,
revision and selection identity; changed targets refuse visibly. Read-only/viewer/Follow,
immutable views and shared native composition refuse dedicated insertion/arming.

Literal arming belongs only to the focused document view. One safe text-producing key
bypasses editor bindings and auto-pairs through the existing native input callback; Escape
cancels and is consumed. Control/Command/Option/function keys and non-text navigation cancel
and continue ordinary routing; no raw shortcut control character is synthesized. Native
marked-text input cancels arming before composition and retains its normal preview/commit
semantics. Literal arming yields to non-layout native input methods; a read-only current-source
kind query and GPUI source-change subscription protect the initial preedit key as well.
For direct layouts, an armed-event-only OS dead-key-state probe yields to native interpretation;
GPUI key_char alone is insufficient because it resolves dead keys to spacing accents. The
probe never inserts translated text and unavailable/ambiguous events follow normal OS input.
Qualified direct input uses exact NSEvent.characters, including Caps Lock and Unicode that
differ from GPUI fallback key_char; counted native UTF-8 retains NUL for control rejection.
Blur and mode changes cancel arming. A changed document/revision cancels and consumes the
pending event, preserving a peer composition that started after arming. The transient state
keeps weak shared-peer references captured before borrowing Editor, including identical-byte
previews that do not change document revision; no persistence or new
input service is added. Dedicated Character Viewer uses the manually checked existing
selector (native-character-viewer-preflight.md), restores editor focus, and keeps OS input.

Acceptance: causal GPUI command/form and native EntityInputHandler tests for emoji, combining
scalars, invalid/surrogate input, multicaret/shared Undo, protected/stale targets; real binding
bypass, one-input scope, Escape, shortcut safety, focus and native IME preservation. Run
focused input/connected regressions, strict affected all-target checks, fmt/diff and unchanged
serial release typing/find input gates. ROOT owns independent review and canonical checks.

U01 worker checks: 17 feature tests, all 858 editor tests, connected input/composition and
command/form/protected-mode regressions, strict affected all-target Clippy, fmt and diff pass.
The exact serial typing/find/UI-find gates passed before the final armed-event-only native-text
repair; fresh feature/input/composition and strict checks cover that repair. ROOT retains
independent review, actual current-build native QA, canonical acceptance and final global gates.

U01 review repair contract: approved physical Return/keypad Enter normalize their native CR/Enter code to the editor LF convention, bypassing autoindent while other raw controls remain rejected. Record resolved literal text (with replacement offsets) rather than transient arming: Escape/navigation cancellation must replay as ordinary text and successful literal replay must not depend on the current input source. Reuse existing typed command/admission/recording and insertion paths; preserve shared IME/protected/Undo behavior. Cover causal native-metadata and real ReplayMacro regressions, affected strict/connected checks, and current-build native Return/Unicode rechecks before acceptance.

U01 accepted 2026-11-02: final20/0/fullUI1711/0/25/editor858/0, strict/fmt/diff and serial finalUI38.138ms/300ms pass. Independent scoped review closes both native-newline and macro findings; native scalar/Unicode/Undo/Return/Tab/Escape/dead-key checks pass. Caps automation/current Viewer AX limits are recorded alongside the successful prior OS service preflight; no unsupported runtime claim. No dependencies or open implementation findings. Next U02 local-snippet templates, then U06/checkpoint63 and U07/final gates.

### U02 implementation contract (Task 62)

File > New From Template… and the matching command-palette/keybinding action open a searchable
list of all existing local snippet definitions, followed by an explicit searchable language
choice. Nothing is created until both choices are confirmed. The selected definition is
reread through the snippet owner at each confirmation; removed, invalid or changed definitions
and missing language IDs refuse visibly without adding a tab. Shared native composition is
admitted before opening or confirming a picker, preserving the source text, selections,
syntax, saved point and Undo state on launch/cancel/refusal.

A successful choice expands the existing snippet parser/model with ordinary variables,
mirrors, transforms and owned choice fields before attaching one dirty untitled document.
Clock/clipboard/project context is captured at creation; untitled file/selection variables use
the new empty document, never the source. Even an empty `$0` expansion is dirty through the
existing forgotten-saved-point metadata, without synthetic text edits. The existing syntax
and preference route applies the selected language; runtime registry IDs retain their identity
with Plain Text when no grammar exists. Save and Save As retain the ordinary untitled flow.
No template code executes and creation writes no file.

Acceptance: causal GPUI command/picker regressions for content/language/fields/mirrors/choices,
commit-time variables, empty dirty content, cancel/invalid/stale choices and tab counts,
shared/local IME refusal, ordinary snippet insertion and normal Save As/copy/read-ticket
protections. Run focused UI/editor/config/syntax checks, affected strict all-target Clippy,
formatting and diff checks; only existing serial gates for any actual hot-path body change.
ROOT owns independent review and the canonical checklist.

U02 worker verification: all eight causal GPUI template regressions pass within affected
UI1719/0/25ignored, editor858/0, syntax184/0/3ignored and config86/0 suites. Existing local
snippet insertion/macro, native composition/literal input, Save As/copy and read-ticket tests
remain passing; strict affected all-target Clippy passes. Search covers titles/filenames/
triggers and runtime language IDs with the existing detail surface. No dependencies or hot
typing/find/render/snippet-expansion body changes; ROOT retains independent review and
canonical acceptance. Exact commands, prior failures and final format/diff results are in
the private Task 62 report.

U02 accepted 2026-11-02: independent combined Spec/Quality approval, no required findings. Final9/0 includes ordinary Save As redetection and fixture completion for runtime/bundled languages; final strict UI/app/Git all-target Clippy/fmt/diff pass. All original affected owner results above retained with their exact scope. No product route change was needed.

U06 accepted 2026-11-02: focused9/0, affected config87/editor858/UI1728 all0, all8 exactserialreleasegates pass; final workspace3276/0/29existingignored with fmt/strict all-target Clippy/diff0. Independent product SpecPASS/QualityAPPROVE, no findings. A checkpoint-only comparison test assumed one parked turn completed4ms/frame rewrapping. Controlled100kline RED proved pending maps; GPUI test display-link is a no-op. Test-only bounded completion now drives actual Window frames and retains every original alignment assertion, with added map/first/middle/last checks. Final aligned8/0 and fresh scoped repair SpecPASS/QualityAPPROVE; production/deadlines/budgets unchanged. Failure logs and actual commands are retained in the Task63 report. No acceptance by blind retry.

### Pre-final F07/L02/E04 regression contract

On a rescue-picker read error, retain the actual typed query and composition, filter replacement actions coherently and keep a visible/focused error; clearing the query reveals retry/return actions. Explicit Restart Language Servers cancels pending/displayed signature ownership before the Manager lock can delay restart, with source text/focus/Undo unchanged. Test the actual restart command with a held Manager and real fixture replies. Strengthen the existing overtype indicator test by proving its control moves between resized frames before label containment, preserving all prior assertions. Native print QA window startup registers the same quit/save hooks as production so terminal cleanup verification uses the actual lifecycle. Defaults, deadlines and performance thresholds remain unchanged. Cover causal regressions, affected UI/all-target strict/fmt/diff, then independent scoped review before the final U07 slice.

Pre-final F07/L02/E04 corrections accepted: causal3fail→3pass; indicator1pass; fullUI1732/0/25existingignored, strictUI/appalltargets/fmt/diff0; independent SpecPASS/QualityAPPROVE/no findings. Native QA startup now uses production quit hooks; actual terminal quit cleanup remains a later native check. Exact commands/failure logs: `.superpowers/sdd/editor-expanded-features-plan/prefinal-report.md`.

Native print backend verification2026-11-02: existing `print_qa` NSPrintSaveJob harness and `swift scripts/verify-print-pdf.swift` both exit0. Document14pages, selection1, numbered-large25, empty1, no-wrap2, color14 and multi-selection7. PDFKit source records/order/Unicode/selected scope/line numbers/options checks passed. Inspected all-page contact sheets and representative first/middle/last pages: readable glyphs/margins and no observed clipping/overlap. No physical jobs. Actual commands/results in `final-native-print-generation.log` and `final-native-print-verification.log`; output directory recorded in private `final-print-qa-path.txt`. Native menu/Page Setup/PDF-panel interaction remains for the final current-build UI pass.

### Final native T06 quit-cleanup repair contract

Ordinary tab/window closure remains nonblocking and retains command review. Application quit
must await the existing PTY worker's child reaping and private launch-file removal, including
workers from just-closed tabs, through GPUI's existing asynchronous quit hook. The worker
remains the sole owner of its integration directory; never remove startup/history files while
its shell can still use them. Keep GPUI's existing shutdown timeout and all performance budgets.
A real integrated two-session regression covers current and previously closed tabs, retained
views, child reaping, positive-control launch files and cleanup before shutdown returns.
Native rerun must confirm exact recorded PIDs and integration paths after Quit. No broad temp
cleanup, user startup-file changes, dependency or replacement lifecycle engine.

Native N43 repair contract: the selectable tag row must keep its name and one-line metadata
caption inside its existing fixed row height even for multiline annotations. The selected
wrapped, bounded, scrollable detail retains the entire original name/metadata/message. Apply
the compact caption only to existing choice-detail pickers; ordinary picker layouts and
original detail/scroll/source-ref assertions stay intact. Add an actual rendered row/label
containment regression and repeat the native tag picker after the repair.

Final native T06 diagnosis: waiting alone fixed the ordinary GPUI regression but the actual
macOS run still left the active session's directory. Temporary worker/quit diagnostics and an
idle-worker stack sample confirmed stop/join delivery and the poll/termination boundary.
`portable-pty` Child::kill has a 200 ms HUP grace, exceeding GPUI's unchanged 100 ms quit budget.
The final worker has a socket cancellation wakeup, retains asynchronous completion handles for
quit (including previously closed tabs), and performs its own 50 ms HUP grace before the
existing forced termination. A reaped child receives no further PID/group signals. The worker
still removes only its own launch files after child reaping. All temporary product diagnostics
were removed; failure logs/sample remain private. Added ignored-HUP integrated shutdown and
normal HUP state-persistence coverage. Rebuilt final native app: explicit tab close→Quit reaped
both 51411/51487 and removed both exact recorded directories (final-native-quit-cleanup-v2.log
EXIT0). Native tag row names now stay visible, with full wrapped/scrollable selected details.
Fresh covering/full final gates and independent review subsequently passed as recorded above.
