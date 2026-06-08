---
title: zRPC Plugin Reference
description: Configuration reference for the edgeplane-zrpc Zellij plugin — environment variables, pipe protocol, and permissions.
---

## Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `EDGEPLANE_ZRPC_PLUGIN_PATH` | Yes (to enable) | Absolute path to `edgeplane_zrpc.wasm`. If unset, `edgeplaned` uses the paste path — no behavior change. |
| `EDGEPLANE_ZRPC_SESSIONS` | Yes (to enable) | Comma-separated list of Zellij session names to watch. |

Both must be set for the plugin to activate.

## Permissions

The plugin requires these Zellij permissions, pre-seeded in `permissions.kdl` before the Zellij server starts:

| Permission | Why |
|------------|-----|
| `ReadApplicationState` | Read pane list and pane state |
| `ReadCliPipes` | Receive commands sent via `zellij pipe` |
| `WriteToStdin` | Inject text into pane PTY stdin |
| `ReadPaneContents` | Read scrollback buffer |
| `ChangeApplicationState` | Send focus events (optional) |

Find your Zellij cache directory:

```bash
zellij setup --check | grep 'CACHE DIR'
```

The `permissions.kdl` file format (raw absolute path as the node name — no `file:` prefix):

```kdl
"/absolute/path/to/edgeplane_zrpc.wasm" {
    ReadApplicationState
    ReadCliPipes
    WriteToStdin
    ReadPaneContents
}
```

## Pipe Protocol

The plugin communicates over the `edgeplane-zrpc` named Zellij pipe. All payloads are JSON.

### Commands (edgeplaned → plugin)

| `kind` | Fields | Description |
|--------|--------|-------------|
| `ping` | — | Health check |
| `inject` | `pane_id: number, text: string` | Inject text + newline into pane PTY stdin |
| `scrollback` | `pane_id: number, lines: number` | Request scrollback contents |
| `list_panes` | — | Request current pane manifest |

### Responses (plugin → edgeplaned)

| `kind` | Fields | Description |
|--------|--------|-------------|
| `pong` | — | Response to `ping` |
| `scrollback_response` | `pane_id: number, content: string` | Scrollback content |
| `pane_list` | `panes: [{id, title, is_focused}]` | Pane manifest |
| `pane_event` | `event: "exit"\|"focus_gained"\|"resize", pane_id: number` | Lifecycle event |

### Example: inject a prompt

```bash
# Sent by edgeplaned internally — shown here for debugging
zellij pipe --name edgeplane-zrpc -- '{"kind":"inject","pane_id":1,"text":"run tests"}'
```

## Build Details

The plugin is a Rust **bin crate** targeting `wasm32-wasip1`. This is required — a `cdylib` on `wasm32-wasip1` with Rust 1.82+ exports `_initialize` (WASI reactor) instead of `_start` (WASI command). Zellij 0.44.x requires `_start` and silently fails to instantiate reactor plugins; all subsequent `zellij pipe` calls hang indefinitely.

Build:

```bash
cargo build --release --target wasm32-wasip1 -p edgeplane-zrpc
```

Verify `_start` is exported:

```bash
wasm-tools print target/wasm32-wasip1/release/edgeplane_zrpc.wasm | grep '(export "_start"'
```

## Logs

Filter `edgeplaned` logs for zRPC activity:

```bash
journalctl --user -u edgeplaned -f | grep zrpc
```

Zellij log for plugin instantiation failures:

```bash
tail -f "$(zellij setup --check | grep 'LOG DIR' | awk '{print $3}')/zellij.log" | grep edgeplane
```

## What's Next

- [Guides: Zellij Integration](/guides/zellij-integration/) — end-to-end setup walkthrough
- [Architecture: Components](/architecture/components/) — where the plugin fits
