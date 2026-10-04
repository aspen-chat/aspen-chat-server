//! A plugin's manifest: what it is, what it asks for, and what it says to people, as the
//! operator reads it before installing. `spec/plugin_manifest.schema.json` is written from
//! these types (`ASPEN_WRITE_SPEC=1 cargo test` writes it afresh), and `check` holds a manifest
//! to everything the schema cannot say.

use super::settings::{self, SettingField};
use super::{API_VERSION, Messages, PluginPermission};
use crate::app::bot_command::Command;
use crate::app::permissions::Permission;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// The longest a plugin's id may be.
pub const MAX_ID_LENGTH: usize = 128;
/// The longest any one of a plugin's messages may be, in characters.
pub const MAX_MESSAGE_CHARS: usize = 2000;
/// The most storage a plugin may ask for, in bytes.
pub const MAX_STORAGE_QUOTA: u64 = 1 << 30;
/// The largest attachment a plugin may ask to read, in bytes.
pub const MAX_ATTACHMENT_LIMIT: u64 = 100 << 20;

/// Everything a plugin declares.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// A domain its author controls, reversed (`org.example.nocursing`).
    pub id: String,
    /// Its version, in semver.
    pub version: String,
    /// The version of the plugin interface it was built against.
    pub api: u32,
    /// The component's file, relative to the manifest.
    pub component: String,
    /// A key of `messages` naming it.
    pub name: String,
    /// A key of `messages` saying what it does.
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// The language every key is given in, a BCP 47 tag.
    pub default_language: String,
    /// Its text in each language it speaks: language tag, then key, then text, with `%{name}`
    /// for what is filled in.
    pub messages: Messages,
    /// What it may do.
    #[serde(default)]
    pub permissions: BTreeSet<PluginPermission>,
    /// Which hooks it answers.
    #[serde(default)]
    pub hooks: Hooks,
    /// What the operator configures.
    #[serde(default)]
    pub settings: Vec<SettingField>,
    /// What each community that turns it on configures.
    #[serde(default)]
    pub community_settings: Vec<SettingField>,
    /// The hosts it may call over HTTPS, with `network`.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// The bytes of storage it may keep, with `storage`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_quota: Option<u64>,
    /// The largest attachment it may read, in bytes, with `attachments.read`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_limit: Option<u64>,
    /// Its own account, with `act`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<Principal>,
    /// A key of `messages` saying what it keeps of what it sees, and for how long.
    pub retention: String,
}

/// The hooks a plugin answers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hooks {
    /// The records it decides before they are saved, and what happens when it fails to.
    #[serde(default)]
    pub intercept: BTreeMap<InterceptHook, InterceptPolicy>,
    /// What it is told of after it happens.
    #[serde(default)]
    pub observe: BTreeSet<ObserveHook>,
}

/// A record decided before it is saved.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum InterceptHook {
    #[serde(rename = "message.create")]
    MessageCreate,
    #[serde(rename = "message.edit")]
    MessageEdit,
}

/// What a plugin is told of.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ObserveHook {
    #[serde(rename = "message.create")]
    MessageCreate,
    #[serde(rename = "message.edit")]
    MessageEdit,
    #[serde(rename = "message.delete")]
    MessageDelete,
    /// A command sent to its principal.
    #[serde(rename = "command.invoke")]
    CommandInvoke,
    /// A community turned it on.
    #[serde(rename = "plugin.enable")]
    PluginEnable,
    /// A community turned it off.
    #[serde(rename = "plugin.disable")]
    PluginDisable,
}

/// How an intercepting hook fails.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InterceptPolicy {
    pub failure: Failure,
}

/// What happens to a record when its plugin fails to decide it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Failure {
    /// As if it allowed it.
    Open,
    /// As if it refused it.
    Closed,
}

/// A plugin's own account.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Principal {
    /// Its username.
    pub username: String,
    /// A key of `messages` it is called by.
    pub display_name: String,
    /// The community permissions it asks for where it is turned on.
    #[serde(default)]
    pub permissions: Vec<Permission>,
    /// The commands it answers, as a bot publishes them (`spec/bot_commands.schema.json`).
    #[serde(default)]
    pub commands: Vec<Command>,
}

impl Manifest {
    /// Whether it asks for `permission`.
    pub fn asks(&self, permission: PluginPermission) -> bool {
        self.permissions.contains(&permission)
    }

    /// Every key of `messages` the manifest names, with where it names it.
    fn named_keys(&self) -> Vec<(&str, String)> {
        let mut keys = vec![
            (self.name.as_str(), "name".to_string()),
            (self.description.as_str(), "description".to_string()),
            (self.retention.as_str(), "retention".to_string()),
        ];
        for (fields, which) in [
            (&self.settings, "settings"),
            (&self.community_settings, "communitySettings"),
        ] {
            for field in fields {
                keys.push((
                    field.label.as_str(),
                    format!("{which}.{}.label", field.name),
                ));
                if let Some(description) = &field.description {
                    keys.push((
                        description.as_str(),
                        format!("{which}.{}.description", field.name),
                    ));
                }
                if let settings::FieldKind::Choice { options } = &field.kind {
                    for option in options {
                        keys.push((
                            option.label.as_str(),
                            format!("{which}.{}.{}", field.name, option.value),
                        ));
                    }
                }
            }
        }
        if let Some(principal) = &self.principal {
            keys.push((
                principal.display_name.as_str(),
                "principal.displayName".to_string(),
            ));
        }
        keys
    }

    /// Everything wrong with it that its schema cannot say, in the operator's words; empty when
    /// nothing is.
    pub fn check(&self) -> Vec<String> {
        let mut wrong = Vec::new();
        if !valid_id(&self.id) {
            wrong.push(format!(
                "id {:?} must be a domain its author controls, reversed (org.example.plugin): \
                 two or more labels of lowercase letters, digits, and hyphens, at most \
                 {MAX_ID_LENGTH} characters",
                self.id
            ));
        }
        if !valid_version(&self.version) {
            wrong.push(format!("version {:?} must be semver (1.2.3)", self.version));
        }
        if self.api != API_VERSION {
            wrong.push(format!(
                "it was built for plugin interface {}, and this Aspen speaks {API_VERSION}",
                self.api
            ));
        }
        match self.messages.get(&self.default_language) {
            None => wrong.push(format!(
                "messages has nothing in its default language, {}",
                self.default_language
            )),
            Some(defaults) => {
                for (key, place) in self.named_keys() {
                    if !defaults.contains_key(key) {
                        wrong.push(format!(
                            "{place} names {key:?}, which messages lacks in {}",
                            self.default_language
                        ));
                    }
                }
            }
        }
        for (language, texts) in &self.messages {
            for (key, text) in texts {
                if key.is_empty() || key.len() > 64 {
                    wrong.push(format!(
                        "messages.{language} has a key of 1 to 64 characters"
                    ));
                }
                if text.chars().count() > MAX_MESSAGE_CHARS {
                    wrong.push(format!(
                        "messages.{language}.{key} is longer than {MAX_MESSAGE_CHARS} characters"
                    ));
                }
            }
        }
        for (fields, community) in [(&self.settings, false), (&self.community_settings, true)] {
            let mut names = HashSet::new();
            for field in fields {
                if !names.insert(&field.name) {
                    wrong.push(format!("setting {} is declared twice", field.name));
                }
                if let Err(e) = settings::check_declaration(field, community) {
                    wrong.push(e);
                }
            }
        }
        let needs = |permission: PluginPermission, wrong: &mut Vec<String>, why: &str| {
            if !self.asks(permission) {
                wrong.push(format!("{why} needs the permission {permission}"));
            }
        };
        if !self.hooks.intercept.is_empty() {
            needs(
                PluginPermission::MessagesRead,
                &mut wrong,
                "intercepting messages",
            );
            if !self.asks(PluginPermission::MessagesRewrite)
                && !self.asks(PluginPermission::MessagesRefuse)
            {
                wrong.push(
                    "intercepting messages needs messages.rewrite or messages.refuse".to_string(),
                );
            }
        }
        for hook in &self.hooks.observe {
            match hook {
                ObserveHook::MessageCreate
                | ObserveHook::MessageEdit
                | ObserveHook::MessageDelete => needs(
                    PluginPermission::MessagesRead,
                    &mut wrong,
                    "observing messages",
                ),
                ObserveHook::CommandInvoke => {
                    needs(PluginPermission::Act, &mut wrong, "observing commands")
                }
                ObserveHook::PluginEnable | ObserveHook::PluginDisable => {}
            }
        }
        if self.asks(PluginPermission::Network) == self.hosts.is_empty() {
            wrong.push("network and hosts go together: list the hosts it calls".to_string());
        }
        for host in &self.hosts {
            if !valid_host(host) {
                wrong.push(format!("host {host:?} must be a domain name"));
            }
        }
        match (self.asks(PluginPermission::Storage), self.storage_quota) {
            (true, Some(quota)) if quota > 0 && quota <= MAX_STORAGE_QUOTA => {}
            (true, _) => wrong.push(format!(
                "storage needs a storageQuota of 1 to {MAX_STORAGE_QUOTA} bytes"
            )),
            (false, Some(_)) => wrong.push("storageQuota needs the permission storage".into()),
            (false, None) => {}
        }
        match (
            self.asks(PluginPermission::AttachmentsRead),
            self.attachment_limit,
        ) {
            (true, Some(limit)) if limit > 0 && limit <= MAX_ATTACHMENT_LIMIT => {}
            (true, _) => wrong.push(format!(
                "attachments.read needs an attachmentLimit of 1 to {MAX_ATTACHMENT_LIMIT} bytes"
            )),
            (false, Some(_)) => {
                wrong.push("attachmentLimit needs the permission attachments.read".into())
            }
            (false, None) => {}
        }
        match (&self.principal, self.asks(PluginPermission::Act)) {
            (Some(principal), true) => {
                if crate::app::user::validate_username(&principal.username).is_err() {
                    wrong.push(format!(
                        "principal.username {:?} is not a username",
                        principal.username
                    ));
                }
                let list = crate::app::bot_command::CommandList {
                    commands: principal.commands.clone(),
                };
                if let Err(e) = crate::app::bot_command::validate(&list) {
                    wrong.push(format!("principal.commands: {e}"));
                }
            }
            (None, true) => wrong.push("act needs a principal".to_string()),
            (Some(_), false) => wrong.push("principal needs the permission act".to_string()),
            (None, false) => {}
        }
        wrong
    }

    /// The permissions it asks for beyond `granted`, as an upgrade must ask again.
    pub fn more_than(&self, granted: &BTreeSet<PluginPermission>) -> Vec<PluginPermission> {
        self.permissions.difference(granted).copied().collect()
    }
}

/// Whether `id` is a reversed domain: two or more labels of lowercase letters, digits, and
/// hyphens, none starting or ending with a hyphen.
pub fn valid_id(id: &str) -> bool {
    id.len() <= MAX_ID_LENGTH && id.split('.').count() >= 2 && id.split('.').all(valid_label)
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_host(host: &str) -> bool {
    host.len() <= 253 && host.split('.').count() >= 2 && host.split('.').all(valid_label)
}

/// Whether `version` is semver: three numbers, then optionally a pre-release and build.
fn valid_version(version: &str) -> bool {
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let numbers: Vec<&str> = core.split('.').collect();
    numbers.len() == 3
        && numbers.iter().all(|n| {
            !n.is_empty()
                && n.chars().all(|c| c.is_ascii_digit())
                && (n == &"0" || !n.starts_with('0'))
        })
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word_filter() -> Manifest {
        serde_json::from_str(include_str!(
            "../../../../plugins/word_filter/aspen-plugin.json"
        ))
        .expect("the example plugin's manifest parses")
    }

    #[test]
    fn the_example_plugin_is_valid() {
        assert_eq!(word_filter().check(), Vec::<String>::new());
    }

    #[test]
    fn ids_are_reversed_domains() {
        assert!(valid_id("org.example.nocursing"));
        assert!(valid_id("com.example-2.a"));
        assert!(!valid_id("nocursing"));
        assert!(!valid_id("Org.Example.x"));
        assert!(!valid_id("org..x"));
        assert!(!valid_id("org.-x"));
    }

    #[test]
    fn versions_are_semver() {
        assert!(valid_version("1.0.0"));
        assert!(valid_version("0.3.12-beta.1+build5"));
        assert!(!valid_version("1.0"));
        assert!(!valid_version("01.0.0"));
    }

    #[test]
    fn what_is_asked_must_go_together() {
        let mut manifest = word_filter();
        manifest.permissions.remove(&PluginPermission::Act);
        assert!(
            manifest
                .check()
                .iter()
                .any(|e| e.contains("principal needs"))
        );
        let mut manifest = word_filter();
        manifest.messages.get_mut("en").unwrap().remove("refused");
        assert!(
            manifest.check().is_empty(),
            "refusals are named by the code, not the manifest"
        );
        manifest.messages.get_mut("en").unwrap().remove("name");
        assert!(manifest.check().iter().any(|e| e.contains("\"name\"")));
    }

    #[test]
    fn upgrades_ask_for_what_is_new() {
        let manifest = word_filter();
        let granted: BTreeSet<PluginPermission> =
            [PluginPermission::MessagesRead].into_iter().collect();
        assert!(
            manifest
                .more_than(&granted)
                .contains(&PluginPermission::Act)
        );
        assert!(manifest.more_than(&manifest.permissions).is_empty());
    }

    /// `spec/plugin_manifest.schema.json` is the schema of these types; `ASPEN_WRITE_SPEC=1
    /// cargo test` writes it afresh after they change.
    #[test]
    fn the_spec_matches_the_types() {
        let schema = schemars::schema_for!(Manifest);
        let written = serde_json::to_string_pretty(&schema).unwrap() + "\n";
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../spec/plugin_manifest.schema.json"
        );
        if std::env::var_os("ASPEN_WRITE_SPEC").is_some() {
            std::fs::write(path, &written).unwrap();
        }
        let on_disk = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            on_disk == written,
            "spec/plugin_manifest.schema.json differs from the types; run with ASPEN_WRITE_SPEC=1"
        );
    }
}
