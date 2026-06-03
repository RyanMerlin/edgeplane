//! DB-free OpenAPI spec emitter.
//!
//! Prints the EdgePlane Tower OpenAPI JSON to stdout. This binary:
//! - does NOT construct AppState
//! - does NOT open a database connection
//! - does NOT read secrets or environment variables
//! - does NOT bind a network socket
//!
//! The spec is derived entirely from Rust types at compile time via utoipa's
//! proc macros. Run it as:
//!
//! ```sh
//! cargo run -p edgeplane-tower --bin gen-openapi > web2/openapi.json
//! ```

use edgeplane_tower::openapi::ApiDoc;
use utoipa::OpenApi;

fn main() {
    let json = ApiDoc::openapi()
        .to_pretty_json()
        .expect("OpenAPI serialization is infallible");
    print!("{json}");
}
