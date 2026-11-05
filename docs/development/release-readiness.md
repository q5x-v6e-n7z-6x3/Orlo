# Local alpha readiness, 2026-10-20

Orlo 0.1.0 is ready for local alpha testing on this Mac: Apple Silicon, macOS 27.0.1,
Rust 1.99.0. The current ten-crate Cargo workspace was checked directly; the earlier
pre-workspace assessment is superseded by the results below. No release was published.

The scope is local testing, as requested. Use the [local alpha guide](../user-guide/local-alpha.md)
to launch the app, exercise the editing workflow, back up sessions and capture problems.

## Verification

All commands ran from the project root using the pinned toolchain and lockfile.

| Gate | Result | Local evidence |
| --- | --- | --- |
| Formatting | Pass | `cargo fmt --all -- --check` |
| Strict Clippy | Pass | `target/alpha-clippy.log` |
| Workspace tests | 1,753 passed, 0 failed, 22 ignored | `target/alpha-tests.log` |
| Release performance and memory | All 21 checks passed | `target/alpha-performance-isolated.log` |
| Dependency audit | 0 vulnerabilities across 765 packages; 17 notices | `target/alpha-audit.json` |
| Workspace release build | Pass | `target/alpha-build.log` |
| App bundle | Built; ad-hoc signature verifies | `target/alpha-bundle.log` |
| Native app smoke test | Launch, Unicode save, undo/redo, Find and session restoration verified | See below |

The 22 ignored tests in the ordinary suite are the 21 release performance/memory checks and
one real-language-server smoke test. The latter was not run: `target/lsp-tools` is absent.
See [development verification](verification.md) for the full commands and server requirements.

The first performance run overlapped the test suite and compilation of the audit tool.
Completion ranking took 10.403 ms against its unchanged 10 ms budget. With those competing
jobs finished, the full serial release run passed; completion ranking took 7.508 ms.
The initial result remains in `target/alpha-performance.log`. Run timing checks on an otherwise
quiet machine; no budget or test was weakened.

The dependency check used cargo-audit 0.22.2 and RustSec database commit
`e6f94f3760d5e6c54c1b05f204002263bc025524`, updated 2026-10-20. The audit tool was installed
under `target/alpha-tools/`, without changing project dependencies. Recheck with:

```sh
target/alpha-tools/bin/cargo-audit audit
```

## Native smoke test

The signed bundle was opened with disposable files in `target/alpha-smoke/`.

- The editor rendered the folder, tabs and Unicode text.
- An edit containing `café 日本語 😀` was saved; its UTF-8 bytes were checked on disk.
- Undo and redo were saved and checked on disk.
- In a normal launch, Command-F opened Find; searching `alpha` displayed one match in the
  unsaved recovery buffer.
- Command-Q exited the instrumented process successfully. A subsequent normal launch restored
  the untitled buffer with its unsaved Unicode text, verified in the session data and window.

The diagnostic launch recorded an app-side first paint of 128 ms in `target/alpha-runtime.log`.
Its automation screenshots sometimes retained an older editor frame until a resize; the same
capture symptom was not reproduced in the subsequent normal launch. This observation does not
establish whether the cause was the diagnostic run, capture tooling or rendering. Check normal
interactive redraw during local alpha use and investigate if it recurs.

The preceding app, support folder and edited documentation were backed up to
`target/alpha-backup/`. These are local build artifacts and are removed by `cargo clean`.

## Build identification

- App: `target/Orlo.app`, version 0.1.0, bundle ID `dev.orlo.editor`, arm64.
- Bundle metadata declares macOS 13.0 or later; only macOS 27.0.1 was tested here.
- Bundled executable SHA-256:
  `f1de8af99e0f4d4b40576545fba37c22e0737eafebefa78e1a91ea335d0cbd5e`.
- `Cargo.lock` SHA-256:
  `0bb34385bd49260cc8798f1244d5590261bdfa8143f799e878260993dba5f1e2`.

This checkout has no Git metadata, so no commit, branch, tag or hosted CI result is asserted.
The local evidence describes these files and this executable.

## Remaining limits

- The audit retains 15 unmaintained-package notices and two `git2` soundness notices through
  GPUI. They were not suppressed. Review upstream updates before wider distribution.
- Cargo reports future Rust compatibility warnings in `block` 0.1.6 and
  `proc-macro-error2` 2.0.1. Strict Clippy passes on the pinned toolchain; details are in
  `target/alpha-upstream-warnings.log`.
- Help > Documentation opens Sublime Text's documentation. Orlo's own guides are in this
  checkout, starting with the local alpha guide.
- Other macOS versions, Intel Macs, multiple displays, VoiceOver, real language servers and
  installation from a downloaded archive were not verified in this run.
- The feature and format limits in [Not in this version](../../README.md#not-in-this-version)
  still apply. Local alpha testing is not a promise of general release stability.

## Distribution

The local bundle is ad-hoc signed with the hardened runtime. This passes
`codesign --verify --deep --strict`, but does not establish downloaded-app Gatekeeper acceptance.
This Mac reports no valid code-signing identity. Signing and notarization for distribution
remain outside this local alpha preparation.

The existing bundle script supports the later distribution workflow:

```sh
# once: store App Store Connect credentials for notarytool in the keychain
xcrun notarytool store-credentials orlo-notary --apple-id YOU@EXAMPLE.COM --team-id TEAMID

# each release: sign, notarize, staple and assess; writes target/Orlo.zip
ORLO_SIGN_IDENTITY="Developer ID Application: NAME (TEAMID)" \
ORLO_NOTARY_PROFILE=orlo-notary ./scripts/bundle.sh
```

That credentialed path was not run. Hosted CI was also not queried; the existing workflow checks
formatting, the dependency audit, strict Clippy, workspace tests and the release build on macOS.
