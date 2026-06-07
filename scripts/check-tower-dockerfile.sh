#!/usr/bin/env bash
# Guard: every [workspace] member must have its Cargo.toml COPY'd into the tower
# Dockerfile. The Dockerfile COPYs + stubs each member individually to cache deps;
# if a member is missing, `cargo build -p edgeplane-tower` fails to load the
# workspace ("failed to load manifest for workspace member ...") at image-build time.
#
# Why a dedicated guard: build-image.yml runs only on main-push + tags, NOT on PRs,
# so this drift passes PR CI and breaks only after merge / on the release tag. It has
# bitten twice (gen-openapi, edgeplane-zrpc-proto → the v0.13.0 image never published).
# This check runs on PRs so the drift fails fast, before merge.
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

DOCKERFILE="crates/edgeplane-tower/Dockerfile"

python3 - "$DOCKERFILE" <<'PY'
import sys, tomllib

dockerfile = sys.argv[1]
with open("Cargo.toml", "rb") as f:
    members = tomllib.load(f)["workspace"].get("members", [])

df = open(dockerfile).read()
missing = [m for m in members if f"COPY {m}/Cargo.toml " not in df]

if missing:
    print(f"::error::{dockerfile} is missing COPY lines for {len(missing)} workspace member(s):")
    for m in missing:
        print(f"  COPY {m}/Cargo.toml ./{m}/Cargo.toml      (and add '{m}' to the stub-dir loop below it)")
    print("")
    print("Every [workspace] member must be COPY'd + stubbed in the tower Dockerfile, or")
    print("`cargo build -p edgeplane-tower` fails to load the workspace at image-build time.")
    print("Note: build-image.yml runs only on main-push/tags, so without this guard the")
    print("breakage would not surface until after merge / on the release tag.")
    sys.exit(1)

print(f"ok: all {len(members)} workspace members are present in {dockerfile}")
PY
