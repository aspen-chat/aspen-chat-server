//! Plugins: WebAssembly components the operator installs to change what the deployment does,
//! as `spec/plugins.md` designs them and `spec/plugin.wit` declares their interface.
//!
//! - `manifest` and `settings`: what a plugin declares and how its settings are checked.
//! - `registry`: the plugins this server runs, compiled, and which run where.
//! - `host`: the sandbox each call runs in, and every host call a plugin may make.
//! - `intercept`: deciding messages before they are saved.
//! - `observe`: handing plugins what happened, through a durable consumer each.
//! - `annotation`: what plugins say about messages and people.
//! - `storage`: what plugins keep, by scope.
//! - `principal`: a plugin's own account, and where it is a member.
//! - `community`: communities turning plugins on and configuring them.
//! - `install`: installing, upgrading, configuring, and removing plugins, from the terminal and
//!   the dashboard.
//!
//! Every read a plugin makes goes through the host, which decides it as Aspen decides it for a
//! person: as the caller of a route, or as the plugin's principal, and only where the plugin
//! runs. No plugin reaches credentials, sessions, sign-in, deployment roles, or federation.

pub mod annotation;
pub mod asset;
pub mod capability;
pub mod card;
pub mod channel_type;
pub mod community;
mod host;
pub mod install;
pub mod intercept;
pub mod manifest;
pub mod notice;
pub mod observe;
pub mod principal;
pub mod registry;
pub mod route;
pub mod settings;
pub mod storage;
pub mod timer;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

pub use aspen_wire::plugin::PluginText;
pub use registry::Plugins;

/// The version of the plugin interface (`spec/plugin.wit`) this host speaks.
pub const API_VERSION: u32 = 1;

/// The subject on which a change to installed plugins, or to a community's use of one, is
/// announced to every API server, which reloads what it holds. Outside the event stream: it is
/// for servers, never clients.
pub const CHANGED_SUBJECT: &str = "aspen.plugins.changed";

/// What a plugin may do, as its manifest asks and the operator grants.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    strum::VariantArray,
)]
pub enum PluginPermission {
    /// Be shown messages in the hooks it answers.
    #[serde(rename = "messages.read")]
    MessagesRead,
    /// Change a message's text before it is saved.
    #[serde(rename = "messages.rewrite")]
    MessagesRewrite,
    /// Refuse a message before it is saved.
    #[serde(rename = "messages.refuse")]
    MessagesRefuse,
    /// Attach annotations to messages.
    #[serde(rename = "messages.annotate")]
    MessagesAnnotate,
    /// Attach annotations to people's profiles.
    #[serde(rename = "users.annotate")]
    UsersAnnotate,
    /// Read the bytes of attachments of messages it is shown.
    #[serde(rename = "attachments.read")]
    AttachmentsRead,
    /// Run in DMs, which the operator grants by name.
    #[serde(rename = "dms")]
    Dms,
    /// Call the hosts its manifest lists.
    #[serde(rename = "network")]
    Network,
    /// Keep data of its own.
    #[serde(rename = "storage")]
    Storage,
    /// Answer requests at routes of its own.
    #[serde(rename = "routes")]
    Routes,
    /// Publish events of its own.
    #[serde(rename = "events")]
    Events,
    /// Have an account of its own, its principal, and act through it.
    #[serde(rename = "act")]
    Act,
    /// Serve pages of its own, its assets, to people's apps.
    #[serde(rename = "views")]
    Views,
    /// Add the kinds of channel its manifest declares.
    #[serde(rename = "channelTypes")]
    ChannelTypes,
    /// Be called back at times it sets.
    #[serde(rename = "timers")]
    Timers,
    /// Tell people of something, as Aspen tells them of a message.
    #[serde(rename = "notify")]
    Notify,
    /// Give a person a private URL of their own to one of its routes.
    #[serde(rename = "capabilities")]
    Capabilities,
}

crate::wire_name_traits!(PluginPermission);

/// How far an installed plugin reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Its hooks run in every community, which may configure it but not turn it off.
    Everywhere,
    /// Its hooks run in the communities that turn it on.
    OptIn,
}

crate::wire_name_traits!(Mode);

/// A plugin's text in each language it speaks: language tag to key to text.
pub type Messages = BTreeMap<String, BTreeMap<String, String>>;

/// The languages to try for `locale`, best first: the tag whole, then with its subtags taken
/// off one by one, then `default`.
fn languages<'a>(locale: &'a str, default: &'a str) -> impl Iterator<Item = &'a str> {
    let mut tags = Vec::new();
    let mut tag = locale;
    loop {
        tags.push(tag);
        match tag.rfind('-') {
            Some(end) => tag = &tag[..end],
            None => break,
        }
    }
    tags.push(default);
    tags.into_iter()
}

/// How a pseudo-locale (`app::locale`) makes its text from a plugin's default language.
fn pseudo(locale: &str) -> Option<fn(&str) -> String> {
    match locale {
        crate::locale::ACCENTED => Some(crate::locale::accented),
        crate::locale::MIRRORED => Some(crate::locale::mirrored),
        _ => None,
    }
}

/// The text of `key` in `locale`, from the language that best matches it and has the key, with
/// its placeholders filled; the key itself when no language has it. A pseudo-locale is made
/// from the plugin's default language.
pub fn render(
    messages: &Messages,
    default_language: &str,
    locale: &str,
    text: &PluginText,
) -> String {
    let template = match pseudo(locale) {
        Some(transform) => messages
            .get(default_language)
            .and_then(|m| m.get(&text.key))
            .map(|template| transform(template)),
        None => languages(locale, default_language)
            .find_map(|language| messages.get(language).and_then(|m| m.get(&text.key)))
            .cloned(),
    };
    let Some(template) = template else {
        return text.key.clone();
    };
    let mut out = template;
    for (name, value) in &text.args {
        out = out.replace(&format!("%{{{name}}}"), value);
    }
    out
}

/// A plugin's messages for one reader: the best match for their language of every key, with
/// placeholders left for the reader's client to fill. A pseudo-locale is made from the plugin's
/// default language.
pub fn catalogue(
    messages: &Messages,
    default_language: &str,
    locale: &str,
) -> BTreeMap<String, String> {
    if let Some(transform) = pseudo(locale) {
        return messages
            .get(default_language)
            .map(|texts| {
                texts
                    .iter()
                    .map(|(k, v)| (k.clone(), transform(v)))
                    .collect()
            })
            .unwrap_or_default();
    }
    let mut out = BTreeMap::new();
    for language in languages(locale, default_language)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if let Some(texts) = messages.get(language) {
            out.extend(texts.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    out
}

/// Who receives a plugin's event.
#[derive(Debug, Clone, Copy)]
pub enum Target {
    /// Whoever may view the channel.
    Channel(crate::ChannelId),
    /// The community's members.
    Community(crate::CommunityId),
    /// That user alone.
    User(crate::UserId),
}

/// Publishes `plugin`'s event of `kind` to `target`, routed as any event there is, so who may
/// view a channel decides who receives its plugin's events.
pub async fn publish(
    state: &crate::context::GlobalServerContext,
    conn: &mut diesel_async::AsyncPgConnection,
    plugin: &str,
    kind: String,
    target: Target,
    payload: serde_json::Value,
) -> crate::Result<()> {
    use aspen_wire::message_enum::server_event::ServerEvent;
    let (scope, channel, community) = match target {
        Target::Channel(channel) => {
            let community = match crate::events::channel_home(state, conn, channel).await? {
                crate::events::ChannelHome::Community { community, .. } => Some(community),
                crate::events::ChannelHome::Direct(_) => None,
            };
            (
                crate::EventScope::Channel(channel),
                Some(channel),
                community,
            )
        }
        Target::Community(community) => (
            crate::EventScope::Community(community),
            None,
            Some(community),
        ),
        Target::User(user) => (crate::EventScope::User(user), None, None),
    };
    crate::publish_event(
        state,
        conn,
        scope,
        &ServerEvent::PluginEvent {
            plugin: plugin.to_string(),
            kind,
            channel,
            community,
            payload,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages() -> Messages {
        let mut messages = Messages::new();
        messages.insert(
            "en".into(),
            [
                ("hello".to_string(), "Hello, %{name}".to_string()),
                ("only".to_string(), "English only".to_string()),
            ]
            .into(),
        );
        messages.insert(
            "fr".into(),
            [("hello".to_string(), "Bonjour, %{name}".to_string())].into(),
        );
        messages
    }

    #[test]
    fn text_is_rendered_in_the_best_language_that_has_it() {
        let hello = PluginText {
            key: "hello".into(),
            args: [("name".to_string(), "Ana".to_string())].into(),
        };
        assert_eq!(render(&messages(), "en", "fr-CA", &hello), "Bonjour, Ana");
        assert_eq!(render(&messages(), "en", "de", &hello), "Hello, Ana");
        let only = PluginText {
            key: "only".into(),
            args: BTreeMap::new(),
        };
        assert_eq!(render(&messages(), "en", "fr", &only), "English only");
        let missing = PluginText {
            key: "missing".into(),
            args: BTreeMap::new(),
        };
        assert_eq!(render(&messages(), "en", "fr", &missing), "missing");
    }

    #[test]
    fn a_catalogue_takes_each_key_from_the_best_language() {
        let fr = catalogue(&messages(), "en", "fr");
        assert_eq!(fr["hello"], "Bonjour, %{name}");
        assert_eq!(fr["only"], "English only");
    }

    #[test]
    fn permissions_have_dotted_wire_names() {
        assert_eq!(PluginPermission::MessagesRead.to_string(), "messages.read");
        assert_eq!(
            "dms".parse::<PluginPermission>().unwrap(),
            PluginPermission::Dms
        );
    }
}
