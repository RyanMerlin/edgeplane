---
title: Zellij Integration
description: How to set up the edgeplane-zrpc Zellij plugin for focus-free agent control.
---

## What the Zellij Plugin Adds

By default, EdgePlane communicates with `zellij_hosted` agents by paste-and-send — focus the pane, inject keystrokes, wait. This works, but has a race: if the agent is actively typing, the inject can land mid-output.

The `edgeplane-zrpc` Zellij plugin replaces this with a named-pipe control path: `edgeplaned` sends commands to the plugin over a Zellij pipe, and the plugin acts without the pane needing focus. This unlocks:

- **Focus-free injection** — send prompts to any pane without a focus race
- **Scrollback reads** — `edgeplaned` reads recent pane output programmatically
- **Pane lifecycle events** — detect when a pane exits, splits, or resizes
- **Cancel** — interrupt a running agent without killing the pane

## Feature Flag

The plugin is **dormant by default**. If `EDGEPLANE_ZRPC_PLUGIN_PATH` is unset, `edgeplaned` falls back to the paste path exactly as before — no behavior change.

## Prerequisites

- Zellij 0.44.x
- Rust toolchain (to build the plugin, or use the prebuilt binary from the GitHub releases page)
- `wasm32-wasip1` target: `rustup target add wasm32-wasip1`

## Building the Plugin

```bash
git clone https://github.com/RyanMerlin/edgeplane.git
cd edgeplane/crates/edgeplane-zrpc
cargo build --release --target wasm32-wasip1
# Output: target/wasm32-wasip1/release/edgeplane_zrpc.wasm
```

Copy the `.wasm` file to a stable path, for example `~/.local/lib/edgeplane/edgeplane_zrpc.wasm`.

## Pre-seeding Permissions

Zellij requires plugin permissions to be granted before the server starts. Find your Zellij cache directory:

```bash
zellij setup --check | grep 'CACHE DIR'
```

Write a `permissions.kdl` file in that cache directory **before starting Zellij**. The key is the raw absolute path to the `.wasm` file (no `file:` prefix):

```kdl
# <ZELLIJ_CACHE_DIR>/permissions.kdl
"/absolute/path/to/edgeplane_zrpc.wasm" {
    ReadApplicationState
    ReadCliPipes
    WriteToStdin
    ReadPaneContents
}
```

## Loading the Plugin in a Session

Add the plugin to your Zellij config or session layout. Note that the config block uses a `file:` URI prefix — this is different from `permissions.kdl`, which uses the raw absolute path without any prefix. Both forms are intentional and required.

```kdl
// ~/.config/zellij/config.kdl
plugins {
    edgeplane-zrpc location="file:/absolute/path/to/edgeplane_zrpc.wasm"
}
load_plugins {
    edgeplane-zrpc
}
```

## Enabling on edgeplaned

Set two environment variables before starting `edgeplaned`:

```bash
export EDGEPLANE_ZRPC_PLUGIN_PATH=/absolute/path/to/edgeplane_zrpc.wasm
export EDGEPLANE_ZRPC_SESSIONS="my-session"   # comma-separated Zellij session names to watch
edgeplane daemon up
```

Or add to `~/.edgeplane/edgeplaned/env`:
```
EDGEPLANE_ZRPC_PLUGIN_PATH=/absolute/path/to/edgeplane_zrpc.wasm
EDGEPLANE_ZRPC_SESSIONS=my-session
```

## Verifying

Start a Zellij session and check the plugin responds:

```bash
zellij attach my-session
# In any pane, send a health check:
zellij pipe --name edgeplane-zrpc -- '{"kind":"ping"}'
# Expected response within 2 seconds: {"kind":"pong"}
```

If `zellij pipe` hangs, the plugin is not instantiated. Check the Zellij log:

```bash
tail -f "$(zellij setup --check | grep 'LOG DIR' | awk '{print $3}')/zellij.log" | grep edgeplane
```

Common failure: permissions not pre-seeded. Verify `permissions.kdl` exists in the correct cache directory and the path matches the `.wasm` exactly.

## Pipe Protocol

The plugin communicates over the `edgeplane-zrpc` named Zellij pipe. All messages are JSON:

| Direction | `kind` | Key Fields | Description |
|-----------|--------|------------|-------------|
| → plugin | `inject` | `pane_id`, `text` | Inject text + newline into pane PTY stdin |
| → plugin | `scrollback` | `pane_id`, `lines` | Request scrollback contents |
| → plugin | `list_panes` | — | Request current pane manifest |
| plugin → | `pong` | — | Response to `ping` |
| plugin → | `scrollback_response` | `pane_id`, `content` | Scrollback content |
| plugin → | `pane_list` | `panes` | Current pane manifest |
| plugin → | `pane_event` | `event`, `pane_id` | Lifecycle event (exit, focus_gained, resize) |

See [zRPC Plugin Reference](/reference/zrpc-plugin/) for the full protocol and all message types.

## What's Next
- [zRPC Plugin Reference](/reference/zrpc-plugin/) — full env var and protocol reference
- [Architecture: Components](/architecture/components/) — where the plugin fits in the stack
