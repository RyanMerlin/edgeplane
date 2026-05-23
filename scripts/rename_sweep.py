#!/usr/bin/env python3
"""
rename_sweep.py — Two-step token rename for the Mission→Domain, Kluster→Mission swap.

Usage:
  python3 scripts/rename_sweep.py --step A1   # mission → domain
  python3 scripts/rename_sweep.py --step A2   # kluster → mission
  python3 scripts/rename_sweep.py --dry-run --step A1  # preview only

Strategy (A1):
  1. Protect brand compound words with placeholders
  2. Replace Mission→Domain, mission→domain, MISSION→DOMAIN
  3. Restore placeholders

Strategy (A2):
  Replace Kluster→Mission, kluster→mission, KLUSTER→MISSION (no brand exclusions needed)

Files processed: .rs, .toml (Cargo configs)
Excluded: target/, .git/, .worktrees/, .sqlx/, migrations/ (SQL files keep history)
"""

import argparse
import os
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).parent.parent

# Brand/binary strings that must NOT be replaced (A1 only).
# Placeholders must NOT contain "Mission", "mission", or "MISSION" — otherwise
# the main substitution step would corrupt the placeholder before restoration.
A1_PROTECT = [
    # Order matters: longer/more-specific first to avoid partial matches on restore.
    # TUI screen: MissionMatrix stays as-is — post-A2 "mission" means workstream, so the name is correct.
    ("MissionMatrix", "__PROTECT_MC_MATRIX_PASCAL__"),
    ("mission_matrix", "__PROTECT_MC_MATRIX_SNAKE__"),
    # Brand compound words — joined and spaced variants
    ("Mission Control", "__BRAND_MC_SPACE_PASCAL__"),   # spaced PascalCase in strings/docs
    ("MISSION CONTROL", "__BRAND_MC_SPACE_UPPER__"),    # spaced all-caps (TUI banner)
    ("mission control", "__BRAND_MC_SPACE_LOWER__"),    # spaced lowercase (defensive)
    ("MissionControl", "__BRAND_MC_FULL__"),      # joined PascalCase brand
    ("Missioncontrol", "__BRAND_MC_MIXED__"),     # first-char-cap variant in CLI structs
    ("missioncontrol", "__BRAND_MC_LOWER__"),     # all-lowercase brand (identifiers, paths)
    ("MISSIONCONTROL", "__BRAND_MC_UPPER__"),     # all-caps (unlikely but defensive)
    ("mission-control", "__BRAND_MC_KEBAB__"),    # kebab in docs/help text
    # MC_ prefix doesn't contain "mission" so no protection needed.
    # "mc", "mcd", "mc-controlplane" don't contain "mission" either.
]

# Directories to skip entirely
SKIP_DIRS = {"target", ".git", ".worktrees", ".sqlx", ".venv", "__pycache__"}

# File extensions to process
PROCESS_EXTENSIONS = {".rs", ".toml"}

# Files to skip explicitly (migration files preserve history)
SKIP_FILES = {
    "migrations",  # skip any path component named migrations
}


def should_skip_path(path: Path) -> bool:
    """Return True if this path should be skipped."""
    parts = path.parts
    for part in parts:
        if part in SKIP_DIRS:
            return True
        if part == "migrations":
            return True
    return False


def apply_a1(content: str) -> str:
    """Apply mission → domain substitution with brand exclusions."""
    # Step 1: protect brand compound words
    for original, placeholder in A1_PROTECT:
        content = content.replace(original, placeholder)

    # Step 2: case-preserving replacements
    # PascalCase first. str.replace is safe here: "Permissions" has lowercase 'm' so
    # replace("Mission", "Domain") never touches it.
    content = content.replace("Mission", "Domain")
    # Lowercase: use lookbehind to skip substrings where "mission" is preceded by a
    # lowercase letter — catches "permissions", "submission", "emission" etc.
    content = re.sub(r'(?<![a-z])mission', 'domain', content)
    # ALL-CAPS: same guard — "PERMISSIONS" has uppercase 'R' before "MISSION".
    content = re.sub(r'(?<![A-Z])MISSION', 'DOMAIN', content)

    # Step 3: restore placeholders
    for original, placeholder in A1_PROTECT:
        content = content.replace(placeholder, original)

    return content


def apply_a2(content: str) -> str:
    """Apply kluster → mission substitution (no brand exclusions needed)."""
    content = content.replace("Kluster", "Mission")
    content = content.replace("kluster", "mission")
    content = content.replace("KLUSTER", "MISSION")
    return content


def process_file(path: Path, step: str, dry_run: bool) -> bool:
    """Process a single file. Returns True if content changed."""
    try:
        original = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, PermissionError):
        return False

    if step == "A1":
        updated = apply_a1(original)
    elif step == "A2":
        updated = apply_a2(original)
    else:
        raise ValueError(f"Unknown step: {step}")

    if updated == original:
        return False

    if dry_run:
        print(f"  [WOULD CHANGE] {path}")
        # Show a few changed lines for preview
        orig_lines = original.splitlines()
        upd_lines = updated.splitlines()
        shown = 0
        for i, (ol, ul) in enumerate(zip(orig_lines, upd_lines)):
            if ol != ul and shown < 5:
                print(f"    -{ol.strip()}")
                print(f"    +{ul.strip()}")
                shown += 1
        if shown == 5:
            print("    ... (more changes)")
    else:
        path.write_text(updated, encoding="utf-8")

    return True


def collect_files(root: Path) -> list[Path]:
    """Walk the repo and collect files to process."""
    result = []
    for dirpath, dirnames, filenames in os.walk(root):
        p = Path(dirpath)
        # Prune skip dirs in-place so os.walk doesn't descend into them
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        if should_skip_path(p):
            dirnames.clear()
            continue
        for fname in filenames:
            fpath = p / fname
            if fpath.suffix in PROCESS_EXTENSIONS:
                if not should_skip_path(fpath):
                    result.append(fpath)
    return result


def main():
    parser = argparse.ArgumentParser(description="MissionControl entity rename sweep")
    parser.add_argument("--step", choices=["A1", "A2"], required=True)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--root", type=Path, default=REPO_ROOT)
    args = parser.parse_args()

    files = collect_files(args.root)
    changed = 0
    total = len(files)

    print(f"Step {args.step}: scanning {total} files under {args.root}")
    if args.dry_run:
        print("DRY RUN — no files will be modified\n")

    for fpath in sorted(files):
        if process_file(fpath, args.step, args.dry_run):
            changed += 1
            if not args.dry_run:
                print(f"  updated: {fpath.relative_to(args.root)}")

    print(f"\nDone. {changed}/{total} files {'would be ' if args.dry_run else ''}changed.")


if __name__ == "__main__":
    main()
