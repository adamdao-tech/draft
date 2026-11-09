#!/usr/bin/env bash
# Builds the browser version into dist/. Needs the wasm32-unknown-unknown Rust target and
# wasm-bindgen-cli at the version in Cargo.lock; wasm-opt (binaryen) is used if present.
# Extra cargo arguments can go in CARGO_EXTRA.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --profile wasm-release --target wasm32-unknown-unknown ${CARGO_EXTRA:-}

rm -rf dist
mkdir -p dist/pkg dist/fonts dist/champion_icons
wasm-bindgen --target web --no-typescript --out-dir dist/pkg \
  target/wasm32-unknown-unknown/wasm-release/draftree.wasm

if command -v wasm-opt >/dev/null; then
  wasm-opt -Os --enable-bulk-memory --enable-nontrapping-float-to-int --enable-reference-types \
    --enable-sign-ext --enable-mutable-globals \
    dist/pkg/draftree_bg.wasm -o dist/pkg/draftree_bg.wasm
fi

cp web/index.html web/_headers dist/
cp assets/icon/recvrve.svg dist/icon.svg
cp assets/fonts/LexendDeca-Variable.ttf dist/fonts/LexendDeca.ttf
cp champion_icons/*.png dist/champion_icons/
du -sh dist dist/pkg/draftree_bg.wasm
