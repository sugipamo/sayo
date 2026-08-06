#!/usr/bin/env sh
set -eu

cargo build --release --lib --target wasm32-unknown-unknown
wasm-bindgen \
  --target web \
  --out-dir playground/pkg \
  target/wasm32-unknown-unknown/release/sayo.wasm
