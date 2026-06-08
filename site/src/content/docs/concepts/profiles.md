---
title: Profiles
description: Personal operator profiles — what they carry, how they travel, and how to switch between them.
---

A profile is a portable bundle of configuration scoped to you as an operator. It travels with you, not with any particular machine. When you launch an agent on a new node, EdgePlane fetches your active profile from the server, writes it locally, and injects the right env and instruction files into the agent process.

## What a Profile Is

A profile bundles three things:

- **Env vars** — `EP_BASE_URL`, tool API keys, and any other key=value pairs the agent needs at startup
- **Instruction files** — `CLAUDE.md` for Claude Code, `AGENTS.md` for Codex and Gemini, and any other runtime-specific instruction files
- **Profile metadata** — name, description, version, and pin SHA for integrity checking

Profiles are stored server-side, scoped strictly to the owner's identity. One operator can have multiple profiles (coding, review, research). Only the active profile is injected at agent startup.

## How Profiles Travel

When `edgeplane run <runtime>` starts an agent:

1. The CLI fetches the active profile bundle from the server (`GET /api/profiles/{name}`)
2. The bundle is written to `~/.edgeplane/profiles/<name>/` via atomic file writes
3. An active-profile symlink is updated to point at the new version
4. Env vars from the profile's `env` file are injected into the agent subprocess
5. Instruction files are placed where the runtime expects them

If the server is unreachable, `edgeplane run` falls back to the last successfully pulled profile on disk. If no local copy exists, the launch fails with a clear error.

## Profile Commands

```bash
# List all your profiles
edgeplane profile list

# Show metadata for the active (or named) profile
edgeplane profile show --name <name>

# Set a profile as the default for new agent launches
edgeplane profile activate --name <name>

# Upload a local profile directory to the server
edgeplane profile publish --name <name>

# Download the latest profile bundle from the server to local cache
edgeplane profile pull --name <name>

# Create an empty profile shell on the server
edgeplane profile create --name <name>
```

`edgeplane profile use --name <name>` combines `activate` and `pull` in one step — sets the profile as default and pulls the bundle immediately.

## Profile Contexts

Separate profiles let you maintain different instruction files and env configurations for different types of work without switching machines or editing files mid-session.

Common patterns:

- A **coding** profile with tighter tool permissions and a CLAUDE.md focused on code quality
- A **review** profile with read-only tool permissions and instructions emphasizing caution
- A **research** profile with broader browsing permissions and a CLAUDE.md tuned for synthesis work

Each profile is independent on the server. Switching profiles is `edgeplane profile use --name <profile>` followed by restarting the agent.

## Profile Structure

```
~/.edgeplane/profiles/<name>/
├── env                    # key=value pairs injected at agent startup
├── CLAUDE.md              # Claude Code instruction file
├── AGENTS.md              # Codex/Gemini instruction file
└── config.json            # Profile metadata (name, description, version, pin sha256)
```

The `env` file uses the same format as a `.env` file — one `KEY=VALUE` per line, no quoting required. Lines starting with `#` are ignored.

The `config.json` file contains the profile's server-side metadata. Do not edit it by hand — use `edgeplane profile publish` to push changes and `edgeplane profile pull` to refresh the local copy.

## What's Next

- [Agent Setup](/guides/agent-setup/) — connecting a node and launching your first agent with a profile
- [Advanced: Personal Fleet Profiles](/advanced/fleet-profiles/) — managing multiple profiles across nodes and teams
