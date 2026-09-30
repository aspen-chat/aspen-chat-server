//! The commands a bot advertises, which people invoke by typing `/name` in a channel the bot
//! can see or a DM with it, and which clients complete from each parameter's type.
//!
//! A bot publishes one list (`PUT /bots/{bot}/commands`, by the bot or its owner), the same
//! wherever it is used; a bot of another deployment publishes its list on each deployment it
//! uses the same way. The list is validated as a whole when published, against the rules here
//! and the published schema (`spec/bot_commands.schema.json`, which the tests hold to these
//! types), and kept in `bot_command_list`. Names are any printable characters of any script but
//! whitespace, which separates a command's arguments, and emoji. A change is announced as
//! `botCommandsChanged` wherever the bot's profile would be, so clients read the list again.

use crate::api::GlobalServerContext;
use crate::api::message_enum::server_event::ServerEvent;
use crate::app::permissions::{ChannelAccess, Permissions, channel_access, require_member};
use crate::app::{self, AttachmentId, ChannelId, CommunityId, EventScope, UserId, publish_event};
use crate::database::schema::bot_command_list;
use crate::t;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use utoipa::ToSchema;

/// The most commands one bot may advertise.
pub const MAX_COMMANDS: usize = 100;
/// The most parameters one command may take.
pub const MAX_PARAMETERS: usize = 25;
/// The longest a command's or parameter's name may be, in characters.
pub const MAX_NAME_CHARS: usize = 32;
/// The longest a description may be, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 100;
/// The most languages a description may be given in besides its own.
pub const MAX_TRANSLATIONS: usize = 50;
/// The longest a `regex` parameter's pattern may be, in characters.
pub const MAX_PATTERN_CHARS: usize = 256;
/// What a pattern's compiled program may use, which keeps any pattern quick to check.
const PATTERN_SIZE_LIMIT: usize = 1 << 16;
/// The largest a whole published list may be, as JSON.
pub const MAX_LIST_BYTES: usize = 64 * 1024;

/// Everything a bot advertises it will answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandList {
    pub commands: Vec<Command>,
}

/// One command: what people type after `/`, what it does, and what it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    /// What people type after `/`: 1 to 32 printable characters of any script, but no
    /// whitespace or emoji. Unique among the bot's commands, ignoring case.
    pub name: String,
    /// What it does, in the bot's own language, 1 to 100 characters.
    pub description: String,
    /// The description in other languages, by BCP 47 tag, which a client shows in place of
    /// `description` when its language matches.
    #[serde(default)]
    pub descriptions: BTreeMap<String, String>,
    /// What it takes, in order; optional ones come last.
    #[serde(default)]
    pub parameters: Vec<Parameter>,
}

/// One thing a command takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    /// Its name, shown while it is typed; the same rules as a command's.
    pub name: String,
    /// What to give it, 1 to 100 characters.
    pub description: String,
    #[serde(default)]
    pub descriptions: BTreeMap<String, String>,
    /// What it must be, which the server checks before the bot hears of it.
    #[serde(rename = "type")]
    pub ty: ParameterType,
    /// Whether it may be left out; only the last parameters may be.
    #[serde(default)]
    pub optional: bool,
    /// For a `regex` parameter, and only for one: what a value must match whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

/// What a parameter's value must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ParameterType {
    /// Someone's id; a client completes it from the people there.
    UserId,
    /// A channel's id, one the person invoking may view.
    ChannelId,
    /// A message's id, one the person invoking may read.
    MessageId,
    /// A community's id, one the person invoking belongs to.
    CommunityId,
    /// A role's id, of the community the command is invoked in.
    RoleId,
    /// A file the person invoking attaches to the command.
    AttachmentId,
    /// A deployment's domain, as federation names deployments.
    DeploymentHost,
    /// One emoji.
    React,
    /// Any text; as the last parameter it takes the rest of what was typed.
    Any,
    /// Text that matches the parameter's `pattern` whole.
    Regex,
}

/// Why a list was refused, for the bot's developer: what is wrong, and where.
fn invalid(key: &'static str, command: &str, extra: Option<&str>) -> app::Error {
    let message: Cow<'static, str> = match extra {
        Some(extra) => t!(key, command = command, detail = extra),
        None => t!(key, command = command),
    };
    app::Error::Validation(message)
}

/// Whether a character may be part of a name: printable, not whitespace, and not an emoji or a
/// piece of one (a joiner, a presentation selector, a keycap, a regional indicator, a skin
/// tone).
fn name_char(c: char) -> bool {
    !(c.is_control()
        || c.is_whitespace()
        || matches!(c, '\u{200D}' | '\u{FE0E}' | '\u{FE0F}' | '\u{20E3}')
        || ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
        || ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
        || emojis::get(c.encode_utf8(&mut [0; 4])).is_some())
}

fn valid_name(name: &str) -> bool {
    let count = name.chars().count();
    (1..=MAX_NAME_CHARS).contains(&count) && name.chars().all(name_char)
}

fn valid_description(text: &str) -> bool {
    (1..=MAX_DESCRIPTION_CHARS).contains(&text.trim().chars().count())
}

fn valid_translations(translations: &BTreeMap<String, String>) -> bool {
    translations.len() <= MAX_TRANSLATIONS
        && translations.iter().all(|(tag, text)| {
            (1..=35).contains(&tag.len())
                && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && valid_description(text)
        })
}

/// A pattern of the subset every client can check alike: no longer than
/// `MAX_PATTERN_CHARS`, compiling within `PATTERN_SIZE_LIMIT`, and without the constructs
/// JavaScript reads differently or not at all (group flags and named groups, `\A` and `\z`,
/// POSIX classes). It must match a value whole.
pub fn compile_pattern(pattern: &str) -> Option<regex::Regex> {
    if pattern.chars().count() > MAX_PATTERN_CHARS
        || pattern.contains("\\A")
        || pattern.contains("\\z")
        || pattern.contains("[[:")
        || pattern
            .match_indices("(?")
            .any(|(at, _)| !pattern[at..].starts_with("(?:"))
    {
        return None;
    }
    regex::RegexBuilder::new(&format!("^(?:{pattern})$"))
        .size_limit(PATTERN_SIZE_LIMIT)
        .dfa_size_limit(PATTERN_SIZE_LIMIT)
        .build()
        .ok()
}

/// Checks a list as a whole, naming the first thing wrong with it.
pub fn validate(list: &CommandList) -> app::Result<()> {
    let bytes = serde_json::to_vec(list)
        .map(|b| b.len())
        .unwrap_or(usize::MAX);
    if bytes > MAX_LIST_BYTES {
        return Err(app::Error::Validation(t!(
            "botCommandsTooLarge",
            max = MAX_LIST_BYTES
        )));
    }
    if list.commands.len() > MAX_COMMANDS {
        return Err(app::Error::Validation(t!(
            "botCommandsTooMany",
            max = MAX_COMMANDS
        )));
    }
    let mut names = HashSet::new();
    for command in &list.commands {
        let name = command.name.as_str();
        if !valid_name(name) {
            return Err(invalid("botCommandName", name, None));
        }
        if !names.insert(name.to_lowercase()) {
            return Err(invalid("botCommandTwice", name, None));
        }
        if !valid_description(&command.description) || !valid_translations(&command.descriptions) {
            return Err(invalid("botCommandDescription", name, None));
        }
        if command.parameters.len() > MAX_PARAMETERS {
            return Err(invalid("botCommandParameters", name, None));
        }
        let mut parameters = HashSet::new();
        let mut optional_seen = false;
        for parameter in &command.parameters {
            let label = parameter.name.as_str();
            if !valid_name(label) || !parameters.insert(label.to_lowercase()) {
                return Err(invalid("botCommandParameterName", name, Some(label)));
            }
            if !valid_description(&parameter.description)
                || !valid_translations(&parameter.descriptions)
            {
                return Err(invalid("botCommandParameterDescription", name, Some(label)));
            }
            if optional_seen && !parameter.optional {
                return Err(invalid("botCommandOptionalOrder", name, Some(label)));
            }
            optional_seen |= parameter.optional;
            match (parameter.ty, parameter.pattern.as_deref()) {
                (ParameterType::Regex, Some(pattern)) => {
                    if compile_pattern(pattern).is_none() {
                        return Err(invalid("botCommandPattern", name, Some(label)));
                    }
                }
                (ParameterType::Regex, None) => {
                    return Err(invalid("botCommandPatternMissing", name, Some(label)));
                }
                (_, Some(_)) => {
                    return Err(invalid("botCommandPatternUnwanted", name, Some(label)));
                }
                (_, None) => {}
            }
        }
    }
    Ok(())
}

/// Publishes `list` as `bot`'s commands, which the bot itself or its owner may do, and tells
/// everyone who can see the bot that it changed.
pub async fn publish(
    state: &GlobalServerContext,
    caller: UserId,
    bot: UserId,
    list: CommandList,
) -> app::Result<CommandList> {
    validate(&list)?;
    let mut conn = state.connection_pool.get().await?;
    if caller != bot && !app::bot::owns(conn.as_mut(), caller, bot).await? {
        return Err(app::Error::Forbidden(t!("botNotYours")));
    }
    let stored = serde_json::to_value(&list)?;
    conn.transaction(|conn| {
        async move {
            diesel::insert_into(bot_command_list::table)
                .values((
                    bot_command_list::bot.eq(bot),
                    bot_command_list::commands.eq(&stored),
                ))
                .on_conflict(bot_command_list::bot)
                .do_update()
                .set((
                    bot_command_list::commands.eq(&stored),
                    bot_command_list::updated_at.eq(diesel::dsl::now),
                ))
                .execute(conn)
                .await?;
            publish_event(
                state,
                conn,
                EventScope::UserEverywhere(bot),
                &ServerEvent::BotCommandsChanged { bot },
            )
            .await?;
            Ok::<_, app::Error>(())
        }
        .scope_boxed()
    })
    .await?;
    Ok(list)
}

/// The commands `bot` advertises; none when it has published none.
pub async fn read(state: &GlobalServerContext, bot: UserId) -> app::Result<CommandList> {
    let mut conn = state.connection_pool.get().await?;
    let stored: Option<serde_json::Value> = bot_command_list::table
        .select(bot_command_list::commands)
        .filter(bot_command_list::bot.eq(bot))
        .first(conn.as_mut())
        .await
        .optional()?;
    Ok(stored
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default())
}

/// One bot's commands, where a channel offers them.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BotCommands {
    pub bot: UserId,
    pub commands: Vec<Command>,
}

/// The commands on offer in `channel` to `caller`, who must be able to view it: those of each
/// bot that can view it too, the community's bots in a community channel and the DM's in a DM,
/// a thread counting as its parent. Bots that advertise nothing are left out.
pub async fn for_channel(
    state: &GlobalServerContext,
    caller: UserId,
    channel: ChannelId,
) -> app::Result<Vec<BotCommands>> {
    use crate::database::schema::{channel as channel_table, community_user, dm_recipient, user};
    let mut conn = state.connection_pool.get().await?;
    let access = channel_access(state, conn.as_mut(), caller, channel).await?;
    let candidates: Vec<UserId> = match &access.community {
        Some(community) => {
            community_user::table
                .inner_join(user::table.on(user::id.eq(community_user::user)))
                .select(user::id)
                .filter(
                    community_user::community
                        .eq(community.community)
                        .and(user::bot)
                        .and(user::deleted_at.is_null()),
                )
                .load(conn.as_mut())
                .await?
        }
        None => {
            let parent: Option<ChannelId> = channel_table::table
                .select(channel_table::parent_channel)
                .filter(channel_table::id.eq(channel))
                .first(conn.as_mut())
                .await?;
            dm_recipient::table
                .inner_join(user::table.on(user::id.eq(dm_recipient::user)))
                .select(user::id)
                .filter(
                    dm_recipient::channel
                        .eq(parent.unwrap_or(channel))
                        .and(user::bot)
                        .and(user::deleted_at.is_null()),
                )
                .load(conn.as_mut())
                .await?
        }
    };
    let mut present = Vec::new();
    for bot in candidates {
        // A bot hears only what it can see, in a community channel whose overrides may hide it.
        let sees = match channel_access(state, conn.as_mut(), bot, channel).await {
            Ok(bot_access) => bot_access.has(Permissions::VIEW_CHANNEL),
            Err(app::Error::Diesel(diesel::result::Error::NotFound)) => false,
            Err(e) => return Err(e),
        };
        if sees {
            present.push(bot);
        }
    }
    let lists: Vec<(UserId, serde_json::Value)> = bot_command_list::table
        .select((bot_command_list::bot, bot_command_list::commands))
        .filter(bot_command_list::bot.eq_any(&present))
        .load(conn.as_mut())
        .await?;
    Ok(lists
        .into_iter()
        .filter_map(|(bot, value)| {
            let list: CommandList = serde_json::from_value(value).ok()?;
            (!list.commands.is_empty()).then_some(BotCommands {
                bot,
                commands: list.commands,
            })
        })
        .collect())
}

/// The longest a text argument may be, in characters.
pub const MAX_ARGUMENT_CHARS: usize = 2000;

/// A command someone invokes: which bot, which of its commands, what they gave it, in order,
/// and the files its `attachmentId` arguments name.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Invocation {
    pub bot: UserId,
    pub name: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<AttachmentId>,
}

/// One argument as the bot receives it: named, typed, and checked against its type, an emoji
/// in its fully qualified form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Argument {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ParameterType,
    pub value: String,
}

/// Why an invocation was refused, for the person who typed it.
fn refused(key: &'static str, command: &str, parameter: &str) -> app::Error {
    app::Error::Validation(t!(key, command = command, parameter = parameter))
}

/// Whether `bot` can see `channel`, which the caller's `access` is to: a member of its
/// community who may view it, or one of the DM's people, a thread counting as its parent.
async fn bot_present(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    access: &ChannelAccess,
    channel: ChannelId,
    bot: UserId,
) -> app::Result<bool> {
    use crate::database::schema::{channel as channel_table, dm_recipient, user};
    let is_bot: Option<bool> = user::table
        .select(user::bot)
        .filter(user::id.eq(bot).and(user::deleted_at.is_null()))
        .first(conn)
        .await
        .optional()?;
    if is_bot != Some(true) {
        return Ok(false);
    }
    if access.community.is_some() {
        return match channel_access(state, conn, bot, channel).await {
            Ok(bot_access) => Ok(bot_access.has(Permissions::VIEW_CHANNEL)),
            Err(app::Error::Diesel(diesel::result::Error::NotFound)) => Ok(false),
            Err(e) => Err(e),
        };
    }
    let parent: Option<ChannelId> = channel_table::table
        .select(channel_table::parent_channel)
        .filter(channel_table::id.eq(channel))
        .first(conn)
        .await?;
    let found: i64 = dm_recipient::table
        .filter(
            dm_recipient::channel
                .eq(parent.unwrap_or(channel))
                .and(dm_recipient::user.eq(bot)),
        )
        .count()
        .get_result(conn)
        .await?;
    Ok(found > 0)
}

/// Checks an invocation by `caller` in `channel`: that the bot can see the channel and
/// answers the command, that the arguments are as many as it takes, and that each is what its
/// parameter says, as the caller may reach it. Returns the command and the arguments as the
/// bot will receive them.
pub(crate) async fn check(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    caller: UserId,
    access: &ChannelAccess,
    channel: ChannelId,
    invocation: &Invocation,
) -> app::Result<(Command, Vec<Argument>)> {
    use crate::database::schema::{attachment, community_role, message, user};
    let name = invocation.name.as_str();
    if !bot_present(state, conn, access, channel, invocation.bot).await? {
        return Err(refused("botCommandBotAbsent", name, ""));
    }
    let stored: Option<serde_json::Value> = bot_command_list::table
        .select(bot_command_list::commands)
        .filter(bot_command_list::bot.eq(invocation.bot))
        .first(conn)
        .await
        .optional()?;
    let list: CommandList = stored
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let wanted = name.to_lowercase();
    let Some(command) = list
        .commands
        .into_iter()
        .find(|c| c.name.to_lowercase() == wanted)
    else {
        return Err(refused("botCommandUnknown", name, ""));
    };
    let required = command.parameters.iter().filter(|p| !p.optional).count();
    if !(required..=command.parameters.len()).contains(&invocation.arguments.len()) {
        return Err(app::Error::Validation(t!(
            "botCommandArity",
            command = command.name.as_str(),
            least = required,
            most = command.parameters.len(),
            given = invocation.arguments.len()
        )));
    }
    let community = access.community.as_ref().map(|c| c.community);
    let mut arguments = Vec::with_capacity(invocation.arguments.len());
    for (parameter, value) in command.parameters.iter().zip(&invocation.arguments) {
        let reason = match parameter.ty {
            ParameterType::UserId => t!("botArgumentUser"),
            ParameterType::ChannelId => t!("botArgumentChannel"),
            ParameterType::MessageId => t!("botArgumentMessage"),
            ParameterType::CommunityId => t!("botArgumentCommunity"),
            ParameterType::RoleId => t!("botArgumentRole"),
            ParameterType::AttachmentId => t!("botArgumentAttachment"),
            ParameterType::DeploymentHost => t!("botArgumentHost"),
            ParameterType::React => t!("botArgumentReact"),
            ParameterType::Any => t!("botArgumentAny", max = MAX_ARGUMENT_CHARS),
            ParameterType::Regex => t!("botArgumentRegex"),
        };
        let bad = || {
            app::Error::Validation(t!(
                "botCommandArgument",
                command = command.name.as_str(),
                parameter = parameter.name.as_str(),
                reason = reason.as_ref()
            ))
        };
        let id = || uuid::Uuid::parse_str(value.trim()).map_err(|_| bad());
        let value = match parameter.ty {
            ParameterType::UserId => {
                let id = id()?;
                let exists: i64 = user::table
                    .filter(user::id.eq(id).and(user::deleted_at.is_null()))
                    .count()
                    .get_result(conn)
                    .await?;
                if exists == 0 {
                    return Err(bad());
                }
                id.to_string()
            }
            ParameterType::ChannelId => {
                let id = id()?;
                if channel_access(state, conn, caller, ChannelId(id))
                    .await
                    .is_err()
                {
                    return Err(bad());
                }
                id.to_string()
            }
            ParameterType::MessageId => {
                let id = id()?;
                let home: Option<ChannelId> = message::table
                    .select(message::channel)
                    .filter(message::id.eq(id).and(message::deleted_at.is_null()))
                    .first(conn)
                    .await
                    .optional()?;
                let Some(home) = home else {
                    return Err(bad());
                };
                if channel_access(state, conn, caller, home).await.is_err() {
                    return Err(bad());
                }
                id.to_string()
            }
            ParameterType::CommunityId => {
                let id = id()?;
                if require_member(conn, caller, CommunityId(id)).await.is_err() {
                    return Err(bad());
                }
                id.to_string()
            }
            ParameterType::RoleId => {
                let id = id()?;
                let Some(community) = community else {
                    return Err(bad());
                };
                let found: i64 = community_role::table
                    .filter(
                        community_role::id
                            .eq(id)
                            .and(community_role::community.eq(community)),
                    )
                    .count()
                    .get_result(conn)
                    .await?;
                if found == 0 {
                    return Err(bad());
                }
                id.to_string()
            }
            ParameterType::AttachmentId => {
                let id = AttachmentId(id()?);
                // A file the command takes goes with it, shown in the channel as the
                // invocation's own.
                if !invocation.attachments.contains(&id) {
                    return Err(bad());
                }
                let ready: i64 = attachment::table
                    .filter(
                        attachment::id
                            .eq(id)
                            .and(attachment::ready_at.is_not_null()),
                    )
                    .count()
                    .get_result(conn)
                    .await?;
                if ready == 0 {
                    return Err(bad());
                }
                id.0.to_string()
            }
            ParameterType::DeploymentHost => {
                let domain = app::federation::Domain::parse(value.trim()).map_err(|_| bad())?;
                String::from(domain)
            }
            ParameterType::React => app::react::canonical_emoji(value.trim())
                .ok_or_else(bad)?
                .to_string(),
            ParameterType::Any => {
                if value.is_empty() || value.chars().count() > MAX_ARGUMENT_CHARS {
                    return Err(bad());
                }
                value.clone()
            }
            ParameterType::Regex => {
                let matches = parameter
                    .pattern
                    .as_deref()
                    .and_then(compile_pattern)
                    .is_some_and(|pattern| pattern.is_match(value));
                if !matches || value.chars().count() > MAX_ARGUMENT_CHARS {
                    return Err(bad());
                }
                value.clone()
            }
        };
        arguments.push(Argument {
            name: parameter.name.clone(),
            ty: parameter.ty,
            value,
        });
    }
    // Every file sent with the command is one of its arguments.
    for attached in &invocation.attachments {
        let named = arguments
            .iter()
            .any(|a| a.ty == ParameterType::AttachmentId && a.value == attached.0.to_string());
        if !named {
            return Err(refused("botCommandStrayAttachment", &command.name, ""));
        }
    }
    Ok((command, arguments))
}

/// The command as the channel shows it: `/name` and its arguments, people and roles as the
/// tags a message names them by (which tag no one, the message's mentions being empty), and
/// anything else quoted where it holds whitespace or a quote, or is empty.
pub fn invocation_text(command: &Command, arguments: &[Argument]) -> String {
    let mut text = format!("/{}", command.name);
    for argument in arguments {
        text.push(' ');
        match argument.ty {
            ParameterType::UserId => {
                text.push_str(&format!("<@{}>", argument.value));
                continue;
            }
            ParameterType::RoleId => {
                text.push_str(&format!("<@&{}>", argument.value));
                continue;
            }
            _ => {}
        }
        let plain = !argument.value.is_empty()
            && !argument
                .value
                .chars()
                .any(|c| c.is_whitespace() || c == '"');
        if plain {
            text.push_str(&argument.value);
        } else {
            text.push('"');
            text.push_str(&argument.value.replace('\\', "\\\\").replace('"', "\\\""));
            text.push('"');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str) -> Command {
        Command {
            name: name.into(),
            description: "Does a thing".into(),
            descriptions: BTreeMap::new(),
            parameters: vec![],
        }
    }

    fn parameter(name: &str, ty: ParameterType) -> Parameter {
        Parameter {
            name: name.into(),
            description: "A thing".into(),
            descriptions: BTreeMap::new(),
            ty,
            optional: false,
            pattern: None,
        }
    }

    fn list(commands: Vec<Command>) -> CommandList {
        CommandList { commands }
    }

    #[test]
    fn names_are_any_script_but_never_spaces_or_emoji() {
        for name in ["roll", "ПОМОЩЬ", "天気", "حالة", "café-2", "x"] {
            assert!(validate(&list(vec![command(name)])).is_ok(), "{name}");
        }
        for name in [
            "",
            "two words",
            "🎲",
            "roll🎲",
            "a\u{200D}b",
            "x".repeat(33).as_str(),
        ] {
            assert!(validate(&list(vec![command(name)])).is_err(), "{name:?}");
        }
    }

    #[test]
    fn a_name_is_used_once_ignoring_case() {
        assert!(validate(&list(vec![command("Roll"), command("roll")])).is_err());
        assert!(validate(&list(vec![command("roll"), command("rolls")])).is_ok());
    }

    #[test]
    fn optional_parameters_come_last() {
        let mut c = command("remind");
        let mut later = parameter("when", ParameterType::Any);
        later.optional = true;
        c.parameters = vec![later, parameter("who", ParameterType::UserId)];
        assert!(validate(&list(vec![c.clone()])).is_err());
        c.parameters.reverse();
        assert!(validate(&list(vec![c])).is_ok());
    }

    #[test]
    fn a_regex_parameter_has_a_pattern_of_the_common_subset_and_only_it_does() {
        let mut c = command("dice");
        let mut sides = parameter("sides", ParameterType::Regex);
        sides.pattern = Some("[0-9]+d[0-9]+".into());
        c.parameters = vec![sides.clone()];
        assert!(validate(&list(vec![c.clone()])).is_ok());
        for bad in ["(?i)x", "(?P<n>x)", "\\Ax", "[[:alpha:]]", "("] {
            sides.pattern = Some(bad.into());
            c.parameters = vec![sides.clone()];
            assert!(validate(&list(vec![c.clone()])).is_err(), "{bad}");
        }
        c.parameters = vec![parameter("sides", ParameterType::Regex)];
        assert!(validate(&list(vec![c.clone()])).is_err());
        let mut plain = parameter("who", ParameterType::UserId);
        plain.pattern = Some("x".into());
        c.parameters = vec![plain];
        assert!(validate(&list(vec![c])).is_err());
    }

    #[test]
    fn a_command_shows_as_sent_quoting_what_would_split() {
        let c = command("say");
        let argument = |value: &str| Argument {
            name: "x".into(),
            ty: ParameterType::Any,
            value: value.into(),
        };
        assert_eq!(invocation_text(&c, &[]), "/say");
        assert_eq!(
            invocation_text(
                &c,
                &[
                    argument("2d6"),
                    argument("for luck"),
                    argument("a \"quote\"")
                ]
            ),
            "/say 2d6 \"for luck\" \"a \\\"quote\\\"\""
        );
        let tagged = |ty, value: &str| Argument {
            name: "x".into(),
            ty,
            value: value.into(),
        };
        assert_eq!(
            invocation_text(
                &c,
                &[
                    tagged(ParameterType::UserId, "u"),
                    tagged(ParameterType::RoleId, "r")
                ]
            ),
            "/say <@u> <@&r>"
        );
    }

    #[test]
    fn a_pattern_matches_a_value_whole() {
        let pattern = compile_pattern("[0-9]+d[0-9]+").unwrap();
        assert!(pattern.is_match("2d6"));
        assert!(!pattern.is_match("roll 2d6 now"));
    }

    /// `spec/bot_commands.schema.json` is what bots are told to publish; it is these types.
    /// `ASPEN_WRITE_SPEC=1 cargo test` writes it afresh after they change.
    #[test]
    fn the_published_schema_is_these_types() {
        let schema =
            serde_json::to_string_pretty(&schemars::schema_for!(CommandList)).unwrap() + "\n";
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../spec/bot_commands.schema.json"
        );
        if std::env::var_os("ASPEN_WRITE_SPEC").is_some() {
            std::fs::write(path, &schema).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(path).unwrap_or_default(),
            schema,
            "spec/bot_commands.schema.json differs from the types; run with ASPEN_WRITE_SPEC=1"
        );
    }

    #[test]
    fn the_counts_are_capped() {
        let many: Vec<Command> = (0..=MAX_COMMANDS)
            .map(|i| command(&format!("c{i}")))
            .collect();
        assert!(validate(&list(many)).is_err());
        let mut c = command("wide");
        c.parameters = (0..=MAX_PARAMETERS)
            .map(|i| parameter(&format!("p{i}"), ParameterType::Any))
            .collect();
        assert!(validate(&list(vec![c])).is_err());
    }
}
