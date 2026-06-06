# edgeplane-zrpc

A hidden Zellij control plugin for edgeplaned: focus-free inject/cancel,
scrollback reads, pane-state classification, and pane lifecycle events — all
over Zellij named pipes.

## Purpose

The legacy control path for ZellijHosted agents (`paste + 300 ms sleep + Enter`
via `zellij action write-chars`) has a fundamental race: it grabs pane focus,
which breaks when two agents share a session tree. `edgeplane-zrpc` removes
that race entirely by using `zellij pipe` (focus-free by design) and the
`write_chars_to_pane_id` / `send_sigint_to_pane_id` API exposed by
`zellij-tile`.

The plugin renders nothing and uses no pane. It is loaded as a background
service via `load_plugins {}` in the session's Zellij config.

## Bin-crate requirement

Zellij plugins **must** be built as a `[[bin]]` crate on the
`wasm32-wasip1` target. `register_plugin!` (from `zellij-tile`) generates
the `fn main()` entry point, which becomes the `_start` export Zellij
requires.

A `[lib] crate-type = ["cdylib"]` compiles to a WASI *reactor* with no
`_start` export. Zellij 0.44.3 rejects reactor modules at instantiation
("could not find exported function"). This is why the `Cargo.toml` uses
`[[bin]]`, not `[lib]`.

## Build

```bash
# from crates/edgeplane-zrpc/
cargo build --release --target wasm32-wasip1
```

The artifact is at:

```
target/wasm32-wasip1/release/edgeplane_zrpc.wasm
```

The crate has its own `Cargo.lock` and `[workspace]` and is **excluded from
the host workspace** (`crates/edgeplane-zrpc` is in the root `exclude` list).
Build it separately from inside its own directory.

## Two requirements for headless use

1. **Bin crate** — the `[[bin]]` shape described above. A `cdylib` will not
   load.

2. **Pre-seeded `permissions.kdl`** — Zellij requires explicit user consent for
   plugin permissions. In a headless (non-interactive) fleet context there is no
   prompt to answer, so the grant must be written to
   `<ZELLIJ_CACHE_DIR>/permissions.kdl` before the session starts. The key is
   the **raw absolute wasm path** (no `file:` prefix), matching
   `RunPluginLocation::to_string()` in zellij-tile. Required permissions:
   `ReadApplicationState`, `ChangeApplicationState`, `WriteToStdin`,
   `ReadPaneContents`, `ReadCliPipes`.

## Transport

### Control pipe (request/response)

edgeplaned sends NDJSON `Request` messages via:

```
zellij --session <session> pipe --name zrpc -- <json-request-line>
```

The plugin writes one `Response` NDJSON line to the pipe's stdout, correlated
by the request `id`. edgeplaned reads stdout line-by-line until it finds the
matching response, then kills the child. The on-demand `--plugin file:<wasm>`
form is deliberately **not** used — it fails to instantiate on 0.44.3
("could not find exported function"). The plugin must be pre-loaded via the
session's Zellij config (`plugins {}` + `load_plugins {}`).

### Event pipe (long-lived)

edgeplaned holds open a second pipe for unsolicited lifecycle events:

```
zellij --session <session> pipe --name zrpc-events
```

The plugin pushes `PluginEvent` NDJSON lines (pane exited, pane closed, pane
update) to this pipe as they fire. edgeplaned reads from the pipe in a
background task.

## Install step

`edgeplaned-runtimes::zellij_install::install_zrpc_plugin(config_path, cache_dir, wasm_path)`
writes both the Zellij config entries and the `permissions.kdl` grant
idempotently. It must be called before the Zellij session starts so that Zellij
reads the updated config at session startup.

To find the cache dir:

```bash
zellij setup --check | grep '\[CACHE DIR\]'
# e.g.: [CACHE DIR]:   /workspace/cache/zellij
```

Or use `edgeplaned_runtimes::zellij_install::resolve_zellij_cache_dir()` from
Rust.

## Wire protocol

The `edgeplane-zrpc-proto` crate (target-independent, unit-tested on the host)
defines all types: `Request`, `Response`, `PluginEvent`, `Call`, `PaneOps`.
This crate is the thin `zellij-tile` glue layer — all business logic lives in
the proto crate.
