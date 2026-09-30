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
use crate::app::permissions::{Permissions, channel_access};
use crate::app::{self, ChannelId, EventScope, UserId, publish_event};
use crate::database::schema::bot_command_list;
use crate::t;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
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
