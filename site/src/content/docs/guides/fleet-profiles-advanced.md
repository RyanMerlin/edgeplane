---
title: Personal Fleet Profiles (Advanced)
description: Managing operator profiles across multiple machines — publish, pull, and switch profiles in a self-hosted EdgePlane setup.
---

This guide covers advanced profile workflows for operators running a self-hosted EdgePlane instance. If you just need to use profiles day-to-day, start with [Concepts: Profiles](/concepts/profiles/).

## Profile Storage Architecture

Profiles are stored server-side in `edgeplane-tower`'s database, scoped to the owning operator. The local machine caches a copy at `~/.edgeplane/profiles/<name>/`.

```
Server (edgeplane-tower)             Local cache
┌──────────────────────────┐         ┌─────────────────────────────────┐
│  profiles table          │◀───────▶│  ~/.edgeplane/profiles/         │
│    owner: operator-id    │ publish/│    my-profile/                  │
│    name: my-profile      │  pull   │      (bundle contents)          │
│    bundle: {...}         │         └─────────────────────────────────┘
└──────────────────────────┘
```

`edgeplane run` always pulls the latest profile from the server before launching — the local cache is just a write-through layer.

## Profile Commands Quick Reference

```bash
edgeplane profile list                          # list your profiles
edgeplane profile show --name <name>            # show profile metadata
edgeplane profile create --name <name>          # create an empty profile
edgeplane profile activate --name <name>        # set as the default profile
edgeplane profile use --name <name>             # activate + pull in one step
edgeplane profile publish --name <name>         # upload local bundle → server
edgeplane profile pull --name <name>            # download server → local
edgeplane profile pin --name <name> --sha256 <hash>  # pin to a specific version
edgeplane profile delete --name <name> --confirm-delete
```

## Creating and Publishing a Profile

```bash
# Create an empty profile
edgeplane profile create --name coding

# Build the bundle locally
mkdir -p ~/.edgeplane/profiles/coding
cat > ~/.edgeplane/profiles/coding/env <<EOF
EP_BASE_URL=https://your-tower-host
EOF
cat > ~/.edgeplane/profiles/coding/CLAUDE.md <<EOF
You are in coding mode. Focus on implementation and correctness.
EOF

# Upload to server
edgeplane profile publish --name coding
```

## Syncing Across Machines

On a second machine:

```bash
edgeplane auth login                         # authenticate to the same tower instance
edgeplane profile pull --name coding         # download profile from server
edgeplane profile activate --name coding     # set as default
edgeplane run claude                         # launches with the coding profile
```

Or in one step:

```bash
edgeplane profile use --name coding          # activate + pull together
edgeplane run claude
```

The profile travels via the server — no direct machine-to-machine transfer needed.

## Pinning a Profile Version

Each publish creates a new version server-side with a SHA-256 hash. To pin to a specific version:

```bash
# Check current status and pin info
edgeplane profile status --name coding

# Pin to a specific SHA-256
edgeplane profile pin --name coding --sha256 <sha256-from-status>
```

A pinned profile will not advance past the pinned SHA-256 on `pull`, protecting against unintended updates.

## What's Next
- [Concepts: Profiles](/concepts/profiles/) — the profile model explained
- [Reference: CLI](/reference/cli/) — full `edgeplane profile` command reference
