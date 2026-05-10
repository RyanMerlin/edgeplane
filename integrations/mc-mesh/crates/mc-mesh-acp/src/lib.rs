//! Pure-Rust client for the [Agent Client Protocol] (ACP).
//!
//! ACP is a JSON-RPC 2.0 protocol over a bidirectional bytestream (typically
//! a child process's stdio). This crate implements the **client** side —
//! i.e. it spawns and drives an ACP-speaking agent (today: the Node-based
//! `@zed-industries/claude-code-acp`).
//!
//! The crate is intentionally MissionControl-agnostic. Higher layers
//! (`mc-mesh-runtimes`, `session_supervisor`) wrap it to plug ACP agents
//! into mc-mesh's runtime model.
//!
//! # Layout
//!
//! - [`schema`] — types generated from the vendored ACP JSON schema. See
//!   `schema/VERSION` for which upstream version we matched. Bump with
//!   `make sync-acp`.
//! - [`consts`] — JSON-RPC method names + protocol version constant.
//! - [`jsonrpc`] — minimal JSON-RPC 2.0 framing over async streams.
//! - [`agent`] — high-level [`Agent`] handle: spawn, initialize, prompt,
//!   shutdown.
//! - [`error`] — crate error type.
//!
//! [Agent Client Protocol]: https://agentclientprotocol.com

pub mod agent;
pub mod consts;
pub mod error;
pub mod jsonrpc;
pub mod schema;
pub mod wire;

pub use agent::{Agent, SpawnOpts};
pub use error::{AcpError, Result};
pub use wire::{ContentBlock, SessionNotification, SessionUpdate};
