//! Tagging people in messages. A message's text tags a member as `<@user-id>`, a role as
//! `<@&role-id>`, and everyone who can see the channel as `@everyone`, wherever it renders as
//! text and not code (`app::markdown`). Each kind
//! takes a channel permission (Mention members, Mention roles, Mention everyone); a tag its
//! author may not make, or that names no member or role here, is left as plain text and counts
//! for no one. What does count is kept on the message as its record carries it
//! ([`Mentions`]), and once per tag in `mention`, from which each reader's unread tags in a
//! channel are counted (`app::read_state`). Editing a message's text tags afresh, with its
//! author's permissions as they are then.

use crate::app::events::{ChannelHome, channel_home, dm_recipients};
use crate::app::permissions::{ChannelAccess, Permissions};
use crate::app::{self, ChannelId, GlobalServerContext, MessageId, RoleId, UserId};
use crate::database::schema::{community_role, community_user, mention};
use diesel::deserialize::{FromSql, FromSqlRow};
use diesel::expression::AsExpression;
use diesel::pg::{Pg, PgValue};
use diesel::prelude::*;
use diesel::serialize::{IsNull, Output, ToSql};
use diesel::sql_types::Jsonb;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use pulldown_cmark::{Event, Tag, TagEnd};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::Write;
use utoipa::ToSchema;
use uuid::Uuid;

/// The most distinct people and roles one message may tag; any beyond are plain text.
pub const MAX_TAGS: usize = 50;

/// Who a message tags, as far as its author was allowed to: these are the tags that count.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[diesel(sql_type = Jsonb)]
#[serde(rename_all = "camelCase")]
pub struct Mentions {
    /// Members tagged by name, in the order first tagged.
    pub users: Vec<UserId>,
    /// Roles tagged, in the order first tagged; everyone's is tagged as `everyone` instead.
    pub roles: Vec<RoleId>,
    /// Whether the message tags everyone who can see the channel.
    pub everyone: bool,
}

impl Mentions {
    pub fn is_empty(&self) -> bool {
        self.users.is_empty() && self.roles.is_empty() && !self.everyone
    }
}

// Postgres sends and takes `jsonb` as a version byte, 1, followed by the JSON text.
impl FromSql<Jsonb, Pg> for Mentions {
    fn from_sql(value: PgValue<'_>) -> diesel::deserialize::Result<Self> {
        let bytes = value.as_bytes();
        match bytes.split_first() {
            Some((1, json)) => Ok(serde_json::from_slice(json)?),
            _ => Err("unsupported jsonb encoding".into()),
        }
    }
}

impl ToSql<Jsonb, Pg> for Mentions {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        out.write_all(&[1])?;
        serde_json::to_writer(out, self)?;
        Ok(IsNull::No)
    }
}

/// The tags a message's text asks for, before permissions and membership are applied.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Requested {
    pub users: Vec<Uuid>,
    pub roles: Vec<Uuid>,
    pub everyone: bool,
}

/// Finds the tags in `content` wherever it is text as a message renders it (`app::markdown`),
/// never in code of any kind, each kept once in the order first written, at most [`MAX_TAGS`]
/// people and roles together.
pub fn parse(content: &str) -> Requested {
    let mut found = Requested::default();
    // A run of text may come as several events; it is read whole, up to the next markup.
    let mut run = String::new();
    let mut code_depth = 0u32;
    for event in crate::app::markdown::parser(content) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                scan_line(&std::mem::take(&mut run), &mut found);
                code_depth += 1;
            }
            Event::End(TagEnd::CodeBlock) => code_depth = code_depth.saturating_sub(1),
            _ if code_depth > 0 => {}
            Event::Text(text) => run.push_str(&text),
            Event::SoftBreak | Event::HardBreak => run.push('\n'),
            _ => scan_line(&std::mem::take(&mut run), &mut found),
        }
    }
    scan_line(&run, &mut found);
    found
}

fn scan_line(line: &str, found: &mut Requested) {
    let mut rest = line;
    let mut previous: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some((tag, len)) = tag_at(rest)
        {
            if found.users.len() + found.roles.len() < MAX_TAGS {
                match tag {
                    Found::User(id) if !found.users.contains(&id) => found.users.push(id),
                    Found::Role(id) if !found.roles.contains(&id) => found.roles.push(id),
                    _ => {}
                }
            }
            rest = &rest[len..];
            previous = Some('>');
            continue;
        }
        if c == '@'
            && rest.starts_with("@everyone")
            && !previous.is_some_and(is_word)
            && !rest["@everyone".len()..]
                .chars()
                .next()
                .is_some_and(is_word)
        {
            found.everyone = true;
        }
        previous = Some(c);
        rest = &rest[c.len_utf8()..];
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

enum Found {
    User(Uuid),
    Role(Uuid),
}

/// The tag `<@id>` or `<@&id>` at the start of `text`, and its length in bytes.
fn tag_at(text: &str) -> Option<(Found, usize)> {
    let (role, body) = if let Some(body) = text.strip_prefix("<@&") {
        (true, body)
    } else {
        (false, text.strip_prefix("<@")?)
    };
    let end = body.find('>')?;
    let id = Uuid::parse_str(body.get(..end)?).ok()?;
    let len = text.len() - body.len() + end + 1;
    Some((if role { Found::Role(id) } else { Found::User(id) }, len))
}

/// The tags among `requested` that count in `channel`, posted by someone with `access`
/// there: people who belong where the channel is (the community, or the DM), the community's
/// roles but everyone's, and everyone, each only with its permission.
pub async fn resolve(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
    access: &ChannelAccess,
    requested: Requested,
) -> app::Result<Mentions> {
    let mut mentions = Mentions {
        everyone: requested.everyone && access.has(Permissions::MENTION_EVERYONE),
        ..Mentions::default()
    };
    let users: Vec<UserId> = if access.has(Permissions::MENTION_MEMBERS) {
        requested.users.into_iter().map(UserId).collect()
    } else {
        Vec::new()
    };
    let roles: Vec<RoleId> = if access.has(Permissions::MENTION_ROLES) {
        requested.roles.into_iter().map(RoleId).collect()
    } else {
        Vec::new()
    };
    match channel_home(state, conn, channel).await? {
        ChannelHome::Community { community, .. } => {
            if !users.is_empty() {
                let members: Vec<UserId> = community_user::table
                    .select(community_user::user)
                    .filter(
                        community_user::community
                            .eq(community)
                            .and(community_user::user.eq_any(&users)),
                    )
                    .load(conn)
                    .await?;
                mentions.users = users.into_iter().filter(|u| members.contains(u)).collect();
            }
            if !roles.is_empty() {
                let known: Vec<RoleId> = community_role::table
                    .select(community_role::id)
                    .filter(
                        community_role::community
                            .eq(community)
                            .and(community_role::everyone.eq(false))
                            .and(community_role::id.eq_any(&roles)),
                    )
                    .load(conn)
                    .await?;
                mentions.roles = roles.into_iter().filter(|r| known.contains(r)).collect();
            }
        }
        ChannelHome::Direct(dm) => {
            if !users.is_empty() {
                let recipients = dm_recipients(conn, dm).await?;
                mentions.users = users
                    .into_iter()
                    .filter(|u| recipients.contains(u))
                    .collect();
            }
        }
    }
    Ok(mentions)
}

/// Writes a message's tags for the unread counts to read, replacing any it had, inside the
/// caller's transaction.
pub async fn record(
    conn: &mut AsyncPgConnection,
    message: MessageId,
    channel: ChannelId,
    mentions: &Mentions,
) -> app::Result<()> {
    diesel::delete(mention::table.filter(mention::message.eq(message)))
        .execute(conn)
        .await?;
    let mut rows = Vec::new();
    for user in &mentions.users {
        rows.push((
            mention::message.eq(message),
            mention::channel.eq(channel),
            mention::target_user.eq(Some(*user)),
            mention::target_role.eq(None::<RoleId>),
            mention::everyone.eq(false),
        ));
    }
    for role in &mentions.roles {
        rows.push((
            mention::message.eq(message),
            mention::channel.eq(channel),
            mention::target_user.eq(None::<UserId>),
            mention::target_role.eq(Some(*role)),
            mention::everyone.eq(false),
        ));
    }
    if mentions.everyone {
        rows.push((
            mention::message.eq(message),
            mention::channel.eq(channel),
            mention::target_user.eq(None::<UserId>),
            mention::target_role.eq(None::<RoleId>),
            mention::everyone.eq(true),
        ));
    }
    if !rows.is_empty() {
        diesel::insert_into(mention::table)
            .values(rows)
            .execute(conn)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "01a0e95a-8b0f-75df-a00b-29a4f0b878d1";
    const B: &str = "01a0e95a-8b0f-75df-a00b-29a4f0b878d2";

    fn id(text: &str) -> Uuid {
        Uuid::parse_str(text).unwrap()
    }

    #[test]
    fn people_roles_and_everyone_are_found_once_each() {
        let found = parse(&format!("hi <@{A}> and <@&{B}>, <@{A}> again, @everyone!"));
        assert_eq!(found.users, vec![id(A)]);
        assert_eq!(found.roles, vec![id(B)]);
        assert!(found.everyone);
    }

    #[test]
    fn code_is_not_tagging() {
        let found = parse(&format!(
            "`<@{A}>` and ``@everyone``\n```\n<@&{B}>\n```\nbut <@{B}>"
        ));
        assert_eq!(found.users, vec![id(B)]);
        assert!(found.roles.is_empty());
        assert!(!found.everyone);
    }

    #[test]
    fn no_kind_of_code_is_tagging() {
        let found = parse(&format!(
            "    <@{A}> indented\n\n````\n```\n<@{B}>\n```\n````\n\n- item `<@&{A}>`\n\n> ```\n> @everyone\n> ```"
        ));
        assert_eq!(found, Requested::default());
    }

    #[test]
    fn tags_in_markup_count() {
        let found = parse(&format!(
            "**<@{A}>** _@everyone_ [see <@&{B}>](https://example.com)"
        ));
        assert_eq!(found.users, vec![id(A)]);
        assert_eq!(found.roles, vec![id(B)]);
        assert!(found.everyone);
    }

    #[test]
    fn everyone_must_stand_alone() {
        assert!(!parse("mail@everyone.example").everyone);
        assert!(!parse("@everyones").everyone);
        assert!(parse("(@everyone)").everyone);
    }

    #[test]
    fn a_malformed_tag_is_text() {
        let found = parse("<@not-a-uuid> <@&> <@");
        assert_eq!(found, Requested::default());
    }

    #[test]
    fn tags_stop_at_the_cap() {
        let content: String = (0..MAX_TAGS + 5)
            .map(|n| format!("<@{}> ", Uuid::from_u128(n as u128 + 1)))
            .collect();
        assert_eq!(parse(&content).users.len(), MAX_TAGS);
    }
}
