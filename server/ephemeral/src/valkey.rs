//! [`Ephemeral::Valkey`](crate::Ephemeral::Valkey): the store as Valkey, each operation that reads
//! and writes several keys at once a Lua script run with `EVAL`, which Valkey runs atomically.

use std::time::Duration;

use fred::clients::Client;
use fred::interfaces::{
    ClientLike, KeysInterface, LuaInterface, SetsInterface, SortedSetsInterface,
};
use fred::types::{Expiration, FromValue, SetOptions};

use crate::{
    Command, Condition, Error, Expiry, GcraRate, ManaAllot, ManaAllotted, ManaKeys, ManaRates,
    ManaSpend, ManaSpent, ManaTurn, Reply, Result, Stored, Ttl, Value,
};

impl From<fred::error::Error> for Error {
    fn from(error: fred::error::Error) -> Self {
        Error::Failed(Box::new(error))
    }
}

impl From<Value> for fred::types::Value {
    fn from(value: Value) -> Self {
        match value {
            Value::Text(text) => text.into(),
            Value::Integer(integer) => integer.into(),
        }
    }
}

/// Valkey's `TTL` and `PTTL` answer -2 for a key not set and -1 for one with no expiry.
fn ttl_of(answer: i64, unit: fn(u64) -> Duration) -> Ttl {
    match u64::try_from(answer) {
        Ok(left) => Ttl::Left(unit(left)),
        Err(_) if answer == -1 => Ttl::NoExpiry,
        Err(_) => Ttl::NoKey,
    }
}

/// A `Ttl` read through a pipeline is a `PTTL`'s.
impl FromValue for Ttl {
    fn from_value(value: fred::types::Value) -> std::result::Result<Self, fred::error::Error> {
        Ok(ttl_of(i64::from_value(value)?, Duration::from_millis))
    }
}

fn expiration(expiry: Expiry) -> Expiration {
    match expiry {
        Expiry::Seconds(seconds) => Expiration::EX(seconds),
        Expiry::Millis(millis) => Expiration::PX(millis),
        Expiry::AtMillis(at) => Expiration::PXAT(at),
    }
}

fn set_options(condition: Condition) -> SetOptions {
    match condition {
        Condition::Absent => SetOptions::NX,
        Condition::Present => SetOptions::XX,
    }
}

pub(crate) async fn connect(url: &str) -> Result<Client> {
    let client = Client::new(fred::prelude::Config::from_url(url)?, None, None, None);
    client.init().await?;
    Ok(client)
}

pub(crate) async fn ping(client: &Client) -> Result<()> {
    let _: String = client.ping(None).await?;
    Ok(())
}

pub(crate) async fn get<V: Stored>(client: &Client, key: &str) -> Result<Option<V>> {
    Ok(client.get(key).await?)
}

pub(crate) async fn getdel<V: Stored>(client: &Client, key: &str) -> Result<Option<V>> {
    Ok(client.getdel(key).await?)
}

pub(crate) async fn set(
    client: &Client,
    key: &str,
    value: Value,
    expiry: Option<Expiry>,
    condition: Option<Condition>,
) -> Result<bool> {
    // `OK` when written, nil when the condition kept it from being.
    let written: Option<String> = client
        .set(
            key,
            fred::types::Value::from(value),
            expiry.map(expiration),
            condition.map(set_options),
            false,
        )
        .await?;
    Ok(written.is_some())
}

pub(crate) async fn set_previous<V: Stored>(
    client: &Client,
    key: &str,
    value: Value,
    expiry: Option<Expiry>,
    condition: Option<Condition>,
) -> Result<Option<V>> {
    Ok(client
        .set(
            key,
            fred::types::Value::from(value),
            expiry.map(expiration),
            condition.map(set_options),
            true,
        )
        .await?)
}

pub(crate) async fn del(client: &Client, keys: Vec<String>) -> Result<i64> {
    Ok(client.del(keys).await?)
}

pub(crate) async fn exists(client: &Client, key: &str) -> Result<bool> {
    let found: i64 = client.exists(key).await?;
    Ok(found > 0)
}

pub(crate) async fn incr<V: Stored>(client: &Client, key: &str) -> Result<V> {
    Ok(client.incr(key).await?)
}

pub(crate) async fn incr_by(client: &Client, key: &str, by: i64) -> Result<i64> {
    Ok(client.incr_by(key, by).await?)
}

pub(crate) async fn expire(client: &Client, key: &str, seconds: i64) -> Result<bool> {
    Ok(client.expire(key, seconds, None).await?)
}

pub(crate) async fn ttl(client: &Client, key: &str) -> Result<Ttl> {
    Ok(ttl_of(client.ttl(key).await?, Duration::from_secs))
}

pub(crate) async fn pttl(client: &Client, key: &str) -> Result<Ttl> {
    Ok(ttl_of(client.pttl(key).await?, Duration::from_millis))
}

pub(crate) async fn mget(client: &Client, keys: Vec<String>) -> Result<Vec<Option<String>>> {
    Ok(client.mget(keys).await?)
}

pub(crate) async fn sadd(client: &Client, key: &str, members: Vec<String>) -> Result<i64> {
    Ok(client.sadd(key, members).await?)
}

pub(crate) async fn smembers(client: &Client, key: &str) -> Result<Vec<String>> {
    Ok(client.smembers(key).await?)
}

pub(crate) async fn zrem(client: &Client, key: &str, member: &str) -> Result<i64> {
    Ok(client.zrem(key, member).await?)
}

pub(crate) async fn run(
    client: &Client,
    commands: Vec<Command>,
) -> Result<Vec<Option<fred::types::Value>>> {
    let pipeline = client.pipeline();
    for command in commands {
        let () = match command {
            Command::Set {
                key,
                value,
                expiry,
                condition,
                previous,
            } => {
                pipeline
                    .set(
                        key,
                        fred::types::Value::from(value),
                        expiry.map(expiration),
                        condition.map(set_options),
                        previous,
                    )
                    .await?
            }
            Command::Pttl(key) => pipeline.pttl(key).await?,
            Command::Mget(keys) => pipeline.mget(keys).await?,
            Command::Zadd { key, members } => {
                pipeline
                    .zadd(key, None, None, false, false, members)
                    .await?
            }
            Command::Expire { key, seconds } => pipeline.expire(key, seconds, None).await?,
            // Scores are bounded as floats: fred reads an integer bound as a rank.
            Command::ZremRangeByScore { key, min, max } => {
                pipeline.zremrangebyscore(key, min, max).await?
            }
            Command::ZrangeByScore { key, min, max } => {
                pipeline.zrangebyscore(key, min, max, false, None).await?
            }
        };
    }
    let answers: Vec<fred::types::Value> = pipeline.all().await?;
    Ok(answers.into_iter().map(Some).collect())
}

pub(crate) fn take<T: Reply>(answers: &mut [Option<fred::types::Value>], at: usize) -> Result<T> {
    let answer = answers.get_mut(at).and_then(Option::take).ok_or_else(|| {
        fred::error::Error::new(
            fred::error::ErrorKind::Parse,
            "no answer for a pipelined command",
        )
    })?;
    Ok(answer.convert()?)
}

/// Checks and updates one GCRA bucket. `KEYS[1]` is the bucket; `ARGV[1]` the emission interval
/// (milliseconds per request) and `ARGV[2]` the burst tolerance (milliseconds of credit beyond
/// one request). Returns 0 when the request is allowed, otherwise the milliseconds until it
/// would be.
const GCRA: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local emission = tonumber(ARGV[1])
local tolerance = tonumber(ARGV[2])
local tat = tonumber(redis.call('GET', KEYS[1]) or now)
if tat < now then
  tat = now
end
local allow_at = tat - tolerance
if now < allow_at then
  return allow_at - now
end
local next_tat = tat + emission
redis.call('SET', KEYS[1], next_tat, 'PX', next_tat - now)
return 0
";

/// As [`GCRA`], but only answers whether a request would be allowed, counting nothing.
const GCRA_PEEK: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local tolerance = tonumber(ARGV[2])
local tat = tonumber(redis.call('GET', KEYS[1]) or now)
if tat < now then
  tat = now
end
local allow_at = tat - tolerance
if now < allow_at then
  return allow_at - now
end
return 0
";

/// Gives back one request [`GCRA`] counted, as if it had not been made: `KEYS[1]` is the bucket
/// and `ARGV[1]` the emission interval. Requests counted since are kept, since each moved the
/// bucket by its own interval.
const GCRA_REFUND: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local tat = tonumber(redis.call('GET', KEYS[1]))
if not tat then
  return 0
end
local next_tat = tat - tonumber(ARGV[1])
if next_tat <= now then
  redis.call('DEL', KEYS[1])
else
  redis.call('SET', KEYS[1], next_tat, 'PX', next_tat - now)
end
return 0
";

async fn gcra(
    client: &Client,
    script: &'static str,
    key: &str,
    rate: GcraRate,
) -> Result<Option<Duration>> {
    let wait: i64 = client
        .eval(
            script,
            key.to_string(),
            vec![rate.emission_ms, rate.tolerance_ms],
        )
        .await?;
    Ok((wait > 0).then(|| Duration::from_millis(wait.unsigned_abs())))
}

pub(crate) async fn gcra_take(
    client: &Client,
    key: &str,
    rate: GcraRate,
) -> Result<Option<Duration>> {
    gcra(client, GCRA, key, rate).await
}

pub(crate) async fn gcra_peek(
    client: &Client,
    key: &str,
    rate: GcraRate,
) -> Result<Option<Duration>> {
    gcra(client, GCRA_PEEK, key, rate).await
}

pub(crate) async fn gcra_refund(client: &Client, key: &str, rate: GcraRate) -> Result<()> {
    gcra(client, GCRA_REFUND, key, rate).await?;
    Ok(())
}

/// Counts one attempt and answers how many the window holds now, starting the window with the
/// first. One script, so the count and its expiry are set together.
const COUNT_ATTEMPT: &str = "
local count = redis.call('INCR', KEYS[1])
if count == 1 then
  redis.call('EXPIRE', KEYS[1], ARGV[1])
end
return count
";

pub(crate) async fn count_attempt(client: &Client, key: &str, window_seconds: i64) -> Result<i64> {
    Ok(client
        .eval(COUNT_ATTEMPT, key.to_string(), window_seconds)
        .await?)
}

/// Marks `ARGV[2]`, a connection of the user whose set is `KEYS[1]`, attended for `ARGV[1]`
/// milliseconds, forgetting members whose time has passed. The set lives as long as its newest
/// member, which this one is.
const ATTENTION_MARK: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local lasting = tonumber(ARGV[1])
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now)
redis.call('ZADD', KEYS[1], now + lasting, ARGV[2])
redis.call('PEXPIRE', KEYS[1], lasting)
return 1
";

/// For each set of `KEYS`, whether any member's time has not yet passed: 1 or 0.
const ATTENTION_ASK: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local found = {}
for i, key in ipairs(KEYS) do
  found[i] = math.min(redis.call('ZCOUNT', key, '(' .. now, '+inf'), 1)
end
return found
";

pub(crate) async fn attention_mark(
    client: &Client,
    key: &str,
    member: &str,
    lasting: Duration,
) -> Result<()> {
    let _: i64 = client
        .eval(
            ATTENTION_MARK,
            vec![key.to_string()],
            vec![lasting.as_millis().to_string(), member.to_string()],
        )
        .await?;
    Ok(())
}

pub(crate) async fn attention_ask(client: &Client, keys: Vec<String>) -> Result<Vec<bool>> {
    let asked = keys.len();
    let found: Vec<i64> = client
        .eval(ATTENTION_ASK, keys, Vec::<String>::new())
        .await?;
    if found.len() != asked {
        return Err(fred::error::Error::new(
            fred::error::ErrorKind::Unknown,
            "Valkey answered whether users are attended for a different number of them",
        )
        .into());
    }
    Ok(found.into_iter().map(|n| n > 0).collect())
}

/// Adds one to a kept badge, answering it, or nothing when none is kept.
const BADGE_ADD: &str = r#"
if redis.call('EXISTS', KEYS[1]) == 1 then
    return redis.call('INCR', KEYS[1])
end
return false
"#;

pub(crate) async fn badge_add(client: &Client, key: &str) -> Result<Option<i64>> {
    Ok(client
        .eval(BADGE_ADD, key.to_string(), Vec::<String>::new())
        .await?)
}

/// One pool's tick and spend, atomically on Valkey's clock. `KEYS[1]` is the pool; `ARGV` are
/// thousandths of mana earned a tick, its capacity in thousandths, the cost in thousandths, and
/// how to spend: `peek` spends nothing, `pay` spends when the pool holds the cost or is full
/// (into debt then), `bypass` always spends. Nothing goes below half the capacity in debt. A pool
/// is a hash of `level` and `tick` (the tick it is as of, 200 ms each since 1970); one Valkey
/// does not hold is full, and one that fills again is forgotten, so a quiet channel keeps no
/// key. `KEYS[3]` is the pool's credit, what its queue's first authors have been credited
/// ([`MANA_SHARE`]): it is the pool's mana too, so the level never rises above the capacity less
/// it, and the level answered includes it. What is spent is added to its sender, `ARGV[5]` (empty
/// for none), in whole mana, in `KEYS[2]`, this minute's spending, kept `ARGV[6]` milliseconds.
/// Answers whether it spent, the level, and the tick's time in milliseconds.
const MANA_POOL: &str = r"
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
local held = 0
for _, credit in ipairs(redis.call('HVALS', KEYS[3])) do
  held = held + tonumber(credit)
end
if level > max - held then
  level = max - held
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
return {spent, level + held, tick * 200}
";

pub(crate) async fn mana_pool(
    client: &Client,
    keys: &ManaKeys,
    rates: ManaRates,
    cost: i64,
    spend: ManaSpend,
    sender: Option<&str>,
    sent_kept_ms: i64,
) -> Result<ManaSpent> {
    let mode = match spend {
        ManaSpend::Peek => "peek",
        ManaSpend::Pay => "pay",
        ManaSpend::Bypass => "bypass",
    };
    let (spent, level, at_ms): (i64, i64, i64) = client
        .eval(
            MANA_POOL,
            vec![keys.pool.clone(), keys.sent.clone(), keys.credit.clone()],
            vec![
                rates.per_tick.to_string(),
                rates.max.to_string(),
                cost.to_string(),
                mode.to_string(),
                sender.unwrap_or_default().to_string(),
                sent_kept_ms.to_string(),
            ],
        )
        .await?;
    Ok(ManaSpent {
        spent: spent == 1,
        level,
        at_ms,
    })
}

/// A pool's queue's allocation, atomically on Valkey's clock, beside [`MANA_POOL`]'s pool
/// (`KEYS[1]`), its credit (`KEYS[2]`), and this minute's spending (`KEYS[3]`). `ARGV` are
/// thousandths earned a tick, the capacity in thousandths, the mode, how long a minute's spending
/// is kept, and then, for each author given, their id, their oldest row's cost in thousandths, and
/// their weight, in thirds.
///
/// `share` is given the queue's first authors, at most three, in order: credit held by anyone
/// else goes back to the pool, and whatever the pool then holds above zero is split into three
/// portions, each author credited their weight's (with the rounding's remainder to the first).
/// It answers, for each, whether their row can be paid: their credit covers it, or, while the
/// pool is not in debt, their credit has reached their weight's share of the capacity, as a full
/// pool pays anything. `pay` is given one author and pays their row so: from their credit, or
/// with all of it and the rest from the pool, into debt down to half the capacity. Credit is
/// kept a minute past the last call, which a running drainer makes five times a second. Answers
/// the level (with the credit held) and the tick's time, then whether each author can be paid
/// (`share`) or whether the one was (`pay`).
const MANA_SHARE: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local tick = math.floor(now / 200)
local per = tonumber(ARGV[1])
local max = tonumber(ARGV[2])
local mode = ARGV[3]
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
local credit = {}
local flat = redis.call('HGETALL', KEYS[2])
for i = 1, #flat, 2 do
  credit[flat[i]] = tonumber(flat[i + 1])
end
local authors, costs, weights = {}, {}, {}
for i = 5, #ARGV, 3 do
  authors[#authors + 1] = ARGV[i]
  costs[#costs + 1] = tonumber(ARGV[i + 1])
  weights[#weights + 1] = tonumber(ARGV[i + 2])
end
local function sum()
  local held = 0
  for _, c in pairs(credit) do
    held = held + c
  end
  return held
end
local function payable(i)
  local c = credit[authors[i]] or 0
  return c >= costs[i] or (level >= 0 and c >= math.floor(weights[i] * max / 3))
end
local answer = {}
if mode == 'share' then
  local top = {}
  for _, author in ipairs(authors) do
    top[author] = true
  end
  for author, c in pairs(credit) do
    if not top[author] then
      level = level + c
      credit[author] = nil
    end
  end
  local held = sum()
  if level > max - held then
    level = max - held
  end
  if level > 0 and #authors > 0 then
    local portion = math.floor(level / 3)
    for i, author in ipairs(authors) do
      local add = weights[i] * portion
      if i == 1 then
        add = add + level - 3 * portion
      end
      credit[author] = (credit[author] or 0) + add
    end
    level = 0
  end
  for i = 1, #authors do
    answer[i] = payable(i) and 1 or 0
  end
else
  local held = sum()
  if level > max - held then
    level = max - held
  end
  local author, cost = authors[1], costs[1]
  local c = credit[author] or 0
  local spent = 0
  if c >= cost then
    credit[author] = c - cost
    spent = 1
  elseif payable(1) then
    level = math.max(level - (cost - c), floor)
    credit[author] = 0
    spent = 1
  end
  if credit[author] == 0 then
    credit[author] = nil
  end
  if spent == 1 and cost > 0 then
    redis.call('ZINCRBY', KEYS[3], math.floor(cost / 1000), author)
    redis.call('PEXPIRE', KEYS[3], tonumber(ARGV[4]))
  end
  answer[1] = spent
end
redis.call('DEL', KEYS[2])
local held = 0
for author, c in pairs(credit) do
  if c > 0 then
    redis.call('HSET', KEYS[2], author, c)
    held = held + c
  end
end
if held > 0 then
  redis.call('PEXPIRE', KEYS[2], 60000)
end
if level >= max then
  redis.call('DEL', KEYS[1])
else
  redis.call('HSET', KEYS[1], 'level', level, 'tick', tick)
  local ticks = math.ceil((max - level) / per)
  redis.call('PEXPIRE', KEYS[1], ticks * 200 + 1000)
end
local out = {level + held, tick * 200}
for i = 1, #answer do
  out[#out + 1] = answer[i]
end
return out
";

pub(crate) async fn mana_allot(
    client: &Client,
    keys: &ManaKeys,
    rates: ManaRates,
    allot: ManaAllot,
    sent_kept_ms: i64,
    turns: &[ManaTurn],
) -> Result<ManaAllotted> {
    let mode = match allot {
        ManaAllot::Share => "share",
        ManaAllot::Pay => "pay",
    };
    let mut args = vec![
        rates.per_tick.to_string(),
        rates.max.to_string(),
        mode.to_string(),
        sent_kept_ms.to_string(),
    ];
    for turn in turns {
        args.push(turn.author.clone());
        args.push(turn.cost.to_string());
        args.push(turn.weight.to_string());
    }
    let answer: Vec<i64> = client
        .eval(
            MANA_SHARE,
            vec![keys.pool.clone(), keys.credit.clone(), keys.sent.clone()],
            args,
        )
        .await?;
    let [level, at_ms, answers @ ..] = &answer[..] else {
        return Err(fred::error::Error::new(
            fred::error::ErrorKind::Parse,
            "Valkey answered a mana allocation without its level",
        )
        .into());
    };
    Ok(ManaAllotted {
        level: *level,
        at_ms: *at_ms,
        answers: answers.iter().map(|answer| *answer == 1).collect(),
    })
}

/// Sums a pool's minutes of spending by sender (`KEYS[2..]`) into `KEYS[1]`, kept `ARGV[2]`
/// milliseconds so viewers asking meanwhile share it, and answers its first `ARGV[1]`, the most
/// first, each followed by their mana. The sum's work grows with how many sent in those minutes,
/// once each time it is made however many ask.
const MANA_TOP: &str = r"
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

pub(crate) async fn mana_top(
    client: &Client,
    cache: &str,
    minutes: Vec<String>,
    count: usize,
    cached_for: Duration,
) -> Result<Vec<(String, f64)>> {
    let mut keys = vec![cache.to_string()];
    keys.extend(minutes);
    let read: Vec<String> = client
        .eval(
            MANA_TOP,
            keys,
            vec![count.to_string(), cached_for.as_millis().to_string()],
        )
        .await?;
    // A score Valkey writes is always a number.
    Ok(read
        .as_chunks::<2>()
        .0
        .iter()
        .filter_map(|[member, score]| Some((member.clone(), score.parse::<f64>().ok()?)))
        .collect())
}
