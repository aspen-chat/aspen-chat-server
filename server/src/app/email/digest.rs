//! The daily digest: once a day, at the hour an account chose in its time zone, mail telling it
//! what arrived for it since the digest before, for an account that asked for one at a verified
//! address.
//!
//! A digest covers the channels the account may view in its communities and its DMs and group
//! DMs, never threads (whose replies their parents' readers follow in the app), and leaves out
//! what never makes a channel unread (`app::read_state`): the account's own messages, those of
//! anyone it blocked, and those it has read, as its read positions stand when the digest is
//! made, as well as channels and DMs it muted. Of what is left, it tells of the [`PER_PLACE`]
//! earliest messages of each of the [`MAX_PLACES`] places where something arrived most
//! recently, and how many more there are, linking to each place. A digest with nothing in it is
//! not sent.
//!
//! Each account's digest comes at a fixed point of its chosen hour ([`spread`]), so a popular
//! hour is an hour's trickle of digests rather than one burst at its start. Every sending server
//! looks for digests that are due each [`TICK`] ([`spawn_scheduler`]), claiming them under
//! `FOR UPDATE SKIP LOCKED`, so each is made once. What a digest says is fixed when it
//! is made and kept in the outbox with it; the names in it are as they were then.

use super::EmailAccount;
use super::outbox::{self, Mail};
use super::render::{Item, Letter, Section};
use crate::app::context::GlobalServerContext;
use crate::app::{self, ChannelId, CommunityId, CustomEmojiId, UserId};
use crate::database::schema::{
    channel, community, community_role, custom_emoji, dm_recipient, user, user_email,
};
use crate::t;
use chrono::{DateTime, Days, NaiveTime, TimeZone, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Nullable, SmallInt, Text, Timestamptz, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use pulldown_cmark::{Event, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

/// How many messages of one place a digest tells of.
pub const PER_PLACE: i64 = 5;
/// How many places a digest tells of.
pub const MAX_PLACES: i64 = 20;
/// The most unread messages of one place counted; more reads as this many.
const MAX_COUNTED: i64 = 10_000;
/// The longest excerpt of one message, in characters.
const EXCERPT_CHARS: usize = 300;
/// How often each server looks for digests that are due.
const TICK: Duration = Duration::from_secs(60);
/// How many digests one look claims.
const CLAIM: i64 = 20;

/// What a digest tells of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Digest {
    /// The account's time zone, which the times of its messages are given in.
    pub time_zone: String,
    pub places: Vec<Place>,
    /// Places with something unread beyond those told of.
    pub more_places: u32,
}

/// A channel or DM with something unread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub kind: PlaceKind,
    /// Where it opens in the app.
    pub link: String,
    pub messages: Vec<Excerpt>,
    /// Unread messages beyond those told of.
    pub more: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlaceKind {
    Channel {
        channel: String,
        community: String,
    },
    /// A DM or group DM, by its name, or by the names of the others in it.
    Dm {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Excerpt {
    pub author: String,
    pub at: DateTime<Utc>,
    /// The message's text as plain text, cut short; empty for a message of attachments alone.
    pub text: String,
}

impl Digest {
    /// The digest as mail, in the locale in scope.
    pub(super) fn letter(&self, deployment: &str) -> Letter {
        let zone = chrono_tz::Tz::from_str(&self.time_zone).unwrap_or(chrono_tz::UTC);
        let sections = self
            .places
            .iter()
            .map(|place| {
                let heading: String = match &place.kind {
                    PlaceKind::Channel { channel, community } => t!(
                        "digestChannelHeading",
                        channel = channel.as_str(),
                        community = community.as_str()
                    )
                    .into(),
                    PlaceKind::Dm { name } => name.clone(),
                };
                Section {
                    items: place
                        .messages
                        .iter()
                        .map(|message| Item {
                            author: message.author.clone(),
                            time: message.at.with_timezone(&zone).format("%H:%M").to_string(),
                            text: if message.text.is_empty() {
                                t!("digestNoText").into()
                            } else {
                                message.text.clone()
                            },
                        })
                        .collect(),
                    more: (place.more > 0).then(|| {
                        t!(
                            "digestMoreInPlace",
                            count = place.more,
                            place = heading.as_str()
                        )
                        .into()
                    }),
                    heading,
                    link: place.link.clone(),
                }
            })
            .collect();
        let mut outro = Vec::new();
        if self.more_places > 0 {
            outro.push(t!("digestMorePlaces", count = self.more_places).into());
        }
        Letter {
            subject: t!("digestSubject", deployment = deployment).into(),
            title: t!("digestTitle").into(),
            intro: vec![t!("digestIntro").into()],
            sections,
            outro,
            footer: vec![t!("digestFooter").into()],
            ..Letter::default()
        }
    }
}

/// How far into its chosen hour `user`'s digest comes: a fixed point of the hour drawn from the
/// random bits of their id, so the digests of everyone who chose one hour in one zone spread
/// over that hour rather than all coming due at its start.
pub fn spread(user: UserId) -> chrono::Duration {
    let seconds = user.0.as_u128() % 3600;
    chrono::Duration::seconds(i64::try_from(seconds).unwrap_or(0))
}

/// When the next digest of `account`, `user`'s, is due after `now`: the next time its chosen
/// hour comes in its time zone, [`spread`] into the hour. An hour a clock change skips is taken
/// as the hour after it. `None` while it has no digest.
pub fn next_due(account: &EmailAccount, user: UserId, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if !account.digest {
        return None;
    }
    let zone = chrono_tz::Tz::from_str(&account.digest_time_zone).unwrap_or(chrono_tz::UTC);
    let hour = u32::try_from(account.digest_hour).unwrap_or(0).min(23);
    let today = now.with_timezone(&zone).date_naive();
    (0..3u64).find_map(|days| {
        let date = today.checked_add_days(Days::new(days))?;
        let due = (hour..24).find_map(|hour| {
            let time = NaiveTime::from_hms_opt(hour, 0, 0)?;
            zone.from_local_datetime(&date.and_time(time)).earliest()
        })?;
        let due = due.with_timezone(&Utc) + spread(user);
        (due > now).then_some(due)
    })
}

/// Starts making digests as they fall due, for as long as the server runs.
pub fn spawn_scheduler(state: GlobalServerContext) {
    if !state.mailer.as_ref().is_some_and(|mailer| mailer.sends()) {
        return;
    }
    tokio::spawn(async move {
        loop {
            match make_due(&state).await {
                Ok(made) if made as i64 == CLAIM => continue,
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "could not make the digests that are due"),
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

#[derive(QueryableByName)]
struct Due {
    #[diesel(sql_type = PgUuid)]
    user: UserId,
    #[diesel(sql_type = Text)]
    digest_time_zone: String,
    #[diesel(sql_type = SmallInt)]
    digest_hour: i16,
    #[diesel(sql_type = Nullable<Timestamptz>)]
    digest_since: Option<DateTime<Utc>>,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    banned: bool,
}

/// Makes the digests that are due, a claim at a time, answering how many it claimed.
async fn make_due(state: &GlobalServerContext) -> app::Result<usize> {
    let mut conn = state.connection_pool.get().await?;
    let made = conn
        .transaction(|conn| {
            async move {
                let due: Vec<Due> = diesel::sql_query(format!(
                    r#"
                    SELECT e."user", e.digest_time_zone, e.digest_hour, e.digest_since,
                           {banned} AS banned
                    FROM user_email e
                    JOIN "user" ON "user".id = e."user"
                    WHERE e.digest
                      AND e.verified_at IS NOT NULL
                      AND e.digest_next_at <= now()
                      AND "user".deleted_at IS NULL
                    ORDER BY e.digest_next_at
                    LIMIT $1
                    FOR UPDATE OF e SKIP LOCKED
                    "#,
                    banned = app::user_ban::BANNED_SQL
                ))
                .bind::<BigInt, _>(CLAIM)
                .load(conn)
                .await?;
                let now = Utc::now();
                for account in &due {
                    // A banned account's digest is skipped, not saved up.
                    if !account.banned {
                        let since = account.digest_since.unwrap_or(now - chrono::Days::new(1));
                        let digest =
                            build(state, conn, account.user, since, &account.digest_time_zone)
                                .await?;
                        if let Some(digest) = digest {
                            outbox::queue(conn, account.user, None, &Mail::Digest { digest })
                                .await?;
                        }
                    }
                    let next = next_due(
                        &EmailAccount {
                            address: String::new(),
                            verified_at: None,
                            shown: false,
                            newsletter: false,
                            digest: true,
                            digest_time_zone: account.digest_time_zone.clone(),
                            digest_hour: account.digest_hour,
                            digest_next_at: None,
                            locale: String::new(),
                        },
                        account.user,
                        now,
                    );
                    diesel::update(user_email::table.filter(user_email::user.eq(account.user)))
                        .set((
                            user_email::digest_since.eq(now),
                            user_email::digest_next_at.eq(next),
                        ))
                        .execute(conn)
                        .await?;
                }
                Ok::<_, app::Error>(due.len())
            }
            .scope_boxed()
        })
        .await?;
    Ok(made)
}

#[derive(QueryableByName)]
struct UnreadPlace {
    #[diesel(sql_type = PgUuid)]
    channel: ChannelId,
    #[diesel(sql_type = Nullable<PgUuid>)]
    community: Option<CommunityId>,
    #[diesel(sql_type = BigInt)]
    unread: i64,
}

#[derive(QueryableByName)]
struct UnreadMessage {
    #[diesel(sql_type = PgUuid)]
    channel: ChannelId,
    #[diesel(sql_type = PgUuid)]
    author: UserId,
    #[diesel(sql_type = Text)]
    content: String,
    #[diesel(sql_type = Timestamptz)]
    timestamp: DateTime<Utc>,
}

/// The condition that message `m` of place `c` is unread for the user `$1`, with `cu`, `dr`, and
/// `rs` their membership, recipiency, and read state there: someone else's but no one they
/// blocked, not deleted, posted after `$3` (the digest before) and after they arrived, and after
/// their read position.
const UNREAD_SQL: &str = r#"
    m.channel = c.id
    AND m.deleted_at IS NULL
    AND m.author <> $1
    AND m."timestamp" > $3
    AND m."timestamp" > COALESCE(cu.joined_at, dr.joined_at)
    AND (rs.message IS NULL OR m.id > rs.message)
    AND NOT EXISTS (
        SELECT 1 FROM user_block b WHERE b.blocker = $1 AND b.blocked = m.author
    )
"#;

/// The places among `$2` the user belongs to, as SQL over `c`, with the joins [`UNREAD_SQL`]
/// reads, and the condition that they are places the user still belongs to and has not muted.
const PLACES_FROM: &str = r#"
    FROM channel c
    LEFT JOIN community_user cu ON cu.community = c.community AND cu."user" = $1
    LEFT JOIN dm_recipient dr ON dr.channel = c.id AND dr."user" = $1
    LEFT JOIN read_state rs ON rs.channel = c.id AND rs."user" = $1
"#;
const PLACES_WHERE: &str = r#"
    c.id = ANY($2)
    AND c.deleted_at IS NULL
    AND c.parent_channel IS NULL
    AND (cu."user" IS NOT NULL OR dr."user" IS NOT NULL)
    AND NOT EXISTS (
        SELECT 1 FROM channel_mute cm
        WHERE cm."user" = $1 AND cm.channel = c.id
          AND (cm.until IS NULL OR cm.until > now())
    )
"#;

/// The digest of what arrived for `user` after `since`, or `None` when nothing did.
pub async fn build(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    since: DateTime<Utc>,
    time_zone: &str,
) -> app::Result<Option<Digest>> {
    let communities = app::events::memberships(conn, user_id).await?;
    let visible = app::visibility::Visibility::load(state, user_id, &communities).await?;
    let mut places: Vec<ChannelId> = visible.visible_channels();
    let dms: Vec<ChannelId> = dm_recipient::table
        .select(dm_recipient::channel)
        .filter(dm_recipient::user.eq(user_id))
        .load(conn)
        .await?;
    places.extend(dms);
    if places.is_empty() {
        return Ok(None);
    }
    let place_ids: Vec<uuid::Uuid> = places.iter().map(|c| c.0).collect();
    let unread: Vec<UnreadPlace> = diesel::sql_query(format!(
        r#"
        SELECT c.id AS channel, c.community AS community, n.unread AS unread
        {PLACES_FROM}
        CROSS JOIN LATERAL (
            SELECT count(*) AS unread, (array_agg(u.id ORDER BY u.id DESC))[1] AS newest
            FROM (SELECT m.id FROM message m WHERE {UNREAD_SQL} ORDER BY m.id DESC LIMIT $4) u
        ) n
        WHERE {PLACES_WHERE} AND n.unread > 0
        ORDER BY n.newest DESC
        "#
    ))
    .bind::<PgUuid, _>(user_id.0)
    .bind::<Array<PgUuid>, _>(&place_ids)
    .bind::<Timestamptz, _>(since)
    .bind::<BigInt, _>(MAX_COUNTED)
    .load(conn)
    .await?;
    if unread.is_empty() {
        return Ok(None);
    }
    let more_places = unread.len().saturating_sub(MAX_PLACES as usize);
    let told: Vec<&UnreadPlace> = unread.iter().take(MAX_PLACES as usize).collect();
    let told_ids: Vec<uuid::Uuid> = told.iter().map(|p| p.channel.0).collect();
    let messages: Vec<UnreadMessage> = diesel::sql_query(format!(
        r#"
        SELECT c.id AS channel, u.author, u.content, u."timestamp"
        {PLACES_FROM}
        CROSS JOIN LATERAL (
            SELECT m.author, m.content, m."timestamp", m.id FROM message m
            WHERE {UNREAD_SQL}
            ORDER BY m.id
            LIMIT $4
        ) u
        WHERE {PLACES_WHERE}
        ORDER BY u.id
        "#
    ))
    .bind::<PgUuid, _>(user_id.0)
    .bind::<Array<PgUuid>, _>(&told_ids)
    .bind::<Timestamptz, _>(since)
    .bind::<BigInt, _>(PER_PLACE)
    .load(conn)
    .await?;
    let names = Names::load(conn, user_id, &told, &messages).await?;
    let web_client = state.config.public_url.as_str();
    let mut by_place: HashMap<ChannelId, Vec<&UnreadMessage>> = HashMap::new();
    for message in &messages {
        by_place.entry(message.channel).or_default().push(message);
    }
    let places = told
        .iter()
        .map(|place| {
            let excerpts: Vec<Excerpt> = by_place
                .get(&place.channel)
                .map(|messages| {
                    messages
                        .iter()
                        .map(|m| Excerpt {
                            author: names.user(m.author),
                            at: m.timestamp,
                            text: names.excerpt(&m.content),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let shown = excerpts.len() as i64;
            Place {
                kind: names.place(place),
                link: link(web_client, place.community, place.channel),
                more: u32::try_from(place.unread - shown).unwrap_or(0),
                messages: excerpts,
            }
        })
        .collect();
    Ok(Some(Digest {
        time_zone: time_zone.to_string(),
        places,
        more_places: u32::try_from(more_places).unwrap_or(u32::MAX),
    }))
}

/// Where a place opens in the deployment's web client.
fn link(web_client: &str, community: Option<CommunityId>, channel: ChannelId) -> String {
    match community {
        Some(community) => format!(
            "{web_client}/communities/{}/channels/{}",
            community.0, channel.0
        ),
        None => format!("{web_client}/dms/{}", channel.0),
    }
}

/// The names a digest calls things by.
struct Names {
    users: HashMap<UserId, String>,
    roles: HashMap<uuid::Uuid, String>,
    emoji: HashMap<CustomEmojiId, String>,
    channels: HashMap<ChannelId, String>,
    communities: HashMap<CommunityId, String>,
    /// The others in each DM, by name.
    recipients: HashMap<ChannelId, Vec<String>>,
}

impl Names {
    async fn load(
        conn: &mut AsyncPgConnection,
        reader: UserId,
        places: &[&UnreadPlace],
        messages: &[UnreadMessage],
    ) -> app::Result<Self> {
        let place_ids: Vec<ChannelId> = places.iter().map(|p| p.channel).collect();
        let community_ids: Vec<CommunityId> = places.iter().filter_map(|p| p.community).collect();
        let mut tagged_users: Vec<UserId> = messages.iter().map(|m| m.author).collect();
        let mut tagged_roles = Vec::new();
        let mut emoji_ids = Vec::new();
        for message in messages {
            let requested = app::mention::parse(&message.content);
            tagged_users.extend(requested.users.into_iter().map(UserId));
            tagged_roles.extend(requested.roles);
            emoji_ids.extend(custom_emoji_in(&message.content));
        }
        let recipient_rows: Vec<(ChannelId, UserId)> = dm_recipient::table
            .select((dm_recipient::channel, dm_recipient::user))
            .filter(dm_recipient::channel.eq_any(&place_ids))
            .filter(dm_recipient::user.ne(reader))
            .load(conn)
            .await?;
        tagged_users.extend(recipient_rows.iter().map(|(_, user)| *user));
        tagged_users.sort();
        tagged_users.dedup();
        let users: HashMap<UserId, String> = user::table
            .select((user::id, user::name, user::display_name))
            .filter(user::id.eq_any(&tagged_users))
            .load::<(UserId, String, Option<String>)>(conn)
            .await?
            .into_iter()
            .map(|(id, name, display_name)| (id, display_name.unwrap_or(name)))
            .collect();
        let roles = community_role::table
            .select((community_role::id, community_role::name))
            .filter(community_role::id.eq_any(&tagged_roles))
            .load::<(uuid::Uuid, String)>(conn)
            .await?
            .into_iter()
            .collect();
        let emoji = custom_emoji::table
            .select((custom_emoji::id, custom_emoji::name))
            .filter(custom_emoji::id.eq_any(&emoji_ids))
            .load::<(CustomEmojiId, String)>(conn)
            .await?
            .into_iter()
            .collect();
        let channels = channel::table
            .select((channel::id, channel::name))
            .filter(channel::id.eq_any(&place_ids))
            .load::<(ChannelId, String)>(conn)
            .await?
            .into_iter()
            .collect();
        let communities = community::table
            .select((community::id, community::name))
            .filter(community::id.eq_any(&community_ids))
            .load::<(CommunityId, String)>(conn)
            .await?
            .into_iter()
            .collect();
        let mut recipients: HashMap<ChannelId, Vec<String>> = HashMap::new();
        for (channel, user) in recipient_rows {
            if let Some(name) = users.get(&user) {
                recipients.entry(channel).or_default().push(name.clone());
            }
        }
        Ok(Self {
            users,
            roles,
            emoji,
            channels,
            communities,
            recipients,
        })
    }

    fn user(&self, id: UserId) -> String {
        self.users.get(&id).cloned().unwrap_or_default()
    }

    fn place(&self, place: &UnreadPlace) -> PlaceKind {
        let name = self
            .channels
            .get(&place.channel)
            .cloned()
            .unwrap_or_default();
        match place.community {
            Some(community) => PlaceKind::Channel {
                channel: name,
                community: self
                    .communities
                    .get(&community)
                    .cloned()
                    .unwrap_or_default(),
            },
            None if !name.is_empty() => PlaceKind::Dm { name },
            None => PlaceKind::Dm {
                name: self
                    .recipients
                    .get(&place.channel)
                    .map(|names| names.join(", "))
                    .unwrap_or_default(),
            },
        }
    }

    /// `content` as plain text, its tags and custom emoji by name, cut to [`EXCERPT_CHARS`].
    fn excerpt(&self, content: &str) -> String {
        let mut text = String::new();
        for event in app::markdown::parser(content) {
            match event {
                Event::Text(t) | Event::Code(t) => text.push_str(&t),
                Event::SoftBreak
                | Event::HardBreak
                | Event::End(TagEnd::Paragraph | TagEnd::Item | TagEnd::Heading(_))
                | Event::Start(Tag::CodeBlock(_)) => text.push(' '),
                _ => {}
            }
        }
        let named = self.name_tokens(&text);
        let collapsed = named.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.chars().count() <= EXCERPT_CHARS {
            return collapsed;
        }
        let mut cut: String = collapsed.chars().take(EXCERPT_CHARS - 1).collect();
        cut.push('…');
        cut
    }

    /// `text` with each `<@user>`, `<@&role>`, and `<:emoji>` written as the name it stands for.
    fn name_tokens(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find('<') {
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            let Some(end) = rest.find('>') else {
                break;
            };
            let token = &rest[..=end];
            let named = if let Some(id) = token
                .strip_prefix("<@&")
                .and_then(|t| uuid::Uuid::parse_str(&t[..t.len() - 1]).ok())
            {
                self.roles.get(&id).map(|name| format!("@{name}"))
            } else if let Some(id) = token
                .strip_prefix("<@")
                .and_then(|t| uuid::Uuid::parse_str(&t[..t.len() - 1]).ok())
            {
                self.users.get(&UserId(id)).map(|name| format!("@{name}"))
            } else {
                app::custom_emoji::referenced(token)
                    .and_then(|id| self.emoji.get(&id))
                    .map(|name| format!(":{name}:"))
            };
            match named {
                Some(name) => {
                    out.push_str(&name);
                    rest = &rest[end + 1..];
                }
                None => {
                    out.push('<');
                    rest = &rest[1..];
                }
            }
        }
        out.push_str(rest);
        out
    }
}

/// The custom emoji `content` names.
fn custom_emoji_in(content: &str) -> Vec<CustomEmojiId> {
    let mut found = Vec::new();
    let mut rest = content;
    while let Some(start) = rest.find("<:") {
        rest = &rest[start..];
        let Some(end) = rest.find('>') else {
            break;
        };
        if let Some(id) = app::custom_emoji::referenced(&rest[..=end]) {
            found.push(id);
        }
        rest = &rest[end + 1..];
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(zone: &str, hour: i16) -> EmailAccount {
        EmailAccount {
            address: "a@example.org".to_string(),
            verified_at: None,
            shown: false,
            newsletter: false,
            digest: true,
            digest_time_zone: zone.to_string(),
            digest_hour: hour,
            digest_next_at: None,
            locale: "en".to_string(),
        }
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn a_digest_is_due_at_the_next_chosen_hour_in_its_zone() {
        let due = next_due(
            &account("America/New_York", 8),
            ON_THE_HOUR,
            at("2026-10-04T10:00:00Z"),
        );
        assert_eq!(due, Some(at("2026-10-04T12:00:00Z")));
        let due = next_due(
            &account("America/New_York", 8),
            ON_THE_HOUR,
            at("2026-10-04T12:00:00Z"),
        );
        assert_eq!(due, Some(at("2026-10-05T12:00:00Z")));
    }

    /// A user whose digest comes on the hour.
    const ON_THE_HOUR: UserId = UserId(uuid::Uuid::from_u128(3600 * 7));

    #[test]
    fn digests_spread_over_their_hour() {
        let late = UserId(uuid::Uuid::from_u128(3599));
        assert_eq!(spread(ON_THE_HOUR), chrono::Duration::zero());
        assert_eq!(spread(late), chrono::Duration::seconds(3599));
        let due = next_due(&account("UTC", 8), late, at("2026-10-04T08:30:00Z"));
        assert_eq!(due, Some(at("2026-10-04T08:59:59Z")));
        let due = next_due(&account("UTC", 8), late, at("2026-10-04T09:00:00Z"));
        assert_eq!(due, Some(at("2026-10-05T08:59:59Z")));
        // Ids made together spread across the hour.
        let spreads: std::collections::HashSet<_> = (0..100)
            .map(|_| spread(UserId::new()).num_seconds() / 600)
            .collect();
        assert_eq!(spreads.len(), 6, "{spreads:?}");
    }

    #[test]
    fn an_hour_a_clock_change_skips_is_taken_as_the_next() {
        // Clocks in New York went from 02:00 to 03:00 on 8 March 2026.
        let due = next_due(
            &account("America/New_York", 2),
            ON_THE_HOUR,
            at("2026-03-08T05:00:00Z"),
        );
        assert_eq!(due, Some(at("2026-03-08T07:00:00Z")));
    }

    #[test]
    fn no_digest_is_due_while_it_is_off() {
        let mut off = account("UTC", 8);
        off.digest = false;
        assert_eq!(next_due(&off, ON_THE_HOUR, Utc::now()), None);
    }

    fn names() -> Names {
        let user = UserId(uuid::Uuid::from_u128(1));
        let role = uuid::Uuid::from_u128(2);
        let emoji = CustomEmojiId(uuid::Uuid::from_u128(3));
        Names {
            users: HashMap::from([(user, "Alex".to_string())]),
            roles: HashMap::from([(role, "Mods".to_string())]),
            emoji: HashMap::from([(emoji, "party".to_string())]),
            channels: HashMap::new(),
            communities: HashMap::new(),
            recipients: HashMap::new(),
        }
    }

    #[test]
    fn excerpts_are_plain_text_with_names() {
        let user = uuid::Uuid::from_u128(1);
        let role = uuid::Uuid::from_u128(2);
        let emoji = uuid::Uuid::from_u128(3);
        let text = format!("**Hi** <@{user}> and <@&{role}> <:{emoji}>\n\n- one\n- `two`");
        assert_eq!(names().excerpt(&text), "Hi @Alex and @Mods :party: one two");
    }

    #[test]
    fn long_excerpts_are_cut() {
        let text = "ж".repeat(EXCERPT_CHARS * 2);
        let excerpt = names().excerpt(&text);
        assert_eq!(excerpt.chars().count(), EXCERPT_CHARS);
        assert!(excerpt.ends_with('…'));
    }

    #[test]
    fn unknown_tokens_are_left_as_they_are() {
        assert_eq!(names().excerpt("a < b <@nope> c"), "a < b <@nope> c");
    }

    #[test]
    fn links_open_in_the_web_client() {
        let community = CommunityId(uuid::Uuid::from_u128(1));
        let channel = ChannelId(uuid::Uuid::from_u128(2));
        assert_eq!(
            link("https://chat.example.org", Some(community), channel),
            format!(
                "https://chat.example.org/communities/{}/channels/{}",
                community.0, channel.0
            )
        );
        assert_eq!(
            link("https://chat.example.org", None, channel),
            format!("https://chat.example.org/dms/{}", channel.0)
        );
    }
}
