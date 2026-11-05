# Feature audit, 2026-10-21

This maintenance pass checked the ten-crate workspace and exercised the native macOS app
through computer use. The assumed workload is one person editing local projects, including
large files covered by the existing release benchmarks. The automated suite covers the
implemented feature families; this is not a claim that every menu item, operating system,
external language server or possible input was manually tested.

## Changes

- **Related Files:** project-excluded and Git-ignored conventional test files no longer
  reappear through the sibling-folder fallback. Loose files outside the project roots still
  find their sibling tests. Both cases have regression tests; each failed before its fix.
  The excluded-file bug was reproduced in the original native app and verified corrected
  in the rebuilt bundle.
- **Fuzzy matching:** scoring streams Unicode characters and stops matching once a term is
  complete, counting the remaining characters for the existing trailing-length penalty.
  It no longer allocates character vectors for every candidate and term. Ranking, word
  boundaries, Unicode folding, gaps and tie order are preserved. A differential check against
  the original implementation matched 200,000 scores and 200 complete ranked lists.
- **Explorer:** the UI now uses `orlo_project::explorer` instead of maintaining a duplicate
  implementation. Directory ordering, ignored-name handling, temporary-save-file filtering
  and polling behavior retain their existing tests.
- **Unused files:** removed the four unreferenced empty scaffolds `.cargo/.gitkeep`,
  `assets/fonts/.gitkeep`, `assets/keymaps/.gitkeep` and `assets/themes/.gitkeep`, plus two
  ignored Finder `.DS_Store` files. The source map no longer lists the empty Cargo folder.
  A module reachability check found no orphan Rust source files. Bundled icons and both
  language fixtures are referenced and retained.

## Feature coverage

| Feature family | Automated checks | Native checks in this pass |
| --- | --- | --- |
| Text input, selections, undo and clipboard | Editor model and GPUI input/selection tests, including Unicode and IME | ASCII entry, Unicode paste, save, undo and redo |
| Editing tools | Transforms, codecs, hashes, statistics, snippets, bookmarks, CSV/TSV and column commands | Quoted CSV column selection containing a comma and emoji; JSON formatting and save |
| Search | Literal/regex search, replacements, selection limits, saved queries, extraction, highlights, pipelines and stale/cancelled work | Find count and navigation, Replace All, Find in Files with explicit paths, globs and Git ignore rules |
| Files and storage | Atomic writes, encodings, read-only/error handling, history, comparisons and disk conflicts | Native Open dialog, Unicode contents on disk and clean-buffer external reload |
| Projects and navigation | Roots/excludes, project/workspace persistence, file/symbol/reference navigation and related files | CLI project opening, Goto Anything, Rust symbol picker and excluded-file bug reproduction |
| Views and appearance | Layout variants, tabs, shared views, wrapping, folds, scrolling, panel docking and customization | Two-column layout, file opened in the second group, outline, folding and unfolding |
| Git | Status, staging/unstaging, branches, commits, diffs and conflicts in disposable repositories | Status and saved-content Unicode diff in a disposable repository |
| Terminal and automation | PTY input/output, lifecycle, macros, packages, filters, tasks, worksheets and customization | Integrated terminal starts in the fixture folder and runs `printf` successfully |
| Language intelligence | Bundled grammars, parsing, completion, spelling and fixture-server protocol/UI tests | Rust highlighting, symbols and outline; unavailable-server status |
| CLI and recovery | Help/version, malformed input, file handoff/waits and TERM/INT/HUP session persistence | CLI folder/file handoff; unsaved Unicode text, tabs and layout restored after quit and relaunch |

The ordinary baseline had 1,753 passing tests. After the three added regressions, all
1,756 tests pass. The 22 ordinarily ignored tests are 21 release performance/memory gates
and one real-language-server smoke test. Formatting and strict workspace Clippy also pass.

All 21 release performance and memory gates passed before and after the changes. The full
workspace release gates ran serially, after compilation finished, with
`ORLO_INDEX_TREE="$PWD/crates"`. The existing workloads measured:

| Workload | Before | After |
| --- | ---: | ---: |
| Fuzzy filtering, 20,000 paths | 3.966 ms | 1.129 ms |
| Completion ranking, 20,000 candidates (best of ten) | 7.474 ms | 5.314 ms |

These are local workload measurements, not an end-to-end typing latency guarantee.
No performance budgets or checks were weakened.

The final workspace release build and `scripts/bundle.sh` passed. The rebuilt
`target/Orlo.app` has a verified ad-hoc signature and was launched for the native bug-fix
and recovery checks. Signing for downloaded distribution remains outside this audit.

## Evidence and remaining limits

Logs and disposable measurement programs are under ignored `target/orlo-audit-*` paths.
Use [the verification guide](verification.md) for the standard commands. The support-folder
backup is `target/orlo-audit-support-backup/`; native editing used `/tmp/orlo-feature-audit/`.
Build caches and earlier recovery backups were retained.
The original session was restored after native testing; the disposable test session is retained
as `target/orlo-audit-session-after.json`.

The dependency audit found zero reported vulnerabilities, with the existing 15 unmaintained
and two soundness notices still present. Rust also reports existing future-compatibility
warnings for `block` and `proc-macro-error2`; no notices or checks were suppressed.

Real language servers were unavailable: `target/lsp-tools` is absent, and the installed
`rust-analyzer` launcher reports that its toolchain component is unavailable. The real-server
smoke test therefore remains unverified; fixture-server tests passed. Other macOS versions,
Intel hardware, multiple displays, VoiceOver, notarization and downloaded-app installation
were not checked. The feature limits already listed in the README remain applicable.
