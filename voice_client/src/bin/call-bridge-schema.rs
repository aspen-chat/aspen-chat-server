//! Writes `call_bridge_schema.json` to the working directory: the JSON Schema of the snapshot the
//! mobile apps' web views read of the call engine (`voice_client::state::CallState`, as
//! `Engine::state_json` writes it), which the client's code generator (`pnpm codegen` in
//! `client/`) reads from the repository root, as it reads `voice_signal_schema.json`.

use schemars::schema_for;
use voice_client::state::CallState;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = schema_for!(CallState);
    std::fs::write(
        "call_bridge_schema.json",
        serde_json::to_string_pretty(&schema)?,
    )?;
    Ok(())
}
