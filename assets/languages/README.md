# Bundled languages

Each folder here is one language, in the same format as a user's
`~/Library/Application Support/Orlo/Languages/<id>/` folder. They are compiled into Orlo
(`crates/syntax/build.rs`), so a build needs no network and no `.app` resources.
A user folder with the same id replaces a bundled one whole.

To add one, start from `docs/templates/example-language/` and follow
[Adding a language](../../docs/user-guide/adding-a-language.md). Each folder that holds
third-party files has a `LICENSE` saying what came from where. A `grammar.wasm` is rebuilt from
its manifest's `tree_sitter.source` by `scripts/build-grammar.sh <id>`. CI checks every folder
with `Orlo --check-language assets/languages/*/`.
