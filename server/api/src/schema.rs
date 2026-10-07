//! The schema files `--gen-openapi-schema` writes: `openapi.yaml`, `federation_schema.json`, and
//! `event_schema.json`.

use crate::{event_stream, openapi};
use aspen_app as app;
use schemars::schema_for;
use std::fs;

/// Writes every schema file to the working directory and exits the process.
pub fn write_schemas_and_exit() -> Result<(), app::Error> {
    fs::write("openapi.yaml", openapi().to_yaml()?)?;
    fs::write(
        "federation_schema.json",
        serde_json::to_string_pretty(&schema_for!(app::federation::protocol::FederationProtocol))?,
    )?;
    let event_schema = schema_for!(event_stream::EventStreamProtocol);
    fs::write(
        "event_schema.json",
        serde_json::to_string_pretty(&event_schema)?,
    )?;
    std::process::exit(0);
}
