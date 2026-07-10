---
title: Installation
description: Install the EdgePlane CLI, daemon, and control plane.
---

EdgePlane has three components:

| Component | Purpose |
|-----------|---------|
| `edgeplane` | Operator CLI and agent launcher — your primary interface |
| `edgeplaned` | Headless executor daemon — manages agent subprocesses and secrets brokering |
| `edgeplane-tower` | HTTP server backing the REST/SSE API — domains, missions, tasks, artifacts, Git publication |

## Install Script (recommended)

The install script downloads a prebuilt binary and falls back to a source build if no binary is available for your platform.

**Linux / macOS (quickest path):**

```bash
curl -fsSL https://edgeplane.ai/install.sh | bash
```

Or directly from the repo:

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/install-edgeplane.sh)
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/bootstrap-edgeplane.ps1 | iex
```

By default installs to `~/.local/bin/edgeplane`. Ensure `~/.local/bin` is on `PATH`.

## Build from Source

Requires a working [Rust toolchain](https://rustup.rs) (stable).

```bash
git clone https://github.com/RyanMerlin/edgeplane.git
cd edgeplane

# Build all three binaries from the workspace root
cargo build --release -p edgeplane -p edgeplaned -p edgeplane-tower

# Install
cp target/release/edgeplane ~/.local/bin/edgeplane
cp target/release/edgeplaned ~/.local/bin/edgeplaned
cp target/release/edgeplane-tower ~/.local/bin/edgeplane-tower
```

## Running the Control Plane

`edgeplane-tower` is the API server all agents and operators talk to. Migrations run automatically on startup.

```bash
edgeplane-tower --bind 0.0.0.0:8008
```

Verify it's up:

```bash
curl http://localhost:8008/api/health
```

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `EP_BASE_URL` | Control plane HTTP base URL | `http://localhost:8008` |

You can set this in your shell profile or pass it inline:

```bash
export EP_BASE_URL="https://edgeplane.example.com"
```

## Authentication

EdgePlane uses three auth mechanisms — no static `EP_TOKEN` is required:

| Auth mode | When to use | How |
|-----------|-------------|-----|
| OIDC (interactive) | Operators, interactive use | `edgeplane auth login` — browser flow, issues a session token |
| Node JWT (machine) | Daemons, `edgeplaned` | `edgeplane agent node register --hostname <name>` — JWT stored at `~/.edgeplane/config/node.json` (`$EP_HOME/config/node.json`) |
| Service account | CI, programmatic | `ep_sa_*` tokens — created via API, passed as `Bearer` |

:::note[v0.13.0 breaking change]
`edgeplane launch` was removed in v0.13.0. Use `edgeplane run <runtime>` as the single agent launcher for all runtimes (`claude`, `codex`, `gemini`, `openclaw`, `custom`).
:::

## Verify

```bash
edgeplane --version
edgeplane health --json
```

## Next Steps

- [Quick Start](/getting-started/quick-start/) — run your first agent in 5 minutes
- [Agent Setup](/getting-started/agent-setup/) — connect Claude Code, Codex, or Gemini CLI
