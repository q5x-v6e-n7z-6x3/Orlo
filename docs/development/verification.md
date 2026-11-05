# Development and verification

Run commands from the repository root with the toolchain pinned in `rust-toolchain.toml` and
the committed `Cargo.lock`. The default workspace member is `orlo-app`; use `--workspace` when
checking all ten crates.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked --release
git diff --check
```

Building needs `cmake` (`brew install cmake`): tree-sitter's `wasm` feature builds wasmtime's C
API with it. After a release build, check every bundled language folder and the contributor
template as Orlo loads them:

```sh
target/release/Orlo --check-language assets/languages/*/ docs/templates/example-language
```

CI runs formatting, `cargo audit`, strict Clippy, workspace tests, the release build and that
language check on macOS. A full Xcode installation is not required for GPUI's runtime shaders.
The signal integration test opens the real application and requires a logged-in macOS GUI
session.

To run the application or create its bundle:

```sh
cargo run -p orlo-app --release -- .
./scripts/bundle.sh
```

The bundle script builds `Orlo`, writes `target/Orlo.app`, and verifies the bundle signature.
It signs ad-hoc by default. Developer ID signing and optional notarization use the existing
`ORLO_SIGN_IDENTITY` and `ORLO_NOTARY_PROFILE` environment variables; see
[distribution](release-readiness.md#distribution).

## Performance and memory

Performance checks are ignored in ordinary test runs because their budgets were measured in a
release build. Run them serially so the counting allocator and timing measurements are not
mixed with other tests:

```sh
ORLO_INDEX_TREE="$PWD/crates" cargo test --workspace --locked --release -- --ignored --skip real_language_servers_smoke --nocapture --test-threads=1
```

This selects all release gates, including the UI keystroke benchmark and completion ranking
timing test whose name does not start with `bench_`. `ORLO_INDEX_TREE` chooses the tree indexed
by the memory benchmark; its default is `target/lsp-tools/go` at the repository root.

`benches/mod.rs` retains the counting allocator, timing checks and large-source fixture helpers.
The workloads are grouped in `editing.rs`, `rendering.rs` and `filesystem.rs`. They run through
UI's test harness rather than a separate benchmark executable, preserving access to the real
project index and GPUI test context. Budget values and ignored flags stay with each check.

`ORLO_PERF=1` makes the application print startup, key-to-paint, scroll-to-paint and frame work
timings to stderr.

## Real language servers

The ignored server smoke test requires local servers under `target/lsp-tools`, or a directory
named by `ORLO_LSP_TOOLS`:

```sh
cargo test -p orlo-syntax --locked real_language_servers_smoke -- --ignored --nocapture
```

The recorded source assessment last passed with rust-analyzer 0.3.3081, gopls 0.24.0,
TypeScript Language Server 6.0.2 with TypeScript 6, and Python LSP Server 1.15.0. These are the
versions used for that assessment, not a statement about the latest available versions.

The added-family smoke test uses the registered defaults on PATH and checks real hover and
completion in C, C++, Swift, Bash and PHP. It needs clangd, SourceKit-LSP, Bash Language Server
and Intelephense. For test-local npm tools:

```sh
npm install --prefix target/language-support/lsp/npm --no-save --package-lock=false bash-language-server intelephense
PATH="$PWD/target/language-support/lsp/npm/node_modules/.bin:$PATH" cargo test -p orlo-syntax --locked real_language_servers_smoke_added_families -- --ignored --nocapture
```

Ordinary workspace tests compile and query every bundled parser, compare incremental Unicode
edits with fresh highlighting, and route every registered LSP ID through fixture subprocesses.
Those fixture checks do not prove compatibility with every external server version. See the
[setup guide](../user-guide/languages.md) for the complete language and server matrix.

The [release readiness assessment](release-readiness.md) records the current local alpha build,
test results, native smoke test and distribution limits. Repeat the commands above for a new
build before relying on that evidence. Run performance checks after other tests and compilation
finish so competing work does not distort the timing budgets.

## Native printing (no physical jobs)

On a logged-in macOS GUI session, use a fresh disposable directory:

```sh
cargo run -p orlo-ui --locked --example print_qa -- /tmp/orlo-print-qa-UNIQUE
swift scripts/verify-print-pdf.swift /tmp/orlo-print-qa-UNIQUE
```

The harness refuses to overwrite existing PDFs and uses `NSPrintOperation` with
`NSPrintSaveJob`, the same pagination/drawing path as the native print panel. It produces
whole-document, selected-source, larger-font/numbered, unwrapped, colored, multi-excerpt and empty-document PDFs. PDFKit verifies
multiple pages, all source records, Unicode, scope and option effects, then renders first,
middle and last pages and all-page contact sheets for visual inspection. No extra packages
are required. Inspect the PNGs for clipping, lost glyphs and page-boundary issues.

For the actual menu/form/modal-loop check, append `--window` to the harness command. This
opens a real workspace using a support directory inside the disposable QA directory. Choose
File > Print, enable page setup and cancel it; verify the editor remains responsive. Repeat and
choose landscape before previewing and saving a PDF. Also reopen Print and **Cancel** the native panel.
Repeat using File: Print in the command palette and save `native-panel.pdf` through the native
**PDF** button into the same directory; the PDFKit script verifies its 120 source records too.
Never choose a physical Print destination during this check. The native API must execute on
the foreground executor outside every GPUI App/Window update, since its modal loop re-enters
the application's callbacks.
