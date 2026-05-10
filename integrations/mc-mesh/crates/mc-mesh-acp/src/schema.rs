//! Types generated from the vendored ACP JSON schema.
//!
//! Source of truth: `schema/schema.json` in this crate. The `VERSION` file
//! alongside it records the `@zed-industries/agent-client-protocol` npm
//! package version we matched. To pull a newer schema, run `make sync-acp`
//! from the workspace root and review the resulting diff.

#![allow(clippy::all)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(non_camel_case_types)]
#![allow(rustdoc::all)]

include!(concat!(env!("OUT_DIR"), "/types.rs"));
