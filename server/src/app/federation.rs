//! Federation: this deployment among others. A deployment is known by its domain
//! ([`Domain`]) and proves what it says with an Ed25519 key pair, whose public half it
//! publishes at `https://{domain}/.well-known/aspen` ([`DeploymentDocument`]) with its policy.
//!
//! The policy is two gates each for users and for bots (deployment settings): who may
//! go from here to other deployments (emigration) and who may come here from them
//! (immigration). A gate is closed, open, or governed by a list of deployments that it allows or
//! blocks ([`FederationList`]); with `shared_list`, both directions read one list.
//!
//! Every other deployment this one knows is a row of `federated_deployment`, added by an
//! administrator (from the dashboard, under Manage federation, or the terminal) or recorded when
//! it is first contacted. Contacting a deployment ([`contact`]) reads its document and pins the
//! key found there the first time; a different key later is refused, and waits as the
//! deployment's offered key until an administrator accepts it ([`accept_key`]).

pub mod abroad;
pub mod contact;
pub mod directory;
pub mod domain;
pub mod fetch;
pub use aspen_federation_core::jws;
pub mod keys;
pub mod notices;
pub use aspen_federation_core::policy;
pub mod protocol;
pub mod received;
pub mod standing;

pub use contact::{ContactOutcome, contact, fetch_document, record_contact};
pub(crate) use directory::lists_of;
pub use directory::{
    FederatedDeployment, Listed, Origin, accept_key, add, get, list, list_all, remove, set_listed,
    set_note,
};
pub use domain::{Domain, own_domain};
pub use keys::{
    DeploymentDocument, Gates, Rotation, current_key, document, ensure_key, fingerprint, rotate_key,
};
pub use policy::{
    Direction, FederationList, FederationPolicy, Gate, MigrationRules, Subject, admits,
};

/// Where a deployment publishes its [`DeploymentDocument`].
pub const WELL_KNOWN_PATH: &str = "/.well-known/aspen";
/// The longest note an administrator may keep on a deployment, in characters.
pub const MAX_NOTE_CHARS: usize = 200;
