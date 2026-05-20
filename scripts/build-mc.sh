#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MC_DIR="$ROOT_DIR/crates/mc"
TARGET_BIN="${MC_TARGET_BIN:-$HOME/.local/bin/mc}"

echo "[mc-build] building release binary..."
cargo build --release --manifest-path "$MC_DIR/Cargo.toml"

echo "[mc-build] installing to $TARGET_BIN"
mkdir -p "$(dirname "$TARGET_BIN")"
cp "$MC_DIR/target/release/mc" "$TARGET_BIN"
chmod +x "$TARGET_BIN"

echo "[mc-build] done"
"$TARGET_BIN" --version
