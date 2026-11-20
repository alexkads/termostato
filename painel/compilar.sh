#!/usr/bin/env bash
# Compila o painel para o navegador e deixa em dist/, de onde o termostato o
# embute (include_bytes!). Rode depois de mudar qualquer coisa em painel/.
#   painel/compilar.sh
# Precisa de: rustup target add wasm32-unknown-unknown
#             cargo install wasm-bindgen-cli --version 0.2.130   (igual ao Cargo.toml)
# Opcional:   wasm-opt (binaryen), que encolhe o .wasm.
set -euo pipefail
cd "$(dirname "$0")"
alvo="${CARGO_TARGET_DIR:-target}"
nice -n 10 cargo build --lib --target wasm32-unknown-unknown --profile wasm-release -j "${JOBS:-2}"
rm -rf dist && mkdir -p dist
wasm-bindgen --target web --no-typescript --out-dir dist --out-name painel \
  "$alvo/wasm32-unknown-unknown/wasm-release/painel.wasm"
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Oz --enable-bulk-memory --enable-mutable-globals --enable-multivalue \
    --enable-nontrapping-float-to-int --enable-reference-types --enable-sign-ext \
    -o dist/painel_bg.wasm dist/painel_bg.wasm
fi
cp web/index.html web/favicon.svg dist/
tamanho=$(wc -c < dist/painel_bg.wasm | tr -d ' ')
gz=$(gzip -9 -c dist/painel_bg.wasm | wc -c | tr -d ' ')
echo "painel em dist/ (${tamanho} bytes; ${gz} gzip)"
