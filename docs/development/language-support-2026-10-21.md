# Fifteen-language support, 2026-10-21

The user selected Octoverse 2025's ten most-used languages plus Kotlin, Ruby, Swift, Rust and C.
The [user guide](../user-guide/languages.md) records the exact set, file extensions, server
defaults, installation sources and LSP limits.

## Implementation

The existing language registry now routes all fifteen languages, JSX/TSX and Terraform's
supported HCL dialects. JavaScript/TypeScript share a server family, as do C/C++; generic HCL
has its own default-disabled family instead of being sent to an incompatible Terraform server.
Loose-file root discovery includes the new build manifests and .NET solution/project files.

Ten grammar crates extend the existing lazy parser registry without a new worker or runtime:

| Crate | Version |
| --- | --- |
| `tree-sitter-c` | 0.24.2 |
| `tree-sitter-cpp` | 0.23.4 |
| `tree-sitter-c-sharp` | 0.23.5 |
| `tree-sitter-java` | 0.23.5 |
| `tree-sitter-php` | 0.25.1 |
| `tree-sitter-ruby` | 0.23.1 |
| `tree-sitter-swift` | 0.7.4 |
| `tree-sitter-kotlin-ng` | 1.1.0 |
| `tree-sitter-bash` | 0.25.1 |
| `tree-sitter-hcl` | 1.1.0 |

Queries come from those crates, except the small Kotlin and HCL queries authored against their
packaged `node-types.json`. C++ combines its query with C's. Capture categories map to Orlo's
existing color schemes and scope selectors. New syntax names also work in View > Syntax and
comment commands. Installed grammars retain precedence. Missing fallback syntaxes stay plain;
only TypeScript deliberately uses JavaScript's Syntect fallback.

Independent review identified a PHP template regression before completion. A failing `.phtml`
test reproduced HTML colors disappearing after the tree query. PHP's `(text)` captures now
retain whole-line Syntect colors and scopes for embedded HTML/CSS/JavaScript, including inline
PHP. Queried fallback entries prevent repeat worker requests and keep Copy as HTML batching
working. Pure PHP lines still use the incremental tree. The reviewer rechecked this change and
reported no remaining Critical or Required findings.

## Verification coverage

Automated checks cover parser/query compilation, representative Unicode documents, token
colors, Kotlin literals and control-flow words, menu names, comment markers, installed grammar
precedence, incremental edits versus fresh highlighting, stale jobs, cancellation and bounded
fallback. Fixture subprocesses verify completion and `didOpen` IDs for all 24 registered
language/dialect variants across 14 server families; root tests cover the new manifests.

Real stdio hover/completion checks passed in C, C++, Swift, Bash and PHP with Apple clangd
21.0.0, SourceKit-LSP from the Swift 6.4 command-line toolchain, Bash Language Server 5.8.1 and
Intelephense 1.18.5. The npm servers were installed under ignored
`target/language-support/lsp/npm`, without changing global installations. Java, C#, Kotlin,
Ruby and Terraform have fixture coverage; their real servers were not installed or tested.
The previous four-family real-server suite still requires its separate tool fixture.

Native computer-use checks on the rebuilt app verified Kotlin, Swift, C#, HCL and PHP template
colors, all fifteen menu entries, Kotlin comment toggle and undo, HCL Unicode save and undo,
and a clangd completion displayed, applied and saved in a C buffer. C# and Kotlin remained
editable with their server tools absent. All editing used disposable files under
`/tmp/orlo-language-support`. The app was closed afterward and the original support folder
was restored with byte-for-byte verification; test session/settings were retained separately.

## Final checks

- `cargo test --workspace --locked`: 1,763 passed, zero failed; 23 opt-in tests ignored.
- `cargo fmt --all -- --check`, `git diff --check` and strict workspace Clippy passed.
- `cargo test -p orlo-syntax --locked real_language_servers_smoke_added_families -- --ignored
  --nocapture` with the test-local npm prefix on PATH: passed for all five file cases.
- `./scripts/bundle.sh`: optimized build and strict ad-hoc bundle signature verification passed.
  The final Kotlin query was also checked in both the binary and bundled executable.
- `target/alpha-tools/bin/cargo-audit audit`: zero reported vulnerabilities; the existing 15
  unmaintained and two soundness warnings remain. Existing Rust future-compatibility warnings
  for `block` and `proc-macro-error2` remain unsuppressed.

Logs are under ignored `target/language-support/`. The initial 21-gate release performance run
overlapped release compilation: 20 passed, while Goto Symbol took 6.424 s against its 3 s budget.
After all compilation and other tests finished, the same release harness reran the failing
`bench_find` gate in isolation and passed: Goto Symbol took 2.664 s. No budgets were changed.

```sh
target/release/deps/orlo_ui-1fa11e99a376f51e --ignored --exact testing::bench::editing::bench_find --nocapture --test-threads=1
```

The isolated run is `performance-find-isolated.log`; the initial full run is `performance.log`.
The grammar-only Kotlin literal query refinement was verified by the final workspace/syntax
tests and bundled build; the performance harness predates that query-only refinement. Its Rust
symbol workload and performance paths were unchanged. Java's upstream launcher cache collision
for roots sharing a directory name remains documented in the setup guide.
