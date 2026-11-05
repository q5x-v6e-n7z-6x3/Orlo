# Runtime grammar fixtures

Built with the tree-sitter CLI 0.27.1 (`tree-sitter build --wasm`).

- `json-wasm/`: tree-sitter-json v0.24.8 (`ee35a6eb`), MIT, its `queries/highlights.scm` unchanged.
- `json-abi12.wasm`: the same grammar with `LANGUAGE_VERSION` in `src/parser.c` set to 12 before
  building, which no Orlo accepts.
- `trap/`: Orlo's own; `src/` is its source (`tree-sitter generate src/grammar.json`, then build).
  Its external scanner traps on `!`.
