# Local macOS alpha

Orlo 0.1.0 is an alpha for testing on the Mac that builds it. The current assessment covers
Apple Silicon on macOS 27.0.1. See the [verification record](../development/release-readiness.md)
for the checks and remaining limits.

## Build and open

From the project root, with Rust installed and the macOS command line tools available:

```sh
./scripts/bundle.sh
open target/Orlo.app
```

Cargo uses the Rust version in `rust-toolchain.toml` and the dependencies in `Cargo.lock`.
The script makes an optimized app with an ad-hoc signature. Developer ID credentials are not
needed for this local build. Rebuilding replaces `target/Orlo.app`; the app keeps its settings
and session separately.

Only one Orlo process runs at a time. Quit an older Orlo before opening a rebuilt app;
otherwise the older process receives the launch. To open a folder or file in the running app:

```sh
target/Orlo.app/Contents/MacOS/Orlo /path/to/test-folder
target/Orlo.app/Contents/MacOS/Orlo /path/to/notes.txt:10:1
```

## Try the editing workflow

Use disposable copies of files for the first pass. Work through these checks before using the
alpha for everyday editing:

- Open a folder, browse its files and switch between tabs.
- Edit and save a text file containing accents, emoji and multiple lines. Undo and redo, then
  reopen the file and check the text on disk.
- Try Find and Replace in one file, then Find in Files. Review Replace in Files before applying
  it to a disposable folder.
- Add multiple carets, split the editor into groups and move a tab between groups.
- Leave an untitled buffer unsaved, quit with Command-Q and reopen. Confirm the text and layout
  are restored.
- Change an open file in another editor and check Orlo's reload/conflict handling.
- Open the integrated terminal and try Git status and diffs in a disposable repository.

Language servers are separate local programs. Syntax highlighting works without them;
language-server completion, diagnostics and navigation need the relevant server installed.
The [README](../../README.md#language-intelligence) describes the supported setup.

## Session backup and recovery

Settings and session recovery data live in `~/Library/Application Support/Orlo`. Quit Orlo
before backing up or restoring this folder. For a backup from the project root:

```sh
backup="target/orlo-support-backup-$(date +%Y%m%d-%H%M%S)"
ditto "$HOME/Library/Application Support/Orlo" "$backup"
```

Keep that backup outside `target/` if you plan to run `cargo clean`. Restoring a backup replaces
the session state, so first keep a copy of the current support folder. Session recovery does
not replace backups of saved files. The preparation run also kept the preceding local app and
support folder in `target/alpha-backup/`.

## Capture a problem

Record the steps, expected result, actual result and `Orlo --version` output. Include a small
sample file when possible. For startup and rendering diagnostics, quit Orlo first, then run:

```sh
ORLO_PERF=1 target/Orlo.app/Contents/MacOS/Orlo > target/alpha-runtime.log 2>&1
```

The log may contain local file paths; review it before sharing. macOS crash reports, if any,
appear in Console under Crash Reports.

Known limits include the ones [listed in the README](../../README.md#not-in-this-version).
Help > Documentation currently opens Sublime Text's documentation; use the Orlo guides in
this checkout for Orlo-specific behavior. Downloaded-app installation, notarization, Intel Macs,
older macOS versions and multiple displays are outside this local alpha assessment.
