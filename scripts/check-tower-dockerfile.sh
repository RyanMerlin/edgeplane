#!/usr/bin/env bash
# Guard for crates/edgeplane-tower/Dockerfile. Two invariants, both checked on PRs:
#
#   1. MANIFEST PRESENCE — every [workspace] member must have its Cargo.toml COPY'd
#      into the Dockerfile. The Dockerfile COPYs + stubs each member to cache deps;
#      a missing member makes `cargo build -p edgeplane-tower` fail to load the
#      workspace ("failed to load manifest for workspace member ...").
#
#   2. SOURCE RESTORATION — every workspace crate the tower actually depends on
#      (its path-deps, transitively) must have its REAL source COPY'd back in after
#      the stub stage, i.e. a `COPY <crate-dir> ./<crate-dir>` directory copy, not
#      just the `COPY <crate-dir>/Cargo.toml ...` line. The Dockerfile stubs every
#      member's src to `pub fn stub() {}` to cache the dependency build; if a crate
#      whose *functions* the tower calls is left stubbed, the final build fails with
#      `error[E0425]: cannot find function ...`. This bit us when #69 added the first
#      tower->edgeplaned-paths call and only the tower's own source was restored.
#
# Why a dedicated guard: build-image.yml runs only on main-push + tags, NOT on PRs,
# so this drift passes PR CI and breaks only after merge / on the release tag (it has
# bitten three times now: gen-openapi, edgeplane-zrpc-proto, edgeplaned-paths). This
# check runs on PRs so the drift fails fast, before merge.
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

DOCKERFILE="crates/edgeplane-tower/Dockerfile"

python3 - "$DOCKERFILE" <<'PY'
import os, re, sys, tomllib

dockerfile = sys.argv[1]
df = open(dockerfile).read()

with open("Cargo.toml", "rb") as f:
    members = tomllib.load(f)["workspace"].get("members", [])

errors = []

# ── Invariant 1: every workspace member's Cargo.toml is COPY'd ───────────────
missing_manifest = [m for m in members if f"COPY {m}/Cargo.toml " not in df]
if missing_manifest:
    errors.append(
        f"{dockerfile} is missing Cargo.toml COPY lines for "
        f"{len(missing_manifest)} workspace member(s):"
    )
    for m in missing_manifest:
        errors.append(
            f"  COPY {m}/Cargo.toml ./{m}/Cargo.toml   "
            f"(and add '{m}' to the stub-dir loop below it)"
        )

# ── Invariant 2: real source restored for the tower's path-deps (transitive) ─
# Build the set of workspace crates reachable from edgeplane-tower via path deps,
# then require each to have a directory-COPY restoring its real source.
member_set = set(members)

def path_deps(crate_dir):
    """Workspace crate dirs that `crate_dir` depends on via `path = "..."`."""
    manifest = os.path.join(crate_dir, "Cargo.toml")
    if not os.path.exists(manifest):
        return []
    with open(manifest, "rb") as fh:
        data = tomllib.load(fh)
    out = []
    for table in ("dependencies", "build-dependencies", "dev-dependencies"):
        for spec in data.get(table, {}).values():
            if isinstance(spec, dict) and "path" in spec:
                resolved = os.path.normpath(os.path.join(crate_dir, spec["path"]))
                if resolved in member_set:
                    out.append(resolved)
    return out

root = "crates/edgeplane-tower"
reachable, queue = set(), [root]
while queue:
    cur = queue.pop()
    for dep in path_deps(cur):
        if dep not in reachable:
            reachable.add(dep)
            queue.append(dep)

# A directory-restore COPY is `COPY <dir> <dest>` — distinct from the
# `COPY <dir>/Cargo.toml <dest>` manifest line.
def has_source_restore(crate_dir):
    return re.search(rf"(?m)^COPY {re.escape(crate_dir)}\s+\S", df) is not None

missing_source = [c for c in sorted(reachable) if not has_source_restore(c)]
if missing_source:
    errors.append(
        f"{dockerfile} stubs but never restores real source for "
        f"{len(missing_source)} crate(s) the tower depends on:"
    )
    for c in missing_source:
        errors.append(
            f"  COPY {c} ./{c}   "
            f"(tower calls into it — the stub `pub fn stub(){{}}` breaks the build)"
        )

if errors:
    print(f"::error::{errors[0]}")
    for line in errors[1:]:
        print(line)
    print("")
    print("Every [workspace] member must be COPY'd + stubbed, AND every crate the")
    print("tower depends on must have its real source restored before the final")
    print("`cargo build -p edgeplane-tower`. build-image.yml runs only on main-push/")
    print("tags, so without this guard the breakage would not surface until after")
    print("merge / on the release tag.")
    sys.exit(1)

print(
    f"ok: all {len(members)} workspace members present; real source restored for "
    f"{len(reachable)} tower path-dep(s): {', '.join(sorted(reachable)) or '(none)'}"
)
PY
