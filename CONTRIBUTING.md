# Contributing

Thanks for helping with Orlo. Bug reports, fixes and focused features are welcome; for a large
change, open an issue first so the approach can be agreed before the work.

- **Where things live**: the [source map](docs/architecture/overview.md) names the crate and
  module for each part of the editor.
- **Building**: macOS with the toolchain pinned in `rust-toolchain.toml`, and `cmake`
  (`brew install cmake`). `cargo run -p orlo-app -- .` opens this folder.
- **Before a pull request**, run the checks CI runs
  ([development checks](docs/development/verification.md)):

  ```sh
  cargo fmt --all -- --check
  cargo clippy --workspace --locked --all-targets -- -D warnings
  cargo test --workspace --locked
  ```

- **Tests**: a bug fix comes with a test that fails without it. Performance work is measured
  against the release benchmarks described in the development checks.
- **Security issues** go through [SECURITY.md](SECURITY.md), not public issues.

By contributing you agree that your work is released under the [MIT licence](LICENSE).
