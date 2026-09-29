//! Which version of the Aspen protocol a deployment speaks, which capabilities beyond that
//! version's baseline it has, and what software it runs. `spec/federation.md` says how the
//! protocol evolves: within a version only by addition, with readers ignoring what they do not
//! know, and a feature added later named as a capability that others check before using it. A
//! capability a fork adds is named by a domain it controls, reversed (`org.example.feature`), so
//! no name of Aspen's own can ever collide with it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The protocol version this deployment speaks, the newest it knows.
pub const PROTOCOL_VERSION: u32 = 1;
/// The oldest protocol version this deployment still speaks: every version released within the
/// support window, thirty-six months.
pub const MINIMUM_PROTOCOL_VERSION: u32 = 1;
/// The capabilities this deployment has beyond the baseline of `PROTOCOL_VERSION`.
pub const CAPABILITIES: &[&str] = &[];

/// A deployment's protocol: the range of versions it speaks and its capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Protocol {
    /// The newest version it speaks.
    pub version: u32,
    /// The oldest version it still speaks.
    pub minimum: u32,
    /// What it can do beyond the baseline of its version: Aspen's capabilities by bare name, a
    /// fork's by a reversed domain of its own. Capabilities it does not know, a reader ignores.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl Protocol {
    /// This deployment's.
    pub fn ours() -> Self {
        Protocol {
            version: PROTOCOL_VERSION,
            minimum: MINIMUM_PROTOCOL_VERSION,
            capabilities: CAPABILITIES.iter().map(|c| (*c).to_string()).collect(),
        }
    }

    /// The version two deployments speak to each other in, the newest both know; `None` when
    /// their ranges do not meet.
    pub fn common_version(&self, other: &Protocol) -> Option<u32> {
        let version = self.version.min(other.version);
        (version >= self.minimum && version >= other.minimum).then_some(version)
    }
}

impl Default for Protocol {
    /// What a deployment that does not say speaks: the first version, with no capabilities.
    fn default() -> Self {
        Protocol {
            version: 1,
            minimum: 1,
            capabilities: Vec::new(),
        }
    }
}

/// The software a deployment runs, for people to read: Aspen's name and version, or a fork's.
/// Nothing decides anything by it; that is what `Protocol` is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Software {
    pub name: String,
    pub version: String,
}

impl Software {
    /// This deployment's.
    pub fn ours() -> Self {
        Software {
            name: "aspen".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

/// Every payload one deployment sends another, as `federation_schema.json` describes them: the
/// document each publishes, and the claims of the statements it signs (compact JWS, EdDSA,
/// each kind with its own `typ`). Never instantiated, as `EventStreamProtocol` is not.
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct FederationProtocol {
    /// Served at `https://{domain}/.well-known/aspen`.
    pub document: super::keys::DeploymentDocument,
    /// The claims of an `aspen-assertion+jwt`.
    pub assertion: super::abroad::Assertion,
    /// The claims of an `aspen-key-handover+jwt`.
    pub handover: super::keys::Handover,
    /// The claims of an `aspen-notice+jwt`, POSTed to `/api/v1/federation/notices` at the
    /// home of the user it is about.
    pub notice: super::notices::Notice,
    /// The claims of an `aspen-standing-request+jwt`, POSTed as `{"request": …}` to
    /// `/api/v1/federation/standing` at a home.
    pub standing_request: super::standing::StandingRequest,
    /// The claims of the `aspen-standing+jwt` a home answers with, as `{"standing": …}`.
    pub standing: super::standing::StandingAnswer,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol(minimum: u32, version: u32) -> Protocol {
        Protocol {
            version,
            minimum,
            capabilities: vec!["org.example.extra".into()],
        }
    }

    #[test]
    fn deployments_speak_the_newest_version_both_know() {
        assert_eq!(protocol(1, 3).common_version(&protocol(2, 5)), Some(3));
        assert_eq!(protocol(1, 1).common_version(&protocol(1, 1)), Some(1));
        assert_eq!(protocol(1, 2).common_version(&protocol(3, 4)), None);
    }

    #[test]
    fn a_protocol_from_a_newer_deployment_reads() {
        let read: Protocol = serde_json::from_str(
            r#"{"version": 4, "minimum": 2, "capabilities": ["dms.v9"], "unheardOf": true}"#,
        )
        .unwrap();
        assert_eq!(read.common_version(&Protocol::ours()), None);
        let bare: Protocol = serde_json::from_str(r#"{"version": 1, "minimum": 1}"#).unwrap();
        assert_eq!(bare, Protocol::default());
    }
}
