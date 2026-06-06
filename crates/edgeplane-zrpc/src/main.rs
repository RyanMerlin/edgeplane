//! `edgeplane-zrpc` — a hidden Zellij control plugin for edgeplaned.
//!
//! This is the thin `zellij-tile` glue layer over the host-tested protocol
//! core in `edgeplane-zrpc-proto`. It does no business logic of its own:
//!
//! * **Control pipe** (`zrpc`): edgeplaned sends NDJSON [`Request`]s via
//!   `zellij pipe --name zrpc`; we parse, [`handle`] each against the live
//!   panes, and write each [`Response`] back on the same pipe.
//! * **Event pipe** (`zrpc-events`): a long-lived pipe edgeplaned holds open;
//!   we push [`PluginEvent`]s (pane exited / closed) as they happen — the
//!   focus-race-free replacement for edgeplaned's dump-screen scrape loop.
//!
//! The plugin renders nothing (loaded as a background service via
//! `load_plugins {}`). Pinned to `zellij-tile` 0.44.3 to match the running
//! fleet server ABI exactly.

use std::collections::BTreeMap;

use edgeplane_zrpc_proto::{handle, parse_pane_ref, parse_requests, PaneKind, PaneOps, PluginEvent, Response};
use zellij_tile::prelude::*;

/// Pipe name for synchronous request/response control traffic.
const CONTROL_PIPE: &str = "zrpc";
/// Pipe name for unsolicited lifecycle events pushed to edgeplaned.
const EVENT_PIPE: &str = "zrpc-events";

#[derive(Default)]
struct ZrpcPlugin {
    /// Latest pane manifest, cached from `PaneUpdate` events; the source for
    /// `list_agent_panes`.
    manifest: Option<PaneManifest>,
    /// CLI pipe id of edgeplaned's long-lived event subscription (the
    /// `zellij pipe --name zrpc-events` it holds open). Cached when that pipe
    /// opens so `update()` can push events back to that specific caller's
    /// stdout. `None` until edgeplaned subscribes.
    event_pipe_id: Option<String>,
}

register_plugin!(ZrpcPlugin);

impl ZellijPlugin for ZrpcPlugin {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState, // pane manifest
            PermissionType::ChangeApplicationState,
            PermissionType::WriteToStdin,     // write_chars_to_pane_id / sigint
            PermissionType::ReadPaneContents, // get_pane_scrollback
            PermissionType::ReadCliPipes,     // control pipe i/o
        ]);
        subscribe(&[
            EventType::PaneUpdate,
            EventType::CommandPaneExited,
            EventType::PaneClosed,
            EventType::PermissionRequestResult,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PaneUpdate(manifest) => self.manifest = Some(manifest),
            Event::CommandPaneExited(pane_id, exit_code, _ctx) => {
                self.emit_event(&PluginEvent::CommandPaneExited { pane_id, exit_code });
            }
            Event::PaneClosed(pane_id) => {
                self.emit_event(&PluginEvent::PaneClosed {
                    pane_id: pane_id_num(pane_id),
                });
            }
            _ => {}
        }
        // Hidden plugin: never requests a render.
        false
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        // We only serve CLI-originated pipes; the `pipe_id` is what
        // `cli_pipe_output`/`block`/`unblock` correlate on (NOT the name), so
        // the reply routes back to *this* `zellij pipe` invocation's stdout.
        let PipeSource::Cli(pipe_id) = pipe_message.source else {
            return false;
        };
        match pipe_message.name.as_str() {
            CONTROL_PIPE => {
                if let Some(payload) = pipe_message.payload.as_deref() {
                    // Block the caller until every response line is written, so
                    // the synchronous `zellij pipe` call returns with our output.
                    block_cli_pipe_input(&pipe_id);
                    for parsed in parse_requests(payload) {
                        let line = match parsed {
                            Ok(req) => handle(&req, self).to_ndjson_line(),
                            // Unparseable line: no id to correlate; best-effort error.
                            Err(e) => Response::error("", e.to_string()).to_ndjson_line(),
                        };
                        cli_pipe_output(&pipe_id, &format!("{line}\n"));
                    }
                    unblock_cli_pipe_input(&pipe_id);
                }
                false
            }
            EVENT_PIPE => {
                // edgeplaned's long-lived event subscription. Keep it open and
                // remember its id so `update()` can push events to it.
                block_cli_pipe_input(&pipe_id);
                self.event_pipe_id = Some(pipe_id);
                false
            }
            _ => false,
        }
    }

    fn render(&mut self, _rows: usize, _cols: usize) {
        // Hidden background service — no UI.
    }
}

impl ZrpcPlugin {
    /// Push a lifecycle event to edgeplaned over its cached event-pipe id.
    /// No-op until edgeplaned has subscribed (opened the `zrpc-events` pipe).
    fn emit_event(&self, event: &PluginEvent) {
        if let Some(pipe_id) = &self.event_pipe_id {
            cli_pipe_output(pipe_id, &format!("{}\n", event.to_ndjson_line()));
        }
    }
}

impl PaneOps for ZrpcPlugin {
    fn inject(&mut self, pane_id: &str, text: &str) -> Result<(), String> {
        let id = to_zellij_pane_id(pane_id)?;
        // Focus-free write — no paste/sleep/send-keys, no focus race. The
        // caller includes any trailing newline needed to submit; bracketed-paste
        // semantics for multi-line prompts are tuned in the live-integration phase.
        write_chars_to_pane_id(text, id);
        Ok(())
    }

    fn cancel(&mut self, pane_id: &str) -> Result<(), String> {
        let id = to_zellij_pane_id(pane_id)?;
        send_sigint_to_pane_id(id);
        Ok(())
    }

    fn read_scrollback(&mut self, pane_id: &str, lines: Option<usize>) -> Result<Vec<String>, String> {
        let id = to_zellij_pane_id(pane_id)?;
        let contents = get_pane_scrollback(id, false)?;
        let mut viewport = contents.viewport;
        if let Some(n) = lines {
            if viewport.len() > n {
                viewport = viewport.split_off(viewport.len() - n);
            }
        }
        Ok(viewport)
    }

    fn list_agent_panes(&mut self) -> Result<Vec<String>, String> {
        let manifest = self
            .manifest
            .as_ref()
            .ok_or_else(|| "pane manifest not yet received".to_string())?;
        let mut out = Vec::new();
        for panes in manifest.panes.values() {
            for pane in panes {
                if !pane.is_plugin && !pane.exited {
                    out.push(format!("terminal_{}", pane.id));
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

/// Numeric id of a pane regardless of kind.
fn pane_id_num(pane_id: PaneId) -> u32 {
    match pane_id {
        PaneId::Terminal(n) | PaneId::Plugin(n) => n,
    }
}

/// Map a wire pane id (`"terminal_3"`) onto `zellij_tile::PaneId`. Parsing
/// (and its edge cases) lives in the host-tested proto crate.
fn to_zellij_pane_id(s: &str) -> Result<PaneId, String> {
    let (kind, n) = parse_pane_ref(s)?;
    Ok(match kind {
        PaneKind::Terminal => PaneId::Terminal(n),
        PaneKind::Plugin => PaneId::Plugin(n),
    })
}
