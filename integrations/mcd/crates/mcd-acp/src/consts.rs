//! Method name constants and protocol version.
//!
//! Mirrors `schema.ts` from `@zed-industries/agent-client-protocol`. Kept
//! hand-vendored (rather than codegen'd from the JSON Schema, which only
//! describes types) so the strings stay in lockstep with the spec.
//!
//! When updating: cross-check against the `AGENT_METHODS` and `CLIENT_METHODS`
//! tables in `schema.ts` of the matched npm version (see `schema/VERSION`).

/// ACP wire protocol version. Sent by the client in `initialize`.
///
/// Generated reference: `@zed-industries/agent-client-protocol@0.4.5`,
/// `schema.ts: PROTOCOL_VERSION`.
pub const PROTOCOL_VERSION: u32 = 1;

/// JSON-RPC methods the **agent** handles (we send these as the client).
pub mod agent_methods {
    pub const AUTHENTICATE: &str = "authenticate";
    pub const INITIALIZE: &str = "initialize";
    pub const SESSION_CANCEL: &str = "session/cancel";
    pub const SESSION_LOAD: &str = "session/load";
    pub const SESSION_NEW: &str = "session/new";
    pub const SESSION_PROMPT: &str = "session/prompt";
    pub const SESSION_SET_MODE: &str = "session/set_mode";
    pub const SESSION_SET_MODEL: &str = "session/set_model";
}

/// JSON-RPC methods the **client** handles (the agent sends these to us).
pub mod client_methods {
    pub const FS_READ_TEXT_FILE: &str = "fs/read_text_file";
    pub const FS_WRITE_TEXT_FILE: &str = "fs/write_text_file";
    pub const SESSION_REQUEST_PERMISSION: &str = "session/request_permission";
    pub const SESSION_UPDATE: &str = "session/update";
    pub const TERMINAL_CREATE: &str = "terminal/create";
    pub const TERMINAL_KILL: &str = "terminal/kill";
    pub const TERMINAL_OUTPUT: &str = "terminal/output";
    pub const TERMINAL_RELEASE: &str = "terminal/release";
    pub const TERMINAL_WAIT_FOR_EXIT: &str = "terminal/wait_for_exit";
}
