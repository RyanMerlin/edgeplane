#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MC_DIR="$ROOT_DIR/crates/edgeplane"
TARGET_BIN="${MC_TARGET_BIN:-$HOME/.local/bin/edgeplane}"

echo "[edgeplane-build] building release binary..."
cargo build --release --manifest-path "$MC_DIR/Cargo.toml"

echo "[edgeplane-build] installing to $TARGET_BIN"
mkdir -p "$(dirname "$TARGET_BIN")"
cp "$MC_DIR/target/release/edgeplane" "$TARGET_BIN"
chmod +x "$TARGET_BIN"

echo "[edgeplane-build] done"
"$TARGET_BIN" --version
