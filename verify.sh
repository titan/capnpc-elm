#!/usr/bin/env bash
# verify.sh — capnpc-elm3 verification entry.
#
# Usage:
#   ./verify.sh          cargo test + full E2E (when test-project/ exists)
#   ./verify.sh --fast   cargo test only
#
# E2E always regenerates Elm code with target/debug/capnpc-elm — NOT the
# installed /usr/local/bin snapshot — so what gets verified is HEAD.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

echo '==> cargo test'
cargo test

if [[ "${1:-}" == '--fast' ]]; then
    echo 'FAST MODE GREEN'
    exit 0
fi

if [[ ! -d test-project ]]; then
    echo '==> test-project/ absent (gitignored) — skipping E2E'
    echo 'GREEN (unit tests only)'
    exit 0
fi

echo '==> cargo build'
cargo build

echo '==> regenerate Elm code + elm make'
(
    cd test-project/frontend
    PATH="$ROOT/target/debug:$PATH" capnp compile \
        -oelm:src ../schema/test.capnp --src-prefix=../schema
    elm make src/Main.elm --output=main.js
)

echo '==> E2E test.ts'
(cd test-project/verify && bun test.ts)

echo '==> E2E test_interop.ts'
(cd test-project/verify && bun test_interop.ts)

echo 'ALL GREEN'
