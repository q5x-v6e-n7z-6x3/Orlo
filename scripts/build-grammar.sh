#!/bin/sh
# Build assets/languages/<id>/grammar.wasm from the repository and revision its language.json
# pins in tree_sitter.source. Needs git and tree-sitter CLI 0.27.1 (it fetches wasi-sdk itself).
set -eu
id=${1:?usage: scripts/build-grammar.sh <language id>}
cd "$(dirname "$0")/.."
folder="assets/languages/$id"
manifest="$folder/language.json"
[ -f "$manifest" ] || { echo "$manifest: no such file" >&2; exit 1; }
field() { plutil -extract "tree_sitter.source.$1" raw -o - "$manifest" 2>/dev/null; }
repository=$(field repository) || { echo "$manifest: no tree_sitter.source.repository" >&2; exit 1; }
revision=$(field revision) || { echo "$manifest: no tree_sitter.source.revision" >&2; exit 1; }
case "$revision" in
  *[!0-9a-f]* | "") echo "$manifest: revision $revision must be a full commit SHA" >&2; exit 1 ;;
esac
[ ${#revision} -eq 40 ] || { echo "$manifest: revision $revision must be a full commit SHA" >&2; exit 1; }
version=$(tree-sitter --version | awk '{print $2}')
[ "$version" = 0.27.1 ] || { echo "tree-sitter CLI 0.27.1 required, found $version" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git -C "$work" init -q
git -C "$work" fetch -q --depth 1 "$repository" "$revision"
git -C "$work" -c advice.detachedHead=false checkout -q FETCH_HEAD
[ "$(git -C "$work" rev-parse HEAD)" = "$revision" ] || { echo "$repository: fetched the wrong revision" >&2; exit 1; }

output="$PWD/$folder/grammar.wasm"
(cd "$work" && tree-sitter build --wasm -o "$work/grammar.wasm" .)
[ "$(head -c 4 "$work/grammar.wasm" | od -An -tx1 | tr -d ' \n')" = 0061736d ] \
  || { echo "$work/grammar.wasm: not a WebAssembly module" >&2; exit 1; }
mv "$work/grammar.wasm" "$output"
echo "$output: $(wc -c <"$output" | tr -d ' ') bytes from $repository@$revision"
