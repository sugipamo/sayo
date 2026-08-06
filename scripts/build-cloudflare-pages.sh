#!/usr/bin/env sh
set -eu

WASM_BINDGEN_VERSION="0.2.126"

if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable
  export PATH="${PATH}:${HOME}/.cargo/bin"
fi

rustup toolchain install stable --profile minimal
export RUSTUP_TOOLCHAIN="stable"
rustup target add wasm32-unknown-unknown --toolchain stable

installed_wasm_bindgen="$(wasm-bindgen --version 2>/dev/null || true)"
if [ "$installed_wasm_bindgen" != "wasm-bindgen ${WASM_BINDGEN_VERSION}" ]; then
  cargo install wasm-bindgen-cli --version "$WASM_BINDGEN_VERSION" --locked
fi

./scripts/build-wasm.sh
