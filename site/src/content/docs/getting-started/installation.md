---
title: Installation
description: Install the MissionControl CLI, daemon, and control plane.
---

MissionControl has three components:

| Component | Purpose |
|-----------|---------|
| `mc` | Operator CLI and agent launcher — your primary interface |
| `mcd` | Headless executor daemon — manages agent subprocesses and secrets brokering |
| `mc-controlplane` | HTTP server backing the REST/SSE API — missions, tasks, approvals |

## Install Script (recommended)

The install script downloads a prebuilt binary and falls back to a source build if no binary is available for your platform.

**Linux / macOS:**

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/install-mc.sh)
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/RyanMerlin/missioncontrol/main/scripts/bootstrap-mc.ps1 | iex
```

By default installs to `~/.local/bin/mc`. Ensure `~/.local/bin` is on `PATH`.

## Build from Source

Requires a working [Rust toolchain](https://rustup.rs) (stable).

```bash
git clone https://github.com/RyanMerlin/missioncontrol.git
cd missioncontrol

# Build and install mc CLI
cd crates/mc && cargo build --release
cp target/release/mc ~/.local/bin/mc

# Build and install mcd daemon
cd ../mcd && cargo build --release
cp target/release/mcd ~/.local/bin/mcd

# Build and install mc-controlplane server
cd ../mc-controlplane && cargo build --release
cp target/release/mc-controlplane ~/.local/bin/mc-controlplane
```

## Running the Control Plane

`mc-controlplane` is the API server all agents and operators talk to. Migrations run automatically on startup.

```bash
mc-controlplane --serve --bind 0.0.0.0:8008
```

Verify it's up:

```bash
curl http://localhost:8008/health
```

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `MC_BASE_URL` | Control plane HTTP base URL | `http://localhost:8008` |
| `MC_TOKEN` | Bearer token for API auth | unset |

You can set these in your shell profile or pass them inline:

```bash
export MC_BASE_URL="https://mc.example.com"
export MC_TOKEN="your-token"
```

## Verify

```bash
mc --version
mc health --json
```

## Next Steps

- [Quick Start](/missioncontrol/getting-started/quick-start/) — run your first agent in 5 minutes
- [Agent Setup](/missioncontrol/getting-started/agent-setup/) — connect Claude Code, Codex, or Gemini CLI
