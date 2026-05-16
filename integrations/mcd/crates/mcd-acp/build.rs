//! Generates Rust types from the vendored ACP schema.
//!
//! The schema lives at `schema/schema.json`, copied verbatim from the
//! `@zed-industries/agent-client-protocol` npm package. The version we
//! matched is recorded in `schema/VERSION`. To pull a newer schema, run
//! `make sync-acp` from the workspace root.
//!
//! Generated types land in `$OUT_DIR/types.rs` and are included from
//! `src/types.rs`.

use std::env;
use std::fs;
use std::path::PathBuf;

use typify::{TypeSpace, TypeSpaceSettings};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let schema_path = manifest_dir.join("schema/schema.json");
    let version_path = manifest_dir.join("schema/VERSION");

    println!("cargo:rerun-if-changed={}", schema_path.display());
    println!("cargo:rerun-if-changed={}", version_path.display());

    let schema_src = fs::read_to_string(&schema_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", schema_path.display()));
    let schema: schemars::schema::RootSchema = serde_json::from_str(&schema_src)
        .unwrap_or_else(|e| panic!("parse {}: {e}", schema_path.display()));

    let mut settings = TypeSpaceSettings::default();
    settings.with_struct_builder(false);

    // The vendored schema uses OpenAPI-style `discriminator` keywords on
    // `ContentBlock` and `SessionUpdate` (and the `SessionNotification`
    // envelope). typify 0.6 lowers these into anonymous `Variant0/Variant1/...`
    // enums whose names are not usable. We replace those types with
    // hand-rolled equivalents in `crate::wire` that serialize the same wire
    // format with named variants. typify then references our types wherever
    // the schema $refs them.
    settings.with_replacement(
        "ContentBlock",
        "crate::wire::ContentBlock",
        std::iter::empty(),
    );
    settings.with_replacement(
        "SessionUpdate",
        "crate::wire::SessionUpdate",
        std::iter::empty(),
    );
    settings.with_replacement(
        "SessionNotification",
        "crate::wire::SessionNotification",
        std::iter::empty(),
    );

    // Several types in the schema are flagged "**UNSTABLE**" and use
    // discriminator/oneOf patterns that typify 0.6 lowers as broken
    // `Variant0/1/...` enums. We don't consume them yet — accept them as
    // opaque JSON. When a caller actually needs any of these, promote
    // to a typed shape in `wire.rs`.
    for ty in [
        "SessionConfigOption",
        "SessionConfigOptionCategory",
        "SessionModelState",
        "SessionModeState",
        "SessionConfigSelectOptions",
        "SessionConfigValueId",
    ] {
        settings.with_replacement(ty, "::serde_json::Value", std::iter::empty());
    }

    let mut type_space = TypeSpace::new(&settings);
    type_space
        .add_root_schema(schema)
        .expect("typify add_root_schema");

    let contents = prettyplease::unparse(
        &syn::parse2::<syn::File>(type_space.to_stream()).expect("parse generated tokens"),
    );

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out_path = out_dir.join("types.rs");
    fs::write(&out_path, contents).expect("write generated types");
}
