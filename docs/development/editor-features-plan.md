# Remaining editor features implementation plan

**Goal:** Complete the authorized audit IDs in order while keeping Orlo a focused macOS text editor.
**Architecture:** Extend existing ten-crate owners, document/view separation, command catalog, settings, widgets and background tasks. Complete and review one vertical slice before starting the next.
**Spec:** The user's authorized scope, docs/development/editor-features-progress.md and the historical docs/development/editor-comparison-2026-10-29.md.

## Global Constraints

- No commits, pushes, PRs, publishing or releases. Remain in this existing isolated worktree.
- Do all assigned implementation yourself; never spawn agents. The controller supplies review.
- Preserve all existing features; deferred F05, F06, E07, E08, E09 and G05 stay excluded.
- Do not add debuggers, advanced refactoring platforms, AI, collaboration services, deployment tools, databases, hosted PR clients, remote development environments, marketplaces or OS ports.
- Read the architecture map, verification guide, relevant audit sections and current implementation before editing. No AGENTS.md exists in this checkout or ancestors.
- Before each feature, append a short behavior specification (defaults, entry points, failure/stale behavior, acceptance tests) to the existing progress document; resolve routine choices and document them.
- Complete backend, reachable UI/commands, settings/persistence as needed, meaningful tests and user documentation. Explain every new dependency. Do not copy upstream reference implementations.
- Keep the single audit-ID checklist in the progress document accurate; mark only implemented and verified behavior complete. Preserve the original audit unchanged.
- Test Unicode, shared/multiple views, undo/redo, read-only, cancellation, stale results, persistence compatibility and save races where relevant.
- Use CARGO_TARGET_DIR=<checkout>/target for builds. Use focused regression tests while iterating; final workspace fmt, strict Clippy, tests, release build, diff check and relevant performance gates are the controller's final gate.
- Do not weaken checks, delete coverage, suppress failures or claim unrun checks. Git verification uses disposable repositories/local remotes only.

## Review Focus

Save preparation serialization and close/quit waiting; view/document ownership and stale asynchronous results; Unicode/IME geometry and exact document bytes; backwards-compatible private persistence; Git protections and all unsaved views.

## Task 1: L04 — Opt-in formatting on save

**Owners to inspect:** crates/ui/src/documents/save.rs; crates/ui/src/views/workspace/files.rs; crates/ui/src/views/workspace/language.rs; crates/config/src/settings/mod.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add opt-in LSP document formatting on save, disabled by default. Integrate asynchronous preparation into the existing per-document save serialization and write leases. Never let formatting race another save, overwrite edits made while it waits, or make quit/close miss a pending save. Preserve safe writes, cleanup, Save As, shared views and ordinary manual formatting. Explicitly define stale/failure behavior in the behavior spec. Test source edits during formatting, queued saves, shared views, close/quit waiting, read-only, errors and default-off behavior using fixture servers and existing save gates.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 2: L05 — Runtime language manifests and server routing

**Owners to inspect:** crates/syntax/src/languages/manifest.rs; crates/syntax/src/languages/registry.rs; crates/ui/src/views/workspace/language.rs; crates/ui/src/views/workspace/goto.rs. Discover connected callers and tests before modifying them.

**Requirements:** Wire the existing language manifests/registry into runtime detection and language-server routing so additional languages can be configured without recompilation. Reuse existing settings, reload and server management. Preserve bundled languages and grammar overrides. Do not add a general plugin runtime or WebAssembly grammar system. Test an additional manifest/file extension and fixture server end to end, overrides/reload, malformed configuration and existing family sharing/root detection.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 3: E01 — Syntax-driven folding

**Owners to inspect:** crates/syntax/src/grammars/treesitter.rs; crates/editor/src/rendering/fold.rs; crates/ui/src/views/editor/commands.rs; crates/ui/src/views/editor/element.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add syntax-driven fold discovery for supported languages, retaining indentation fallback and existing manual/inline selection folding. Fold discovery must be reused by commands and gutter controls, and stale parser output must not fold unrelated current text. Test braces/comments/markup as supported, Unicode, nested and inline folds, edits, fallback and rendering performance gates.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 4: E02 — Persist valid fold state

**Owners to inspect:** crates/workspace/src/sessions/mod.rs; crates/editor/src/rendering/fold.rs; crates/ui/src/views/workspace/sessions.rs. Discover connected callers and tests before modifying them.

**Requirements:** Persist fold state across reopening and session restoration, per view rather than document. Restore only valid state and remain compatible with older sessions. Preserve exact inline selection folds, nested folds and unchanged document bytes/undo. Define stale-file validation/fingerprints and reopen behavior. Test old schemas, corrupt/out-of-range/stale state, multiple/shared views and close/reopen/session restoration.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 5: E03 — Unusual Unicode character warnings

**Owners to inspect:** crates/config/src/settings/mod.rs; crates/ui/src/views/editor/element.rs; crates/ui/src/views/editor/measure.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add configurable warning glyphs/descriptions for unusual Unicode characters. Preserve document bytes, caret geometry, selections, IME behavior and legitimate Unicode sequences such as emoji ZWJ/variation sequences and combining text. Reuse existing render overlays rather than replacing text. Define exact defaults and supported configuration; provide reachable UI for descriptions. Test Unicode sequences, selection/caret geometry, composition and rendering gates.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 6: E04 — Optional overtype mode

**Owners to inspect:** crates/editor/src/commands/typing.rs; crates/editor/src/buffer/state.rs; crates/ui/src/views/editor/actions.rs; crates/ui/src/bars/statusbar/mod.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add optional overtype mode with a clear indicator and reachable toggle. Ordinary insertion is the default. Preserve grapheme boundaries, line endings, multiple carets, shared views, undo/redo, read-only and IME. Define mode ownership and paste/newline behavior. Test actual input paths and mode indication, not only a backend flag.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 7: E05 — Virtual-space editing

**Owners to inspect:** crates/editor/src/selection/mod.rs; crates/editor/src/commands/typing.rs; crates/ui/src/views/editor/mouse.rs; crates/ui/src/views/editor/measure.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add configurable virtual-space editing beyond line ends. Materialize padding only when an edit requires it. Handle tabs, Unicode/graphemes, multiple cursors, shared documents/views, undo/redo, read-only and IME correctly. Preserve normal insertion and rectangular tools. Define defaults, coordinate representation, navigation and persistence compatibility. Exercise real GPUI input plus release rendering/input gates.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 8: E06 — Declarative modelines

**Owners to inspect:** crates/config/src/settings/mod.rs; crates/config/src/settings/editorconfig.rs; crates/ui/src/views/editor/mod.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add a limited declarative modeline subset for indentation, tabs, wrapping and margins. Never execute modeline contents. Define supported syntax, scan bounds, enable/default behavior and precedence with existing settings, EditorConfig and explicit overrides. Preserve automatic indentation and settings reload. Test valid fields, malicious/unsupported text, precedence, Unicode and multiple views.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 9: F01 — Legacy and UTF-32 encodings

**Owners to inspect:** crates/project/src/documents/encoding.rs; crates/project/src/documents/io.rs; crates/editor/src/buffer/save.rs; crates/ui/src/views/workspace/encoding.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add legacy encoding and UTF-32 support with explicit reinterpret/reopen versus save-conversion workflows. Cover common Windows, ISO and CJK encodings and document the exact supported set. Keep ISO-8859-1 distinct from Windows-1252. Reject silent lossy writes and preserve BOM/newline metadata. Apply compatible handling consistently across saves, search, replacements, comparisons, pipelines and history. Read all encoding callers before editing. Use existing/native codecs if reasonable; explain any new dependency. Test malformed/unrepresentable input, round trips, BOM/newlines and every storage consumer.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 10: F02 — Native printing and PDF

**Owners to inspect:** crates/ui/src/platform/mod.rs; crates/ui/src/views/workspace/menus.rs; crates/ui/src/views/workspace/tools.rs; crates/utils/src/objc.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add native paginated printing and system PDF output, including selection/document scope and basic source-formatting options. Reuse native macOS APIs and UI helpers. Printing must not edit a document or run physical jobs during verification. Test paginated PDF output via a disposable output file, selection versus document, Unicode/tabs, settings and reachable Print command/menu.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 11: F03 — Read-only log following

**Owners to inspect:** crates/project/src/watchers/mod.rs; crates/ui/src/views/workspace/disk.rs; crates/editor/src/buffer/state.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add explicitly invoked read-only log-follow mode. Handle append, truncation, rotation and stopping cleanly, with background reads and cancellation/stale guards. Preserve normal external-change behavior, save leases, byte encodings, read-only state and shared views. Define dirty-document entry policy and caret/follow behavior. Test append/truncate/rename rotation, partial encoded input, stop/close and shared-view safety.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 12: F04 — Opt-in file autosave

**Owners to inspect:** crates/ui/src/documents/save.rs; crates/ui/src/views/workspace/files.rs; crates/ui/src/views/workspace/disk.rs; crates/config/src/settings/mod.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add opt-in autosave to existing document files, disabled by default. Keep it separate from session recovery and historical backups. Reuse save serialization including L04 formatting, external-change protection and safe writes; skip untitled/read-only/follow documents. Define debounce and failure behavior and expose settings/UI. Test shared views, edits during saves, external changes, close/quit, untitled/default-off and recovery/history separation.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 13: F07 — Discarded untitled rescue

**Owners to inspect:** crates/workspace/src/sessions/mod.rs; crates/ui/src/views/workspace/files.rs; crates/ui/src/views/workspace/sessions.rs. Discover connected callers and tests before modifying them.

**Requirements:** Recover explicitly discarded untitled documents through bounded private retention. Document exact bounds/behavior and add a clear-history action plus reachable restore picker. Preserve existing hot-exit/session recovery and historical backups; only capture a genuinely discarded document, including Discard All, without losing still-open shared views. Test private permissions, bounded Unicode data, restore, clear, older sessions and close/quit/discard semantics.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 14: G04 — Amend and commit composition

**Owners to inspect:** crates/git/src/commits/mod.rs; crates/ui/src/panels/git/mod.rs; crates/git/src/tests.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add amend controls and assisted commit composition/coauthor trailers. Preserve ordinary commits, staged-only behavior, partial staging protections and Git-configured signing. Validate trailer inputs and amend eligibility; show useful errors. Update product documentation to explain the intentionally expanded Git scope. Verify using disposable repositories only; never mutate the working repo just to test Git features.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 15: G02 — Stash workflows

**Owners to inspect:** crates/git/src/lib.rs; crates/ui/src/panels/git/mod.rs; crates/git/src/tests.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add stash list/create/preview/apply/pop/drop with asynchronous UI and useful errors. Respect unsaved open documents, stale selections and Git conflict protections; do not silently force/drop work. Preserve existing Git features. Use disposable repositories and test conflicts, missing/stale stashes, previews and selection preservation. No real user remotes or repositories for test operations.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 16: G03 — Worktree lifecycle

**Owners to inspect:** crates/git/src/branches/mod.rs; crates/git/src/repository/mod.rs; crates/ui/src/panels/git/mod.rs; crates/ui/src/views/workspace/projects.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add worktree list/create/open/remove/prune, respecting Git protections and unsaved documents across windows. Reuse existing branch/link-worktree discovery and workspace opening. Avoid forced removal or destructive bypasses. Test linked worktrees, current/dirty/locked worktrees, unsaved documents, open navigation and prune in disposable repositories only. Do not create a checkout for the agent itself.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.

## Task 17: G01 — Explicit remote Git controls

**Owners to inspect:** crates/git/src/operations/mod.rs; crates/ui/src/panels/git/mod.rs; crates/git/src/tests.rs. Discover connected callers and tests before modifying them.

**Requirements:** Add explicitly invoked fetch/pull/push and remote/upstream controls with asynchronous execution and useful error reporting. No automatic remote contact; preserve Git-configured signing, credentials and pull policies, and unsaved-file safeguards. Update README/local-only policy intentionally. Verify with disposable repositories and local bare test remotes, including errors/divergence/upstream changes; never operate against real user remotes merely to test. No hosted PR client or remote-development environment.

- Write the feature behavior specification in the existing progress document before product edits.
- Implement the smallest complete vertical slice; preserve existing architecture and previous feature work.
- Add meaningful regression tests and run focused checks; update user docs and checklist accurately.
- Self-review and write the requested report with changed files, commands, actual results, decisions and remaining concerns. Never commit or spawn agents.
