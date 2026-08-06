//! Writes the OpenAPI document to `backend/openapi.json`.
//!
//! Run from the workspace root with `cargo run --bin generate-openapi`, then
//! regenerate the typed client with `npm --prefix frontend run generate-api`.
//! This used to happen as a side effect of a unit test, which meant the file
//! silently drifted whenever the suite was not run from the crate directory.

use std::path::PathBuf;

use matrix_bot::api::openapi::ApiDoc;
use utoipa::OpenApi;

fn main() -> std::io::Result<()> {
    let json = ApiDoc::openapi()
        .to_pretty_json()
        .expect("OpenAPI document should serialize");

    // Resolve relative to this crate, so the output lands in the same place
    // whether invoked from the workspace root or from `backend/`.
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("openapi.json");
    std::fs::write(&out, json)?;

    println!("Wrote {}", out.display());
    Ok(())
}
