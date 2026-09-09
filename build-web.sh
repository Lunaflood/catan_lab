#!/bin/sh
# WASM をビルドして web/ に配置する。外部ツールは要らない。
set -e
cargo build -p catan-wasm --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/catan_wasm.wasm web/
cp target/wasm32-unknown-unknown/release/catan_wasm.wasm web/colonist/
ls -la web/catan_wasm.wasm
