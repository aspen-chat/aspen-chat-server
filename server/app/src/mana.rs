//! Channel mana: a budget each channel spends as messages are posted and earns back over time,
//! which bounds how fast it fills however many people post in it. See
//! `docs/architecture/channel-mana/index.md`.

use crate::context::GlobalServerContext;
use crate::deployment_settings::DeploymentSettings;
use crate::{ChannelId, t};
use aspen_wire::channel::ChannelType;
use diesel::sql_types::{Integer, Nullable, Uuid as PgUuid};
use diesel::{OptionalExtension, QueryableByName};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use fred::interfaces::LuaInterface;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

pub mod queue;

/// How often a pool earns back mana: five times a second.
pub const TICK: Duration = Duration::from_millis(200);
/// Mana is kept in thousandths, so every rate a second divides evenly into ticks.
pub const MILLI: i64 = 1000;

/// The capacity and recharge of a pool as they apply now, in whole mana.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolSettings {
    /// Mana earned back a second.
    pub recharge: u32,
    /// The most the pool holds.
    pub max: u32,
}

impl PoolSettings {
    /// The deployment's: what a pool that sets none of its own has, and the most one may set.
    pub fn deployment(settings: &DeploymentSettings) -> Self {
        Self {
            recharge: settings.mana_max_recharge,
            max: settings.mana_max,
        }
    }

    /// A pool's settings from its channel's own (`None` where it sets none, as a group DM's
    /// always are) and the deployment's, which are also their ceilings: a channel set above
    /// one, before the deployment's was lowered, is held to it.
    pub fn of(deployment: Self, recharge: Option<i32>, max: Option<i32>) -> Self {
        let own = |value: Option<i32>, ceiling: u32| {
            value
                .and_then(|value| u32::try_from(value).ok())
                .map_or(ceiling, |value| value.min(ceiling))
                .max(1)
        };
        Self {
            recharge: own(recharge, deployment.recharge),
            max: own(max, deployment.max),
        }
    }
}

/// Refuses a channel's own settings when they are not positive, or above the deployment's.
pub fn check_channel_settings(
    deployment: PoolSettings,
    recharge: Option<u32>,
    max: Option<u32>,
) -> crate::Result<()> {
    if let Some(recharge) = recharge {
        if recharge == 0 {
            return Err(crate::Error::Validation(t!("manaRechargeZero")));
        }
        if recharge > deployment.recharge {
            return Err(crate::Error::Validation(t!(
                "manaRechargeAboveDeployment",
                max = deployment.recharge
            )));
        }
    }
    if let Some(max) = max {
        if max == 0 {
            return Err(crate::Error::Validation(t!("manaMaxZero")));
        }
        if max > deployment.max {
            return Err(crate::Error::Validation(t!(
                "manaMaxAboveDeployment",
                max = deployment.max
            )));
        }
    }
    Ok(())
}

/// A message's text as glyphs: grapheme clusters, what a reader sees as one character, in
/// every script alike. A tag (`<@id>`, `<@&id>`), a custom emoji (`<:id>`), and `@everyone`
/// are one glyph each however long their stored form; each is ASCII, one cluster a byte, so
/// counting it whole and taking back the rest is exact.
pub fn glyphs(content: &str) -> u32 {
    let mut count = content.graphemes(true).count();
    let mut rest = content;
    while let Some(at) = rest.find('<') {
        rest = &rest[at..];
        let token = if let Some((_, len)) = crate::mention::tag_at(rest) {
            len
        } else if rest.len() >= EMOJI_TOKEN_LEN
            && rest.is_char_boundary(EMOJI_TOKEN_LEN)
            && crate::custom_emoji::referenced(&rest[..EMOJI_TOKEN_LEN]).is_some()
        {
            EMOJI_TOKEN_LEN
        } else {
            1
        };
        count = count.saturating_sub(token - 1);
        rest = &rest[token..];
    }
    count = count.saturating_sub(content.matches(EVERYONE).count() * (EVERYONE.len() - 1));
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// `<:` and a hyphenated UUID and `>`.
const EMOJI_TOKEN_LEN: usize = 2 + 36 + 1;
const EVERYONE: &str = "@everyone";

/// What a message costs: its glyphs, and each attachment the deployment's
/// `mana_attachment_cost`.
pub fn message_cost(settings: &DeploymentSettings, content: &str, attachments: usize) -> u32 {
    cost(settings.mana_attachment_cost, content, attachments)
}

/// What an edit costs: the glyphs it adds beyond the message's length before, and its added
/// attachments. Shortening or rewording costs nothing.
pub fn edit_cost(
    settings: &DeploymentSettings,
    before: &str,
    after: &str,
    added_attachments: usize,
) -> u32 {
    cost(settings.mana_attachment_cost, "", added_attachments)
        .saturating_add(glyphs(after).saturating_sub(glyphs(before)))
}

fn cost(attachment_cost: u32, content: &str, attachments: usize) -> u32 {
    glyphs(content).saturating_add(
        attachment_cost.saturating_mul(u32::try_from(attachments).unwrap_or(u32::MAX)),
    )
}

/// A channel's pool and the settings it has now.
#[derive(Clone, Copy, Debug)]
pub struct Pool {
    pub channel: ChannelId,
    pub settings: PoolSettings,
}

#[derive(QueryableByName)]
struct PoolRow {
    #[diesel(sql_type = aspen_schema::sql_types::ChannelType)]
    ty: ChannelType,
    #[diesel(sql_type = Nullable<Integer>)]
    recharge: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    max: Option<i32>,
    #[diesel(sql_type = Nullable<aspen_schema::sql_types::ChannelType>)]
    parent_ty: Option<ChannelType>,
    #[diesel(sql_type = Nullable<Integer>)]
    parent_recharge: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    parent_max: Option<i32>,
}

/// The pool of `channel`, if it has one: a community text channel's, a thread's (with its
/// parent's settings; none in a one-to-one DM), and a group DM's (with the deployment's).
pub async fn pool_of(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
) -> crate::Result<Option<Pool>> {
    let row: Option<PoolRow> = diesel::sql_query(
        r#"
        SELECT c.ty, c.mana_recharge AS recharge, c.mana_max AS max, p.ty AS parent_ty,
               p.mana_recharge AS parent_recharge, p.mana_max AS parent_max
        FROM channel c LEFT JOIN channel p ON p.id = c.parent_channel
        WHERE c.id = $1 AND c.deleted_at IS NULL
        "#,
    )
    .bind::<PgUuid, _>(channel)
    .get_result(conn)
    .await
    .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let deployment = PoolSettings::deployment(&state.settings());
    let settings = match (row.ty, row.parent_ty) {
        (ChannelType::Text, _) => PoolSettings::of(deployment, row.recharge, row.max),
        (ChannelType::Thread, Some(ChannelType::Text)) => {
            PoolSettings::of(deployment, row.parent_recharge, row.parent_max)
        }
        (ChannelType::GroupDm, _) | (ChannelType::Thread, Some(ChannelType::GroupDm)) => {
            PoolSettings::of(deployment, None, None)
        }
        _ => return Ok(None),
    };
    Ok(Some(Pool { channel, settings }))
}

/// A pool's level as of its latest tick, for viewers (`app::mana::live`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    /// In thousandths of mana; below zero in debt.
    pub level: i64,
    /// The time of the tick it is as of, in Unix milliseconds.
    pub at_ms: i64,
}

/// One pool's tick and spend, atomically on Valkey's clock. `KEYS[1]` is the pool; `ARGV` are
/// thousandths of mana earned a tick, its capacity in thousandths, the cost in thousandths, and
/// how to spend: `peek` spends nothing, `pay` spends when the pool holds the cost or is full
/// (into debt then), `bypass` always spends. Nothing goes below half the capacity in debt. A pool
/// is a hash of `level` and `tick` (the tick it is as of, 200 ms each since 1970); one Valkey
/// does not hold is full, and one that fills again is forgotten, so a quiet channel keeps no
/// key. What is spent is added to its sender, `ARGV[5]` (empty for none), in whole mana, in
/// `KEYS[2]`, this minute's [`sent_key`], kept `ARGV[6]` milliseconds ([`top_senders`]).
/// Answers whether it spent, the level, and the tick's time in milliseconds.
const POOL: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local tick = math.floor(now / 200)
local per = tonumber(ARGV[1])
local max = tonumber(ARGV[2])
local cost = tonumber(ARGV[3])
local mode = ARGV[4]
local floor = -math.floor(max / 2)
local state = redis.call('HMGET', KEYS[1], 'level', 'tick')
local level = tonumber(state[1])
local last = tonumber(state[2])
if level == nil or last == nil then
  level = max
  last = tick
end
if tick > last then
  level = level + (tick - last) * per
end
if level > max then
  level = max
end
local spent = 0
if mode == 'bypass' or (mode == 'pay' and (level >= cost or level == max)) then
  level = math.max(level - cost, floor)
  spent = 1
  if cost > 0 and ARGV[5] ~= '' then
    redis.call('ZINCRBY', KEYS[2], math.floor(cost / 1000), ARGV[5])
    redis.call('PEXPIRE', KEYS[2], tonumber(ARGV[6]))
  end
end
if level >= max then
  redis.call('DEL', KEYS[1])
else
  redis.call('HSET', KEYS[1], 'level', level, 'tick', tick)
  local ticks = math.ceil((max - level) / per)
  redis.call('PEXPIRE', KEYS[1], ticks * 200 + 1000)
end
return {spent, level, tick * 200}
";

#[derive(Clone, Copy)]
enum Mode {
    Peek,
    Pay,
    Bypass,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Peek => "peek",
            Mode::Pay => "pay",
            Mode::Bypass => "bypass",
        }
    }
}

fn key(channel: ChannelId) -> String {
    format!("mana:{}", channel.0)
}

/// How far back [`top_senders`] reads: the past hour, a minute at a time.
pub const SENT_MINUTES: i64 = 60;
/// How many senders [`top_senders`] answers.
pub const TOP_SENDERS: usize = 100;
/// How long one reading of a pool's top senders serves every viewer asking.
const TOP_SENDERS_CACHED: Duration = Duration::from_secs(5);

/// The set of what each sender spent from `channel`'s pool in `minute` (minutes since 1970).
fn sent_key(channel: ChannelId, minute: i64) -> String {
    format!("mana:{}:sent:{minute}", channel.0)
}

fn this_minute() -> i64 {
    chrono::Utc::now().timestamp().div_euclid(60)
}

/// Sums a pool's last [`SENT_MINUTES`] of spending by sender into `KEYS[1]`, kept `ARGV[2]`
/// milliseconds so viewers asking meanwhile share it, and answers its first `ARGV[1]`, the most
/// first, each followed by their mana. The sum's work grows with how many sent in the hour,
/// once each [`TOP_SENDERS_CACHED`] however many ask.
const TOP: &str = r"
if redis.call('EXISTS', KEYS[1]) == 0 then
  local minutes = {}
  for i = 2, #KEYS do
    minutes[#minutes + 1] = KEYS[i]
  end
  if redis.call('ZUNIONSTORE', KEYS[1], #minutes, unpack(minutes)) > 0 then
    redis.call('PEXPIRE', KEYS[1], tonumber(ARGV[2]))
  end
end
return redis.call('ZREVRANGE', KEYS[1], 0, tonumber(ARGV[1]) - 1, 'WITHSCORES')
";

/// Who spent most of `channel`'s mana in the past hour, as [`top_senders`] says, for `viewer`,
/// who must be able to read the channel (`channel_access`); none where it has no pool.
pub async fn read_top_senders(
    state: &GlobalServerContext,
    viewer: crate::UserId,
    channel: ChannelId,
) -> crate::Result<Vec<(crate::UserId, u64)>> {
    let pool = {
        let mut conn = state.connection_pool.get().await?;
        crate::permissions::channel_access(state, conn.as_mut(), viewer, channel).await?;
        pool_of(state, conn.as_mut(), channel).await?
    };
    Ok(match pool {
        Some(pool) => top_senders(state, &pool).await,
        None => Vec::new(),
    })
}

/// Who spent most of `pool`'s mana in the past hour, the most first, at most [`TOP_SENDERS`],
/// each with what they spent in whole mana: posting at once, with Bypass, or paid for by the
/// drainer, never what was refused or dropped. Read fresh at most each
/// [`TOP_SENDERS_CACHED`]. Empty when Valkey cannot be read, as mana that cannot be counted is
/// none.
pub async fn top_senders(state: &GlobalServerContext, pool: &Pool) -> Vec<(crate::UserId, u64)> {
    let minute = this_minute();
    let mut keys = vec![format!("mana:{}:top", pool.channel.0)];
    keys.extend((0..SENT_MINUTES).map(|ago| sent_key(pool.channel, minute - ago)));
    let read: Result<Vec<String>, fred::error::Error> = state
        .valkey
        .eval(
            TOP,
            keys,
            vec![
                TOP_SENDERS.to_string(),
                TOP_SENDERS_CACHED.as_millis().to_string(),
            ],
        )
        .await;
    match read {
        Ok(read) => read
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|[user, mana]| {
                let user = user.parse::<uuid::Uuid>().ok()?;
                let mana = mana.parse::<f64>().ok()?;
                Some((crate::UserId(user), mana.max(0.0) as u64))
            })
            .collect(),
        Err(error) => {
            unavailable(&error);
            Vec::new()
        }
    }
}

async fn run(
    state: &GlobalServerContext,
    pool: &Pool,
    cost: u32,
    mode: Mode,
    sender: Option<crate::UserId>,
) -> Result<(bool, Level), fred::error::Error> {
    let per_tick = i64::from(pool.settings.recharge) * MILLI * 200 / 1000;
    let (spent, level, at_ms): (i64, i64, i64) = state
        .valkey
        .eval(
            POOL,
            vec![key(pool.channel), sent_key(pool.channel, this_minute())],
            vec![
                per_tick.to_string(),
                (i64::from(pool.settings.max) * MILLI).to_string(),
                (i64::from(cost) * MILLI).to_string(),
                mode.name().to_string(),
                sender.map_or(String::new(), |sender| sender.0.to_string()),
                // A minute's set outlives the hour it is read in by a minute.
                ((SENT_MINUTES + 1) * 60_000).to_string(),
            ],
        )
        .await?;
    Ok((spent == 1, Level { level, at_ms }))
}

/// The pool's level now, spending nothing; `None` when it cannot be counted.
pub async fn level(state: &GlobalServerContext, pool: &Pool) -> Option<Level> {
    match run(state, pool, 0, Mode::Peek, None).await {
        Ok((_, level)) => Some(level),
        Err(error) => {
            unavailable(&error);
            None
        }
    }
}

/// Whether a message may be posted now, or must wait its turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Go,
    Wait,
}

/// Decides a message costing `cost` sent to `pool`, spending for it when it goes ahead. Someone
/// holding Bypass channel mana there (`bypass`) goes ahead at once and spends, down to the
/// floor; anyone else waits while others wait in the pool, and otherwise goes ahead when the
/// pool holds the cost, or is full. Mana that cannot be counted lets everything through.
pub async fn decide(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    pool: &Pool,
    sender: crate::UserId,
    bypass: bool,
    cost: u32,
) -> crate::Result<Decision> {
    if cost == 0 {
        return Ok(Decision::Go);
    }
    if bypass {
        spend(state, pool, cost, Mode::Bypass, sender).await;
        return Ok(Decision::Go);
    }
    if queue::waiting_in(conn, pool.channel).await? {
        return Ok(Decision::Wait);
    }
    Ok(match spend(state, pool, cost, Mode::Pay, sender).await {
        Some(false) => Decision::Wait,
        Some(true) | None => Decision::Go,
    })
}

/// Decides `author`'s sending of something costing `cost` to `channel`: through its pool when it
/// has one, with Bypass channel mana as the author holds it there (a thread's from its parent's).
/// The caller has checked that the author may post there, so that only those who could post
/// spend a channel's mana.
pub async fn decide_for(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    author: crate::UserId,
    channel: ChannelId,
    cost: u32,
) -> crate::Result<Decision> {
    let Some(pool) = pool_of(state, conn, channel).await? else {
        return Ok(Decision::Go);
    };
    let bypass = crate::permissions::channel_access(state, conn, author, channel)
        .await?
        .has(crate::permissions::Permissions::BYPASS_CHANNEL_MANA);
    decide(state, conn, &pool, author, bypass, cost).await
}

/// Pays for a waiting message as the pool's drainer posts it: whether the pool held the cost,
/// or was full. Mana that cannot be counted pays.
pub async fn pay_waiting(
    state: &GlobalServerContext,
    pool: &Pool,
    sender: crate::UserId,
    cost: u32,
) -> bool {
    spend(state, pool, cost, Mode::Pay, sender)
        .await
        .unwrap_or(true)
}

/// Spends, telling viewers of the new level; `None` when Valkey cannot count it.
async fn spend(
    state: &GlobalServerContext,
    pool: &Pool,
    cost: u32,
    mode: Mode,
    sender: crate::UserId,
) -> Option<bool> {
    match run(state, pool, cost, mode, Some(sender)).await {
        Ok((spent, level)) => {
            if spent {
                live::changed(state, pool, level).await;
            }
            Some(spent)
        }
        Err(error) => {
            unavailable(&error);
            None
        }
    }
}

/// Unix milliseconds of the last "mana uncounted" log line, so an outage logs twice a minute
/// rather than for every message.
static LAST_LOG_MS: AtomicU64 = AtomicU64::new(0);

fn unavailable(error: &fred::error::Error) {
    let now = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0);
    let last = AtomicU64::load(&LAST_LOG_MS, Ordering::Relaxed);
    if now.saturating_sub(last) >= 30_000
        && LAST_LOG_MS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        tracing::error!(%error, "channel mana cannot be counted, so messages post as if every pool were full");
    }
}

pub mod live;

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0190f0a0-0000-7000-8000-000000000001";

    #[test]
    fn a_glyph_is_what_a_reader_sees_as_one_character() {
        assert_eq!(glyphs(""), 0);
        assert_eq!(glyphs("hello"), 5);
        // A precomposed and a decomposed é, a CJK character, an emoji with a skin tone, a
        // family, and a flag are one each.
        assert_eq!(glyphs("é"), 1);
        assert_eq!(glyphs("e\u{0301}"), 1);
        assert_eq!(glyphs("中文"), 2);
        assert_eq!(glyphs("👍🏽"), 1);
        assert_eq!(glyphs("👨‍👩‍👧"), 1);
        assert_eq!(glyphs("🇯🇵"), 1);
        // A line break of CR and LF is one.
        assert_eq!(glyphs("a\r\nb"), 3);
    }

    #[test]
    fn a_tag_an_emoji_and_everyone_are_one_glyph_each() {
        assert_eq!(glyphs(&format!("hi <@{ID}>")), 4);
        assert_eq!(glyphs(&format!("<@&{ID}>")), 1);
        assert_eq!(glyphs(&format!("<:{ID}><:{ID}>")), 2);
        assert_eq!(glyphs("@everyone look"), 6);
        // Text shaped like a token but not one counts as it is.
        assert_eq!(glyphs("<@nobody>"), 9);
        assert_eq!(glyphs("<:short>"), 8);
        assert_eq!(glyphs("<"), 1);
    }

    #[test]
    fn attachments_cost_what_the_deployment_says_and_edits_only_what_they_add() {
        assert_eq!(cost(30, "hi", 2), 62);
        assert_eq!(cost(0, "hi", 2), 2);
        let added = |before: &str, after: &str, files| {
            cost(30, "", files) + glyphs(after).saturating_sub(glyphs(before))
        };
        assert_eq!(added("hello", "hello world", 0), 6);
        assert_eq!(added("hello world", "hi", 0), 0);
        assert_eq!(added("hello", "jello", 1), 30);
    }

    #[test]
    fn the_deployment_s_settings_are_the_default_and_the_ceiling() {
        let deployment = PoolSettings {
            recharge: 50,
            max: 20_000,
        };
        assert_eq!(PoolSettings::of(deployment, None, None), deployment);
        assert_eq!(
            PoolSettings::of(deployment, Some(10), Some(500)),
            PoolSettings {
                recharge: 10,
                max: 500
            }
        );
        // Set before the deployment's were lowered: held to them.
        assert_eq!(
            PoolSettings::of(deployment, Some(80), Some(30_000)),
            deployment
        );
        assert!(check_channel_settings(deployment, Some(50), Some(20_000)).is_ok());
        assert!(check_channel_settings(deployment, None, None).is_ok());
        assert!(check_channel_settings(deployment, Some(51), None).is_err());
        assert!(check_channel_settings(deployment, None, Some(20_001)).is_err());
        assert!(check_channel_settings(deployment, Some(0), None).is_err());
        assert!(check_channel_settings(deployment, None, Some(0)).is_err());
    }
}
