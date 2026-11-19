//! The short-lived state every API server shares: keys that expire, counters, sets, sorted sets,
//! and the few operations that read and write several of them at once (rate limit buckets,
//! channel mana, attention, attempt counts, badges), each a method of its own. Each server reaches
//! it through an [`Ephemeral`], whose types are Aspen's own, so what a server keeps there does not
//! depend on what keeps it. [`Ephemeral::Valkey`] is Valkey (or Redis).
//!
//! Everything here may be lost: a key is gone once it expires, and all of them when the store is
//! restarted without persistence. What reads it treats a missing key as the state it starts from.

mod valkey;

use std::marker::PhantomData;
use std::time::Duration;

/// A connection to the store. Cheap to clone; every clone shares the connection.
#[derive(Clone)]
pub enum Ephemeral {
    Valkey(fred::clients::Client),
}

/// What went wrong reaching the store: it could not be reached or did not answer in time, it
/// refused the command, or it answered with what the command cannot answer.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Failed(BoxError),
}

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

/// When a key written with [`Ephemeral::set`] expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    /// This many seconds from now (`EX`).
    Seconds(i64),
    /// This many milliseconds from now (`PX`).
    Millis(i64),
    /// At this Unix time in milliseconds (`PXAT`).
    AtMillis(i64),
}

/// What must hold for [`Ephemeral::set`] to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// The key is not set (`NX`).
    Absent,
    /// The key is set (`XX`).
    Present,
}

/// How long a key has left ([`Ephemeral::ttl`], [`Ephemeral::pttl`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ttl {
    /// The key is not set.
    NoKey,
    /// The key is set and never expires.
    NoExpiry,
    /// The key expires this long from now: whole seconds from `ttl`, milliseconds from `pttl`.
    Left(Duration),
}

/// A value written to a key. Integers are kept as their decimal text, so one written as either
/// reads back as either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Integer(i64),
}

impl From<String> for Value {
    fn from(text: String) -> Self {
        Value::Text(text)
    }
}

impl From<&String> for Value {
    fn from(text: &String) -> Self {
        Value::Text(text.clone())
    }
}

impl From<&str> for Value {
    fn from(text: &str) -> Self {
        Value::Text(text.to_string())
    }
}

impl From<i64> for Value {
    fn from(integer: i64) -> Self {
        Value::Integer(integer)
    }
}

impl From<i32> for Value {
    fn from(integer: i32) -> Self {
        Value::Integer(integer.into())
    }
}

/// One key or several, for commands that take either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keys(Vec<String>);

impl From<String> for Keys {
    fn from(key: String) -> Self {
        Keys(vec![key])
    }
}

impl From<&String> for Keys {
    fn from(key: &String) -> Self {
        Keys(vec![key.clone()])
    }
}

impl From<&str> for Keys {
    fn from(key: &str) -> Self {
        Keys(vec![key.to_string()])
    }
}

impl From<Vec<String>> for Keys {
    fn from(keys: Vec<String>) -> Self {
        Keys(keys)
    }
}

mod sealed {
    /// What the store's answers convert into; implemented only here.
    pub trait Sealed: fred::types::FromValue {}
}

/// What a key's value reads as: its text, or the integer it holds (an error when it holds none).
pub trait Stored: sealed::Sealed {}

impl sealed::Sealed for String {}
impl Stored for String {}
impl sealed::Sealed for i64 {}
impl Stored for i64 {}
impl sealed::Sealed for u64 {}
impl Stored for u64 {}

/// What a [`Pipeline`] command answers.
pub trait Reply: sealed::Sealed {}

impl<V: Stored> sealed::Sealed for Option<V> {}
impl<V: Stored> Reply for Option<V> {}
impl sealed::Sealed for Ttl {}
impl Reply for Ttl {}
impl sealed::Sealed for Vec<String> {}
impl Reply for Vec<String> {}
impl sealed::Sealed for Vec<Option<String>> {}
impl Reply for Vec<Option<String>> {}

/// Commands sent together in one round trip and answered together, not atomically: each is
/// carried out as it would be alone, and others' commands may come between them. Built with its
/// methods and sent with [`Ephemeral::run`]; a command whose answer is wanted hands back a
/// [`Slot`] to take it from the [`Replies`] by.
#[derive(Debug, Default)]
pub struct Pipeline {
    commands: Vec<Command>,
}

#[derive(Debug)]
enum Command {
    Set {
        key: String,
        value: Value,
        expiry: Option<Expiry>,
        condition: Option<Condition>,
        previous: bool,
    },
    Pttl(String),
    Mget(Vec<String>),
    Zadd {
        key: String,
        members: Vec<(f64, String)>,
    },
    Expire {
        key: String,
        seconds: i64,
    },
    ZremRangeByScore {
        key: String,
        min: f64,
        max: f64,
    },
    ZrangeByScore {
        key: String,
        min: f64,
        max: f64,
    },
}

/// Where one command's answer is among a pipeline's [`Replies`].
#[derive(Debug)]
pub struct Slot<T> {
    at: usize,
    answers: PhantomData<fn() -> T>,
}

/// What a [`Pipeline`]'s commands answered, in their order.
pub struct Replies(Vec<Option<fred::types::Value>>);

impl Replies {
    /// The answer at `slot`, converted to what its command answers. Taken once.
    pub fn take<T: Reply>(&mut self, slot: Slot<T>) -> Result<T> {
        valkey::take(&mut self.0, slot.at)
    }
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `command`, answering where its answer will be.
    fn push<T>(&mut self, command: Command) -> Slot<T> {
        self.commands.push(command);
        Slot {
            at: self.commands.len() - 1,
            answers: PhantomData,
        }
    }

    /// As [`Ephemeral::set`], its answer not wanted.
    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: impl Into<Value>,
        expiry: Option<Expiry>,
        condition: Option<Condition>,
    ) {
        self.push::<()>(Command::Set {
            key: key.into(),
            value: value.into(),
            expiry,
            condition,
            previous: false,
        });
    }

    /// As [`Ephemeral::set_previous`].
    pub fn set_previous<V: Stored>(
        &mut self,
        key: impl Into<String>,
        value: impl Into<Value>,
        expiry: Option<Expiry>,
        condition: Option<Condition>,
    ) -> Slot<Option<V>> {
        self.push(Command::Set {
            key: key.into(),
            value: value.into(),
            expiry,
            condition,
            previous: true,
        })
    }

    /// As [`Ephemeral::pttl`].
    pub fn pttl(&mut self, key: impl Into<String>) -> Slot<Ttl> {
        self.push(Command::Pttl(key.into()))
    }

    /// As [`Ephemeral::mget`].
    pub fn mget(&mut self, keys: Vec<String>) -> Slot<Vec<Option<String>>> {
        self.push(Command::Mget(keys))
    }

    /// Adds each of `members` to the sorted set `key` with its score, or gives one already there
    /// its new score.
    pub fn zadd(&mut self, key: impl Into<String>, members: Vec<(f64, String)>) {
        self.push::<()>(Command::Zadd {
            key: key.into(),
            members,
        });
    }

    /// As [`Ephemeral::expire`], its answer not wanted.
    pub fn expire(&mut self, key: impl Into<String>, seconds: i64) {
        self.push::<()>(Command::Expire {
            key: key.into(),
            seconds,
        });
    }

    /// Removes the members of the sorted set `key` scored from `min` to `max`, both included.
    pub fn zremrangebyscore(&mut self, key: impl Into<String>, min: f64, max: f64) {
        self.push::<()>(Command::ZremRangeByScore {
            key: key.into(),
            min,
            max,
        });
    }

    /// The members of the sorted set `key` scored from `min` to `max`, both included, lowest
    /// first.
    pub fn zrangebyscore(
        &mut self,
        key: impl Into<String>,
        min: f64,
        max: f64,
    ) -> Slot<Vec<String>> {
        self.push(Command::ZrangeByScore {
            key: key.into(),
            min,
            max,
        })
    }
}

/// A GCRA bucket's rate: the milliseconds each request takes of it (its emission interval), and
/// the milliseconds of credit beyond one request it may hold (its burst tolerance).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcraRate {
    pub emission_ms: u64,
    pub tolerance_ms: u64,
}

/// The keys of one channel's mana pool ([`Ephemeral::mana_pool`], [`Ephemeral::mana_allot`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManaKeys {
    /// The pool: a hash of `level` and `tick`.
    pub pool: String,
    /// What its queue's first authors have been credited: a hash of author to thousandths.
    pub credit: String,
    /// This minute's spending: a sorted set of sender to whole mana.
    pub sent: String,
}

/// A pool's rates, in thousandths of mana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManaRates {
    /// Earned each tick of 200 ms.
    pub per_tick: i64,
    /// The most the pool holds.
    pub max: i64,
}

/// How [`Ephemeral::mana_pool`] spends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManaSpend {
    /// Spends nothing, answering the level.
    Peek,
    /// Spends when the pool holds the cost or is full, into debt then.
    Pay,
    /// Always spends.
    Bypass,
}

/// What [`Ephemeral::mana_pool`] answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManaSpent {
    pub spent: bool,
    /// In thousandths, what is credited included; below zero in debt.
    pub level: i64,
    /// The time of the tick the level is as of, in Unix milliseconds on the store's clock.
    pub at_ms: i64,
}

/// What [`Ephemeral::mana_allot`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManaAllot {
    Share,
    Pay,
}

/// One author [`Ephemeral::mana_allot`] is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManaTurn {
    pub author: String,
    /// Their oldest row's cost, in thousandths.
    pub cost: i64,
    /// How many thirds of what the pool earns they are credited.
    pub weight: u32,
}

/// What [`Ephemeral::mana_allot`] answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManaAllotted {
    /// In thousandths, what is credited included; below zero in debt.
    pub level: i64,
    /// The time of the tick the level is as of, in Unix milliseconds on the store's clock.
    pub at_ms: i64,
    /// For [`ManaAllot::Share`], whether each author given can be paid for, in their order; for
    /// [`ManaAllot::Pay`], whether the one was.
    pub answers: Vec<bool>,
}

impl Ephemeral {
    /// Connects to the Valkey at `url` with the client's defaults: no time limit on commands and
    /// no reconnecting, for tests and tools. A server builds its client with its own settings and
    /// wraps it in [`Ephemeral::Valkey`].
    pub async fn valkey_at(url: &str) -> Result<Self> {
        valkey::connect(url).await.map(Ephemeral::Valkey)
    }

    /// Whether the store answers.
    pub async fn ping(&self) -> Result<()> {
        match self {
            Ephemeral::Valkey(client) => valkey::ping(client).await,
        }
    }

    pub async fn get<V: Stored>(&self, key: &str) -> Result<Option<V>> {
        match self {
            Ephemeral::Valkey(client) => valkey::get(client, key).await,
        }
    }

    /// Reads `key` and removes it, in one step, so of several taking it at once one gets it.
    pub async fn getdel<V: Stored>(&self, key: &str) -> Result<Option<V>> {
        match self {
            Ephemeral::Valkey(client) => valkey::getdel(client, key).await,
        }
    }

    /// Writes `value` to `key` when `condition` holds, replacing any value and expiry it had,
    /// with no expiry when `expiry` is `None`. Whether it was written.
    pub async fn set(
        &self,
        key: &str,
        value: impl Into<Value>,
        expiry: Option<Expiry>,
        condition: Option<Condition>,
    ) -> Result<bool> {
        match self {
            Ephemeral::Valkey(client) => {
                valkey::set(client, key, value.into(), expiry, condition).await
            }
        }
    }

    /// As [`set`](Self::set), answering the value `key` held before, `None` when it held none,
    /// whether or not `condition` let this write.
    pub async fn set_previous<V: Stored>(
        &self,
        key: &str,
        value: impl Into<Value>,
        expiry: Option<Expiry>,
        condition: Option<Condition>,
    ) -> Result<Option<V>> {
        match self {
            Ephemeral::Valkey(client) => {
                valkey::set_previous(client, key, value.into(), expiry, condition).await
            }
        }
    }

    /// Removes `keys`, answering how many were set. An empty list is an error.
    pub async fn del(&self, keys: impl Into<Keys>) -> Result<i64> {
        match self {
            Ephemeral::Valkey(client) => valkey::del(client, keys.into().0).await,
        }
    }

    pub async fn exists(&self, key: &str) -> Result<bool> {
        match self {
            Ephemeral::Valkey(client) => valkey::exists(client, key).await,
        }
    }

    /// Adds one to the integer at `key`, from zero when it is not set, answering the new value;
    /// an error when it holds no integer. An expiry it has is kept, and one not set gets none.
    pub async fn incr<V: Stored>(&self, key: &str) -> Result<V> {
        match self {
            Ephemeral::Valkey(client) => valkey::incr(client, key).await,
        }
    }

    /// As [`incr`](Self::incr), adding `by`.
    pub async fn incr_by(&self, key: &str, by: i64) -> Result<i64> {
        match self {
            Ephemeral::Valkey(client) => valkey::incr_by(client, key, by).await,
        }
    }

    /// Makes `key` expire `seconds` from now, whether or not it had an expiry. Whether it is set.
    pub async fn expire(&self, key: &str, seconds: i64) -> Result<bool> {
        match self {
            Ephemeral::Valkey(client) => valkey::expire(client, key, seconds).await,
        }
    }

    /// How long `key` has left, in whole seconds.
    pub async fn ttl(&self, key: &str) -> Result<Ttl> {
        match self {
            Ephemeral::Valkey(client) => valkey::ttl(client, key).await,
        }
    }

    /// How long `key` has left, in milliseconds.
    pub async fn pttl(&self, key: &str) -> Result<Ttl> {
        match self {
            Ephemeral::Valkey(client) => valkey::pttl(client, key).await,
        }
    }

    /// The values of `keys`, in their order. An empty list is an error.
    pub async fn mget(&self, keys: Vec<String>) -> Result<Vec<Option<String>>> {
        match self {
            Ephemeral::Valkey(client) => valkey::mget(client, keys).await,
        }
    }

    /// Adds `members` to the set `key`, answering how many were not there already.
    pub async fn sadd(&self, key: &str, members: Vec<String>) -> Result<i64> {
        match self {
            Ephemeral::Valkey(client) => valkey::sadd(client, key, members).await,
        }
    }

    /// The members of the set `key`, in no order; none when it is not set.
    pub async fn smembers(&self, key: &str) -> Result<Vec<String>> {
        match self {
            Ephemeral::Valkey(client) => valkey::smembers(client, key).await,
        }
    }

    /// Removes `member` from the sorted set `key`, answering how many were removed.
    pub async fn zrem(&self, key: &str, member: &str) -> Result<i64> {
        match self {
            Ephemeral::Valkey(client) => valkey::zrem(client, key, member).await,
        }
    }

    /// Sends `pipeline`'s commands in one round trip. An error when any of them fails.
    pub async fn run(&self, pipeline: Pipeline) -> Result<Replies> {
        match self {
            Ephemeral::Valkey(client) => valkey::run(client, pipeline.commands).await.map(Replies),
        }
    }

    /// Takes one request from the GCRA bucket `key` at `rate`: `None` when it is allowed, or how
    /// long until it would be, counting nothing then. In one step, on the store's clock in
    /// milliseconds: the bucket holds its theoretical arrival time, `now` when it is not set or
    /// has passed. A request is allowed once `now` is no earlier than that time less the burst
    /// tolerance; allowed, the time moves on by the emission interval and the bucket expires when
    /// `now` reaches it, so a bucket left alone refills by being gone.
    pub async fn gcra_take(&self, key: &str, rate: GcraRate) -> Result<Option<Duration>> {
        match self {
            Ephemeral::Valkey(client) => valkey::gcra_take(client, key, rate).await,
        }
    }

    /// As [`gcra_take`](Self::gcra_take), only answering whether a request would be allowed,
    /// counting nothing.
    pub async fn gcra_peek(&self, key: &str, rate: GcraRate) -> Result<Option<Duration>> {
        match self {
            Ephemeral::Valkey(client) => valkey::gcra_peek(client, key, rate).await,
        }
    }

    /// Gives back one request [`gcra_take`](Self::gcra_take) counted in `key`, as if it had not
    /// been made, in one step on the store's clock: the arrival time moves back by the emission
    /// interval, and the bucket is removed once that is no later than now, or keeps expiring when
    /// now reaches it. A bucket not set is left so. Requests counted since are kept, since each
    /// moved the bucket by its own interval.
    pub async fn gcra_refund(&self, key: &str, rate: GcraRate) -> Result<()> {
        match self {
            Ephemeral::Valkey(client) => valkey::gcra_refund(client, key, rate).await,
        }
    }

    /// Counts one attempt at the count `key` and answers how many it holds now, in one step: the
    /// first attempt makes it, expiring `window_seconds` later, and later ones add to it without
    /// moving its expiry, so the window starts with the first and a lost connection never leaves
    /// a count that does not expire.
    pub async fn count_attempt(&self, key: &str, window_seconds: i64) -> Result<i64> {
        match self {
            Ephemeral::Valkey(client) => valkey::count_attempt(client, key, window_seconds).await,
        }
    }

    /// Marks `member` of the sorted set `key` attended for `lasting`, in one step on the store's
    /// clock in milliseconds: members whose time is `now` or earlier are removed, `member` is
    /// scored `now + lasting` (whatever it was before), and the set expires `lasting` from now,
    /// living as long as its newest member, which this one is.
    pub async fn attention_mark(&self, key: &str, member: &str, lasting: Duration) -> Result<()> {
        match self {
            Ephemeral::Valkey(client) => valkey::attention_mark(client, key, member, lasting).await,
        }
    }

    /// For each sorted set of `keys`, in their order, whether any member's time (its score, in
    /// Unix milliseconds) is still after `now` on the store's clock. A set not there has none.
    pub async fn attention_ask(&self, keys: Vec<String>) -> Result<Vec<bool>> {
        match self {
            Ephemeral::Valkey(client) => valkey::attention_ask(client, keys).await,
        }
    }

    /// Adds one to the integer at `key` and answers it when the key is set, keeping its expiry;
    /// `None`, writing nothing, when it is not. An error when it holds no integer.
    pub async fn badge_add(&self, key: &str) -> Result<Option<i64>> {
        match self {
            Ephemeral::Valkey(client) => valkey::badge_add(client, key).await,
        }
    }

    /// One mana pool's tick and spend, in one step on the store's clock. The pool's `level` (in
    /// thousandths) is as of its `tick` (200 ms each since 1970); a pool not set, or missing
    /// either, is full as of now. It earns `per_tick` for each tick since, and is held to `max`
    /// less what `credit` holds in all. Then `spend` decides: `Bypass` spends `cost`, and `Pay`
    /// spends it when the level holds it or is `max`; either goes no lower than `-floor(max / 2)`.
    /// When it spent a cost above zero and `sender` is given, `sender` is credited
    /// `floor(cost / 1000)` in `sent`, which then expires `sent_kept_ms` from now. A pool at
    /// `max` or above is removed; otherwise its level and the tick are written, and it expires
    /// once it would be full again (`ceil((max - level) / per_tick)` ticks) and a second more.
    /// Answers whether it spent, the level plus what `credit` holds, and the tick's time.
    pub async fn mana_pool(
        &self,
        keys: &ManaKeys,
        rates: ManaRates,
        cost: i64,
        spend: ManaSpend,
        sender: Option<&str>,
        sent_kept_ms: i64,
    ) -> Result<ManaSpent> {
        match self {
            Ephemeral::Valkey(client) => {
                valkey::mana_pool(client, keys, rates, cost, spend, sender, sent_kept_ms).await
            }
        }
    }

    /// A mana pool's queue's allocation, in one step on the store's clock, the pool earning and
    /// held as in [`mana_pool`](Self::mana_pool). `turns` are the queue's first authors, at most
    /// three, in order.
    ///
    /// `Share`: credit held by anyone not in `turns` goes back to the pool, the pool is held to
    /// `max` less the credit left, and whatever it then holds above zero is split into three
    /// portions of `floor(level / 3)`, each author credited their weight's (the rounding's
    /// remainder to the first), leaving the pool at zero. Answers, for each, whether their row can
    /// be paid: their credit covers its cost, or, while the level is not below zero, their credit
    /// has reached `floor(weight * max / 3)`.
    ///
    /// `Pay`: given one author, pays their row from their credit when it covers the cost, or else,
    /// when it can be paid as above, with all of their credit and the rest from the pool, no lower
    /// than `-floor(max / 2)`. A cost above zero paid is recorded as in `mana_pool`, in `sent`.
    /// Answers whether it was paid.
    ///
    /// Either way the credit is written back without those at zero, expiring a minute from now
    /// when any is left, and the pool as `mana_pool` writes it. The level answered includes the
    /// credit.
    pub async fn mana_allot(
        &self,
        keys: &ManaKeys,
        rates: ManaRates,
        allot: ManaAllot,
        sent_kept_ms: i64,
        turns: &[ManaTurn],
    ) -> Result<ManaAllotted> {
        match self {
            Ephemeral::Valkey(client) => {
                valkey::mana_allot(client, keys, rates, allot, sent_kept_ms, turns).await
            }
        }
    }

    /// The first `count` members of the union of the sorted sets `minutes`, highest score first,
    /// each with its score: the sum of its scores in them. The union is kept at `cache` for
    /// `cached_for` once made, when it is not empty, and read from there while it lasts.
    pub async fn mana_top(
        &self,
        cache: &str,
        minutes: Vec<String>,
        count: usize,
        cached_for: Duration,
    ) -> Result<Vec<(String, f64)>> {
        match self {
            Ephemeral::Valkey(client) => {
                valkey::mana_top(client, cache, minutes, count, cached_for).await
            }
        }
    }
}

#[cfg(test)]
mod tests;
