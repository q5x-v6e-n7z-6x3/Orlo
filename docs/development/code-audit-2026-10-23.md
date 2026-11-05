# Code audit, 2026-10-23

This maintenance pass reviews all tracked first-party Rust source and tests, the Python
language-server fixtures, bundled grammar/query assets, Cargo manifests, CI and the bundle
script. The workload is one person editing local macOS projects, including the existing large
file benchmarks and a window with 1,024 documents in 2,048 views. Generated `target/` content,
binary assets and third-party dependency implementations are outside the source review.

The source audit, repairs, independent patch review, full local verification and native smoke
check are complete on `fix/full-code-audit`.

## Review coverage

The initial Rust inventory at `fef782ba8daa0368c0a8677aef6680a41d547048` is **248 files and
137,591 lines**, including tests and benchmarks. Coverage is divided by ownership: core editor,
configuration, Git, terminal and utilities; filesystem, recovery and syntax services; editor UI
and panels; workspace UI; and application startup, integration tests and benchmarks. Every file
is read in full, rather than treating a pattern search or green test suite as source-review
coverage. Final patch review covers changed lines after the original file reads.

All 248 Rust files have been read completely, including every inline test. The additional
21 source/configuration files comprise two Python fixtures, four grammar/query assets,
12 TOML files, Cargo.lock, the bundle script and CI. The final manifest records baseline and
final line counts and final SHA-256 hashes for all 269 reviewed files.
The [tab-separated manifest](code-audit-2026-10-23-manifest.tsv) identifies each file; coverage
means lines 1 through EOF at baseline plus subsequent patch review, including added tests.
The final Rust inventory is **139,392 lines**. The manifest's SHA-256 is
`71197db5dcd4a93ac45a1e3756d82965af6e16c34e2703f747979c3f159aa3d7`.

## Branch cleanup

After fetching and checking ancestry against `origin/main`, deleted `feat/holyc-freebasic`
locally and remotely. Its tip, `8e15d0619aa2bc8c523562f56ffc129fc230661c`, is contained in
`main` at `fef782ba8daa0368c0a8677aef6680a41d547048`. The default branch was retained.
Repairs are on `fix/full-code-audit`.

## Repairs

- **Recovery and privacy:** session JSON is private, recovery drafts cannot follow leaf or
  directory symlinks, and a FIFO at the session path cannot block startup. Backup traversal
  refuses a symlinked draft directory rather than moving unrelated files.
- **Launch boundary:** the handoff socket is private and authenticates the peer's kernel UID.
  File-open URLs reject remote hosts, malformed escapes, NULs and unescaped query/fragment
  components.
- **Filesystem operations:** valid long filenames can be saved atomically, including temp-file
  cleanup; copying a folder into its own descendant is rejected before staging a recursive copy.
- **Editor state:** adopting a shared view preserves its highlights. Very large column offsets
  clamp without overflowing. Line deletion, bookmarked-line deletion, transformations and
  final-newline cleanup retain complete CRLF boundaries. Transpose retains whole graphemes.
  Long flag runs retain grapheme pairing, out-of-range cluster navigation clamps safely,
  and fold-end mapping retains whole CRLF pairs. Accented-word selection and literal
  whole-word search share the same continuation rule, including cached and scoped searches.
  Scoped search checks full-document word boundaries before consuming a candidate, so a
  rejected prefix cannot hide a later valid overlapping match.
- **Structured editing:** CSV column operations preserve literal trailing carriage returns;
  Smart Quotes handles an opening quote immediately inside the other kind of opening quote.
  Deterministic parser/transform regressions exercise CSV, substitutions and snippet mirrors.
- **Presentation and tabs:** Git patch snapshots reject editing. Multiple fold placeholders
  retain valid UTF-8 when drawing whitespace marks. Pinned blank/shared views survive bulk
  closes, file opens and tab adoption. Option-drag ignores closed tabs and focuses the grabbed
  tab's copy. Reloading customizations preserves an explicitly chosen Plain Text syntax.
- **Session restoration:** deferred reads respect active file writers. Tab ordering, pinned
  ordering and active identity are preserved across background completion and captures made
  while restoration is pending. A newer user selection supersedes the saved active tab,
  including selection through tabs, keyboard, direct editor focus and existing panel views;
  both capture and later background completion retain that choice.
- **Test safety:** buffer fixtures use exclusively created private per-run directories, so a
  second test process cannot recursively remove the first process's files.
- **Dependencies and CI:** Tokio is updated from 1.53.2 to 1.53.3. CI has read-only repository
  contents permissions, does not persist checkout credentials, and pins the existing action
  versions to commits resolved from their official repositories.

The source findings were reproduced with failing regression tests before narrow repairs.
Independent review additionally checks the resulting diff; the final test results below
supersede intermediate runs made while other reviewers were editing.
Final review objections about wrapping complexity, pinned restore order, pending active-tab
capture, accented whole-word continuation and scoped overlapping matches are resolved. The
last scoped-search delta passed six independent focused tests; its preceding continuation
review also checked 480 actual editor cache-edit cases without a cache mismatch.

## Performance

The foreground part of the 500 ms disk poll previously compared each view with every document
already collected. It now keeps a set of document entity IDs, preserving the original first-view
order while avoiding repeated pairwise comparisons.

An identical release benchmark with 1,024 documents and 2,048 views measured three alternating
old/new runs. Each run has seven samples of ten polls; medians across 21 samples were
**2.360 ms before and 0.149 ms after**, a **15.8× speedup** in this foreground workload.
Old samples ranged from 2.281–3.291 ms; new samples ranged from 0.146–0.153 ms. Disk work and
subprocesses were already in flight, so these numbers isolate UI scheduling/deduplication work
and are not an end-to-end filesystem-latency claim. The ignored release benchmark enforces
a 1 ms median budget.

The Unicode wrapping correction computes grapheme boundaries lazily once per line. This
removes repeated segmentation of growing prefixes while retaining the normal ASCII/CJK
fast path. A deterministic visit-count regression checks linear scanning at 8k, 16k and
32k characters, alongside exact row-output regressions for multiple scripts and graphemes.

Independent review caught quadratic work in an intermediate correctness repair. Optimized
isolated probes comparing that intermediate version with the final functions measured a
32k Arabic line at **71.755 ms versus 402.458 µs**; a complete repeated run measured
67.172 ms versus 361.041 µs. These are comparisons against the intermediate repair, not
the original HEAD. Every measured before/after row vector matched. An eager-cache attempt
regressed 32k CJK from 67.542 µs to 410.375 µs and was discarded. The retained lazy version
measured CJK at 68.292 µs versus 69.542 µs; repeated measurements were 67.541 µs versus
66.750 µs. Full raw measurements and the rejected attempt remain in the local evidence.

## Verification

Final formatting and strict Clippy passed. The full workspace suite passed **1,995 tests,
0 failures**, with 27 ignored release/external-server checks, in 118.7 seconds. This includes
real application signal/hot-exit behavior and fixture language-server subprocesses.
The optimized workspace build also passed (106.638 seconds). All **25 release performance
gates passed**, serially, after other tests and compilation. The complete release-test command
took 363.183 seconds, including test-binary compilation. The two ignored real-server smoke
tests were excluded; performance checks, including completion ranking and the find-panel
keystroke check, were included. The final disk-poll median was **0.1486 ms**, below its 1 ms
budget. No existing budget or test assertion was weakened.

The bundle script completed and strict `codesign` verification passed with the default ad-hoc
identity. Native computer use verified Rust syntax, accented text and emoji rendering, and two
highlighted `alpha` matches. Cmd-D twice on decomposed `café cafés café`, followed by replacing
both selected words with `ok`, left the middle word untouched. Save was checked against exact
on-disk UTF-8 content; native Undo and Save restored the original bytes. Both audit fixtures
were left intact, and the window returned to normal mode. The initial captured search state
was refreshed with a window-size transition; this smoke check does not measure frame latency.
The running app's session file and handoff socket both had mode **0600**.

The final checks use the manifest's source hashes. Debug and release test counts are reported
separately; intentionally failing pre-repair reproductions are not counted as passing tests.
All 269 source/configuration hashes remained unchanged through the checks and native smoke.

Commands used for the final gates:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked --release
ORLO_INDEX_TREE="$PWD/crates" cargo test --workspace --locked --release -- --ignored --skip real_language_servers_smoke --nocapture --test-threads=1
git diff --check
./scripts/bundle.sh
```

The installed local `cargo-audit 0.22.2` also completed successfully against Cargo.lock.
Final command statuses and timings: `target/audit-final-gate-results.json`; complete output:
`target/audit-final-tests.log`, `target/audit-clippy.log`,
`target/audit-final-release-build.log`, `target/audit-release-gates.log`,
`target/audit-final-bundle.log` and `target/audit-dependencies-final.json`.

## Evidence and limits

Detailed source ledgers, failing/passing regression logs, independent review and measurements
are retained under ignored `target/audit-*` paths. The native fixture is
`target/audit-native-smoke.rs`; the support-folder snapshot made before the later native checks
is retained at `target/audit-native-support-before/`.

The dependency audit reports **0 vulnerability entries**, **15 unmaintained package/version
notices** and **2 unsoundness notices**, without suppressions. Advisory database commit:
`84d943a6deeb8cc6c6f632531aaf4f1c62275239`, updated 2026-10-23. The remaining two notices,
RUSTSEC-2026-0183 and RUSTSEC-2026-0184, concern `git2 0.20.4` from GPUI's cross-platform
dependency graph; it is absent from the macOS build graph. Moving that transitive dependency
to the patched major/minor line requires a GPUI compatibility change and is outside these
narrow repairs. Existing Rust future-incompatibility warnings for `block 0.1.6` and
`proc-macro-error2 2.0.1` remain.

No source audit finding is deferred merely because it is rare: the CSV trailing-CR case was
reproduced and repaired. Changes to diacritic-removal policy, contextual case conversion and
display-column conventions were considered but no contract mismatch was established. The
existing repeated bookmark-toggle behavior is intentionally pinned by a Sublime compatibility
test and was retained. A pre-existing 100 ms test timing assertion passed; no reproducible
flakiness was established and its budget was retained.

The source review does not prove the absence of latent bugs. Real external language servers,
other macOS versions, Intel hardware, multiple displays, VoiceOver, notarization and downloaded
app installation are outside this pass. IPC peer checks use real same-user sockets and a
different expected UID; a second-user end-to-end test was not performed. Recovery backup
traversal is not claimed to resist a malicious process of the same UID racing its private
directory.

References: [GitHub workflow security](https://docs.github.com/en/actions/reference/security/secure-use),
[Tokio I/O safety correction](https://github.com/tokio-rs/tokio/pull/8587),
[git2 remote-list notice](https://rustsec.org/advisories/RUSTSEC-2026-0183.html),
[git2 blame notice](https://rustsec.org/advisories/RUSTSEC-2026-0184.html).
