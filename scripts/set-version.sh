#!/usr/bin/env bash
# Bump edgeplane / edgeplaned / edgeplane-tower to a unified version, in lockstep.
# Source of truth: /VERSION at the repo root.
# CI asserts that VERSION + the three Cargo.toml [workspace.package] versions
# all agree (see .github/workflows/version-sync.yml).
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <new-version>" >&2
  echo "example: $0 0.12.0" >&2
  exit 2
fi

new="$1"
if ! [[ "$new" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$ ]]; then
  echo "error: '$new' is not a valid semver (e.g. 0.11.0, 1.0.0-rc.1)" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

echo "$new" > VERSION

for crate in edgeplane edgeplaned edgeplane-tower; do
  toml="crates/$crate/Cargo.toml"
  if [[ ! -f "$toml" ]]; then
    echo "error: $toml not found" >&2
    exit 1
  fi
  # Update the single [workspace.package] version line. Each of the three
  # Cargo.toml files starts with [workspace] then [workspace.package];
  # the first `version = "X"` after that block is the one we want.
  python3 - "$toml" "$new" <<'PY'
import re, sys
path, new = sys.argv[1], sys.argv[2]
text = open(path).read()
# Match the workspace.package section's version line.
pat = re.compile(
    r'(\[workspace\.package\][^\[]*?\nversion\s*=\s*")[^"]+(")',
    re.DOTALL,
)
new_text, n = pat.subn(rf'\g<1>{new}\g<2>', text, count=1)
if n != 1:
    print(f"error: did not find [workspace.package] version in {path}", file=sys.stderr)
    sys.exit(1)
open(path, "w").write(new_text)
PY
done

echo "Bumped VERSION + 3 workspaces to $new"
echo "Run cargo check in each workspace to confirm."
