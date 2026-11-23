//! [`Ephemeral::Memory`](crate::Ephemeral::Memory): the store held in this process, answering
//! every operation as Valkey answers it for one caller, and each of Valkey's Lua scripts
//! ([`crate::valkey`]) ported to Rust step for step.
//!
//! The scripts run on Valkey's Lua 5.1, where every number is a double, so their ports compute
//! in `f64` as Lua does: `math.floor` and `math.ceil` are [`f64::floor`] and [`f64::ceil`] (the
//! same on negatives), `tonumber` is [`tonumber`], a number passed to a command is the text
//! Valkey makes of it ([`lua_text`], [`lua_integer_arg`]), and a number returned is truncated
//! toward zero as Valkey's `(long long)` cast truncates it ([`lua_integer`]). Every value they
//! handle in practice is an integer far below 2^53, where this is the integer arithmetic it
//! reads as; computing in doubles keeps them alike beyond that too.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Bound as Edge;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::sealed::Sealed;
use crate::{
    Command, Condition, Error, Expiry, GcraRate, ManaAllot, ManaAllotted, ManaKeys, ManaRates,
    ManaSpend, ManaSpent, ManaTurn, Result, Ttl, Value,
};

/// Bytes counted for each key beside its text: the map's slot, the entry, and the deadline
/// index's place. The key's text is counted twice, since the deadline index holds a copy; it is
/// counted so whether or not the key expires, so giving a key an expiry never needs room.
pub(crate) const KEY_OVERHEAD: usize = 96;

/// Bytes counted for each member of a set, sorted set, or hash beside its text.
pub(crate) const MEMBER_OVERHEAD: usize = 48;

/// How many keys past their deadline each operation removes before its own work, soonest first.
const SWEEP: usize = 20;

/// The store held in this process, for a deployment whose one API server is all that reads it.
/// Cheap to clone; every clone holds the same keys.
///
/// It holds at most `max_bytes`, counting each key's text, its value's or members' text, and a
/// fixed overhead for each (`KEY_OVERHEAD`, `MEMBER_OVERHEAD`). A write that would pass the
/// bound first evicts keys that expire, the soonest to expire first (Valkey's `volatile-ttl`),
/// never the key it writes nor one its script names; when evicting every such key would not make
/// room, it evicts none and is refused with [`Error::Full`]. A script refused so partway is
/// undone, leaving every key as it was, as Valkey refuses a script before it starts when it has
/// no room. Keys past their deadline are removed when they are next touched, and each operation
/// first removes up to a handful of them, soonest first, from an index ordered by deadline, so
/// the space of keys nobody reads again comes back without anything scanning every key.
///
/// Each operation, each pipeline, and each script holds one lock for the whole of it, so it is
/// atomic to every other task, and reads the clock once: a key expires once that time is past its
/// deadline in Unix milliseconds of [`SystemTime`], and the scripts' `TIME` is that same instant.
#[derive(Clone)]
pub struct Memory {
    state: Arc<Mutex<State>>,
}

/// What an operation answered, converted to what its caller asked for by `Sealed::from_answer`.
#[derive(Debug)]
pub enum Answer {
    Nil,
    Text(String),
    Integer(i64),
    Texts(Vec<String>),
    MaybeTexts(Vec<Option<String>>),
    Ttl(Ttl),
}

/// What Valkey would refuse, or what keeps the store from answering, as [`Error::Failed`]. The
/// texts are Valkey's where it has one.
#[derive(Debug, thiserror::Error)]
enum Refusal {
    #[error("WRONGTYPE Operation against a key holding the wrong kind of value")]
    WrongType,
    #[error("ERR value is not an integer or out of range")]
    NotInteger,
    #[error("ERR increment or decrement would overflow")]
    Overflow,
    #[error("ERR invalid expire time in '{0}' command")]
    ExpireTime(&'static str),
    #[error("ERR value is not a valid float")]
    NotFloat,
    #[error("ERR resulting score is not a number (NaN)")]
    NaN,
    #[error("ERR min or max is not a float")]
    Bound,
    #[error("ERR wrong number of arguments for '{0}' command")]
    Arity(&'static str),
    #[error("ERR at least 1 input key is needed for '{0}' command")]
    NoInputKeys(&'static str),
    #[error("ERR Error running script: {0}")]
    Script(&'static str),
    #[error("the value read is not what it was read as")]
    Unreadable,
    #[error("no answer for a pipelined command")]
    NoAnswer,
    #[error("the system clock is set before 1970")]
    Clock,
    #[error("the in-process ephemeral store was left unusable by a panic while it was locked")]
    Poisoned,
}

impl From<Refusal> for Error {
    fn from(refusal: Refusal) -> Self {
        Error::Failed(Box::new(refusal))
    }
}

impl Sealed for String {
    fn from_answer(answer: Answer) -> Result<Self> {
        match answer {
            Answer::Text(text) => Ok(text),
            Answer::Integer(integer) => Ok(integer.to_string()),
            Answer::Nil | Answer::Texts(_) | Answer::MaybeTexts(_) | Answer::Ttl(_) => {
                Err(Refusal::Unreadable.into())
            }
        }
    }
}

/// Text reads as an integer by [`str::parse`], as the Valkey client reads it.
impl Sealed for i64 {
    fn from_answer(answer: Answer) -> Result<Self> {
        match answer {
            Answer::Text(text) => text.parse().map_err(|_| Refusal::Unreadable.into()),
            Answer::Integer(integer) => Ok(integer),
            Answer::Nil | Answer::Texts(_) | Answer::MaybeTexts(_) | Answer::Ttl(_) => {
                Err(Refusal::Unreadable.into())
            }
        }
    }
}

impl Sealed for u64 {
    fn from_answer(answer: Answer) -> Result<Self> {
        match answer {
            Answer::Text(text) => text.parse().map_err(|_| Refusal::Unreadable.into()),
            Answer::Integer(integer) => {
                u64::try_from(integer).map_err(|_| Refusal::Unreadable.into())
            }
            Answer::Nil | Answer::Texts(_) | Answer::MaybeTexts(_) | Answer::Ttl(_) => {
                Err(Refusal::Unreadable.into())
            }
        }
    }
}

impl<V: crate::Stored> Sealed for Option<V> {
    fn from_answer(answer: Answer) -> Result<Self> {
        match answer {
            Answer::Nil => Ok(None),
            answer @ (Answer::Text(_)
            | Answer::Integer(_)
            | Answer::Texts(_)
            | Answer::MaybeTexts(_)
            | Answer::Ttl(_)) => V::from_answer(answer).map(Some),
        }
    }
}

impl Sealed for Ttl {
    fn from_answer(answer: Answer) -> Result<Self> {
        match answer {
            Answer::Ttl(ttl) => Ok(ttl),
            Answer::Nil
            | Answer::Text(_)
            | Answer::Integer(_)
            | Answer::Texts(_)
            | Answer::MaybeTexts(_) => Err(Refusal::Unreadable.into()),
        }
    }
}

impl Sealed for Vec<String> {
    fn from_answer(answer: Answer) -> Result<Self> {
        match answer {
            Answer::Texts(texts) => Ok(texts),
            Answer::Nil
            | Answer::Text(_)
            | Answer::Integer(_)
            | Answer::MaybeTexts(_)
            | Answer::Ttl(_) => Err(Refusal::Unreadable.into()),
        }
    }
}

impl Sealed for Vec<Option<String>> {
    fn from_answer(answer: Answer) -> Result<Self> {
        match answer {
            Answer::MaybeTexts(texts) => Ok(texts),
            Answer::Nil
            | Answer::Text(_)
            | Answer::Integer(_)
            | Answer::Texts(_)
            | Answer::Ttl(_) => Err(Refusal::Unreadable.into()),
        }
    }
}

pub(crate) fn take<T: crate::Reply>(answers: &mut [Option<Answer>], at: usize) -> Result<T> {
    let answer = answers
        .get_mut(at)
        .and_then(Option::take)
        .ok_or(Refusal::NoAnswer)?;
    T::from_answer(answer)
}

/// Where the time comes from: the system's clock, or in tests one they set.
enum Clock {
    System,
    #[cfg(test)]
    Set(Arc<std::sync::atomic::AtomicI64>),
}

impl Clock {
    /// Unix microseconds.
    fn micros(&self) -> Result<i64> {
        match self {
            Clock::System => {
                let since = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| Refusal::Clock)?;
                Ok(i64::try_from(since.as_micros()).map_err(|_| Refusal::Clock)?)
            }
            #[cfg(test)]
            Clock::Set(micros) => Ok(micros.load(std::sync::atomic::Ordering::SeqCst)),
        }
    }
}

/// A sorted set's score, ordered as Valkey orders them: by value, `-0` equal to `0`. Never NaN,
/// which every write refuses.
#[derive(Debug, Clone, Copy)]
struct Score(f64);

impl PartialEq for Score {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for Score {}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Score {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self.0 < other.0 {
            std::cmp::Ordering::Less
        } else if self.0 > other.0 {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }
}

/// A sorted set: each member's score, and the members in Valkey's order, by score and then by
/// member's bytes.
#[derive(Debug, Default)]
struct Sorted {
    scores: HashMap<String, f64>,
    order: BTreeSet<(Score, String)>,
}

impl Sorted {
    /// Gives `member` `score`, or removes it for `None`, answering its score before.
    fn put(&mut self, member: &str, score: Option<f64>) -> Option<f64> {
        let prior = match score {
            Some(score) => self.scores.insert(member.to_string(), score),
            None => self.scores.remove(member),
        };
        if let Some(prior) = prior {
            self.order.remove(&(Score(prior), member.to_string()));
        }
        if let Some(score) = score {
            self.order.insert((Score(score), member.to_string()));
        }
        prior
    }

    /// The members within `min` and `max`, lowest first.
    fn range(&self, min: Bound, max: Bound) -> impl Iterator<Item = &(Score, String)> {
        self.order
            .range((
                Edge::Included((Score(min.value), String::new())),
                Edge::Unbounded,
            ))
            .skip_while(move |(score, _)| !min.admits_above(score.0))
            .take_while(move |(score, _)| max.admits_below(score.0))
    }
}

/// One end of a range of scores, as Valkey reads `5` (included) or `(5` (excluded).
#[derive(Debug, Clone, Copy)]
struct Bound {
    value: f64,
    excluded: bool,
}

impl Bound {
    fn included(value: f64) -> Result<Self> {
        if value.is_nan() {
            return Err(Refusal::Bound.into());
        }
        Ok(Bound {
            value,
            excluded: false,
        })
    }

    fn excluded(value: f64) -> Result<Self> {
        if value.is_nan() {
            return Err(Refusal::Bound.into());
        }
        Ok(Bound {
            value,
            excluded: true,
        })
    }

    /// Whether `score` is no lower than this bound as a range's minimum.
    fn admits_above(self, score: f64) -> bool {
        if self.excluded {
            score > self.value
        } else {
            score >= self.value
        }
    }

    /// Whether `score` is no higher than this bound as a range's maximum.
    fn admits_below(self, score: f64) -> bool {
        if self.excluded {
            score < self.value
        } else {
            score <= self.value
        }
    }
}

#[derive(Debug)]
enum Data {
    Text(String),
    Set(HashSet<String>),
    Sorted(Sorted),
    Hash(HashMap<String, String>),
}

impl Data {
    fn bytes(&self) -> usize {
        match self {
            Data::Text(text) => text.len(),
            Data::Set(members) => members.iter().map(|member| set_member_bytes(member)).sum(),
            Data::Sorted(sorted) => sorted
                .scores
                .keys()
                .map(|member| sorted_member_bytes(member))
                .sum(),
            Data::Hash(fields) => fields
                .iter()
                .map(|(field, value)| field_bytes(field, value))
                .sum(),
        }
    }
}

pub(crate) fn key_bytes(key: &str) -> usize {
    KEY_OVERHEAD + 2 * key.len()
}

fn set_member_bytes(member: &str) -> usize {
    MEMBER_OVERHEAD + member.len()
}

/// A sorted set holds each member twice, in its scores and its order, with its score.
pub(crate) fn sorted_member_bytes(member: &str) -> usize {
    MEMBER_OVERHEAD + 2 * member.len() + 2 * size_of::<f64>()
}

pub(crate) fn field_bytes(field: &str, value: &str) -> usize {
    MEMBER_OVERHEAD + field.len() + value.len()
}

#[derive(Debug)]
struct Entry {
    data: Data,
    /// Unix milliseconds; the key is gone once the time is past it.
    deadline: Option<i64>,
    /// What [`Memory`] counts it as: [`key_bytes`] and its data's.
    bytes: usize,
}

impl Entry {
    fn new(key: &str, data: Data, deadline: Option<i64>) -> Self {
        Entry {
            bytes: key_bytes(key) + data.bytes(),
            data,
            deadline,
        }
    }
}

/// How to put back one change a script made, should it be refused for room partway.
enum Undo {
    /// The key held this before, or nothing.
    Key {
        key: String,
        prior: Option<Entry>,
    },
    Field {
        key: String,
        field: String,
        prior: Option<String>,
    },
    Score {
        key: String,
        member: String,
        prior: Option<f64>,
    },
    /// The member was added to the set.
    Member {
        key: String,
        member: String,
    },
    Deadline {
        key: String,
        prior: Option<i64>,
    },
}

/// A script in progress: the keys it names, which nothing it writes evicts, and how to undo it.
struct Script {
    keys: Vec<String>,
    undo: Vec<Undo>,
}

struct State {
    entries: HashMap<String, Entry>,
    deadlines: BTreeSet<(i64, String)>,
    bytes: usize,
    /// Of `bytes`, what keys with a deadline hold.
    volatile_bytes: usize,
    max_bytes: usize,
    clock: Clock,
    /// The operation in progress's time, in Unix milliseconds.
    now: i64,
    /// The same instant as a script's `TIME` makes it: seconds times 1000 plus the floor of the
    /// microseconds over 1000, in doubles.
    lua_now: f64,
    script: Option<Script>,
}

/// Lua 5.1's `tonumber` of a string, which is C's `strtod` with spaces around it allowed: decimal
/// text, with an exponent, `inf`, or `nan`, or hexadecimal integer text.
fn tonumber(text: &str) -> Option<f64> {
    let trimmed =
        text.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r'));
    if let Ok(number) = trimmed.parse::<f64>() {
        return Some(number);
    }
    let (negative, unsigned) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let digits = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))?;
    let magnitude = u128::from_str_radix(digits, 16).ok()? as f64;
    Some(if negative { -magnitude } else { magnitude })
}

/// Valkey's `double2ll`: the integer a double is, when it is one within half of `i64`'s range.
fn double2ll(number: f64) -> Option<i64> {
    let half = (i64::MAX / 2) as f64;
    if !(-half..=half).contains(&number) {
        return None;
    }
    let integer = number as i64;
    (integer as f64 == number).then_some(integer)
}

/// The text Valkey makes of a Lua number passed to a command: the integer when [`double2ll`]
/// finds one, otherwise the shortest text that reads back as the same double (Valkey writes an
/// exponent where this writes every digit; both read back alike).
fn lua_text(number: f64) -> String {
    match double2ll(number) {
        Some(integer) => integer.to_string(),
        None if number.is_nan() => "nan".to_string(),
        None if number == f64::INFINITY => "inf".to_string(),
        None if number == f64::NEG_INFINITY => "-inf".to_string(),
        None => number.to_string(),
    }
}

/// A Lua number passed where a command reads an integer: refused unless [`lua_text`] wrote one.
fn lua_integer_arg(number: f64) -> Result<i64> {
    Ok(double2ll(number).ok_or(Refusal::NotInteger)?)
}

/// A Lua number returned by a script, as Valkey's `(long long)` cast makes it an integer reply:
/// truncated toward zero (saturating where C leaves it undefined).
fn lua_integer(number: f64) -> i64 {
    number as i64
}

/// Lua 5.1's `math.max` of two: the first unless the second is greater.
fn lua_max(first: f64, second: f64) -> f64 {
    if second > first { second } else { first }
}

/// Valkey's `string2ll`: only the integer's own decimal text, no sign but `-`, no leading zeros.
fn string2ll(text: &str) -> Option<i64> {
    let integer = text.parse::<i64>().ok()?;
    (integer.to_string() == text).then_some(integer)
}

impl Memory {
    /// A store holding at most `max_bytes`, as [`Memory`] counts them.
    pub fn new(max_bytes: usize) -> Self {
        Self::with(max_bytes, Clock::System)
    }

    fn with(max_bytes: usize, clock: Clock) -> Self {
        Memory {
            state: Arc::new(Mutex::new(State {
                entries: HashMap::new(),
                deadlines: BTreeSet::new(),
                bytes: 0,
                volatile_bytes: 0,
                max_bytes,
                clock,
                now: 0,
                lua_now: 0.0,
                script: None,
            })),
        }
    }

    /// A store whose time is `micros`, Unix microseconds, which the test moves.
    #[cfg(test)]
    pub(crate) fn with_clock(max_bytes: usize, micros: Arc<std::sync::atomic::AtomicI64>) -> Self {
        Self::with(max_bytes, Clock::Set(micros))
    }

    /// How many bytes it holds, as it counts them, keys past their deadline included until they
    /// are removed.
    pub fn used_bytes(&self) -> Result<usize> {
        let state = self.state.lock().map_err(|_| Refusal::Poisoned)?;
        Ok(state.bytes)
    }

    /// Runs `operation` under the lock, at one time, after sweeping.
    fn locked<T>(&self, operation: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let mut state = self.state.lock().map_err(|_| Refusal::Poisoned)?;
        state.begin()?;
        operation(&mut state)
    }

    pub(crate) fn ping(&self) -> Result<()> {
        self.locked(|_| Ok(()))
    }

    pub(crate) fn get(&self, key: &str) -> Result<Answer> {
        self.locked(|state| state.get(key))
    }

    pub(crate) fn getdel(&self, key: &str) -> Result<Answer> {
        self.locked(|state| {
            let answer = state.get(key)?;
            state.remove(key);
            Ok(answer)
        })
    }

    pub(crate) fn set(
        &self,
        key: &str,
        value: Value,
        expiry: Option<Expiry>,
        condition: Option<Condition>,
    ) -> Result<bool> {
        self.locked(|state| {
            state
                .set(key, value, expiry, condition, false)
                .map(|(written, _)| written)
        })
    }

    pub(crate) fn set_previous(
        &self,
        key: &str,
        value: Value,
        expiry: Option<Expiry>,
        condition: Option<Condition>,
    ) -> Result<Answer> {
        self.locked(|state| {
            state
                .set(key, value, expiry, condition, true)
                .map(|(_, previous)| previous)
        })
    }

    pub(crate) fn del(&self, keys: Vec<String>) -> Result<i64> {
        self.locked(|state| state.del(&keys))
    }

    pub(crate) fn exists(&self, key: &str) -> Result<bool> {
        self.locked(|state| Ok(state.lookup(key).is_some()))
    }

    pub(crate) fn incr_by(&self, key: &str, by: i64) -> Result<i64> {
        self.locked(|state| state.incr_by(key, by))
    }

    pub(crate) fn expire(&self, key: &str, seconds: i64) -> Result<bool> {
        self.locked(|state| state.expire(key, seconds, Unit::Seconds))
    }

    pub(crate) fn ttl(&self, key: &str) -> Result<Ttl> {
        self.locked(|state| Ok(state.ttl(key, Unit::Seconds)))
    }

    pub(crate) fn pttl(&self, key: &str) -> Result<Ttl> {
        self.locked(|state| Ok(state.ttl(key, Unit::Millis)))
    }

    pub(crate) fn mget(&self, keys: Vec<String>) -> Result<Vec<Option<String>>> {
        self.locked(|state| state.mget(&keys))
    }

    pub(crate) fn sadd(&self, key: &str, members: Vec<String>) -> Result<i64> {
        self.locked(|state| state.sadd(key, &members))
    }

    pub(crate) fn smembers(&self, key: &str) -> Result<Vec<String>> {
        self.locked(|state| match state.lookup(key).map(|entry| &entry.data) {
            None => Ok(Vec::new()),
            Some(Data::Set(members)) => Ok(members.iter().cloned().collect()),
            Some(Data::Text(_) | Data::Sorted(_) | Data::Hash(_)) => Err(Refusal::WrongType.into()),
        })
    }

    pub(crate) fn zrem(&self, key: &str, member: &str) -> Result<i64> {
        self.locked(|state| {
            if state
                .sorted(key)?
                .is_none_or(|sorted| !sorted.scores.contains_key(member))
            {
                return Ok(0);
            }
            state.set_score(key, member, None);
            state.drop_if_empty(key);
            Ok(1)
        })
    }

    /// Runs `commands` in order, each as it would run alone, answering each; the first error
    /// when any fails, after running the rest, as Valkey runs every command of a pipeline.
    pub(crate) fn run(&self, commands: Vec<Command>) -> Result<Vec<Option<Answer>>> {
        self.locked(|state| {
            let mut answers = Vec::with_capacity(commands.len());
            let mut failed = None;
            for command in commands {
                match state.command(command) {
                    Ok(answer) => answers.push(Some(answer)),
                    Err(error) => {
                        answers.push(None);
                        failed.get_or_insert(error);
                    }
                }
            }
            match failed {
                Some(error) => Err(error),
                None => Ok(answers),
            }
        })
    }

    pub(crate) fn gcra_take(&self, key: &str, rate: GcraRate) -> Result<Option<Duration>> {
        self.locked(|state| state.gcra(key, rate, Gcra::Take))
            .map(waited)
    }

    pub(crate) fn gcra_peek(&self, key: &str, rate: GcraRate) -> Result<Option<Duration>> {
        self.locked(|state| state.gcra(key, rate, Gcra::Peek))
            .map(waited)
    }

    pub(crate) fn gcra_refund(&self, key: &str, rate: GcraRate) -> Result<()> {
        self.locked(|state| state.gcra_refund(key, rate))
    }

    pub(crate) fn count_attempt(&self, key: &str, window_seconds: i64) -> Result<i64> {
        self.locked(|state| state.count_attempt(key, window_seconds))
    }

    pub(crate) fn attention_mark(&self, key: &str, member: &str, lasting: Duration) -> Result<()> {
        self.locked(|state| state.attention_mark(key, member, lasting))
    }

    pub(crate) fn attention_ask(&self, keys: Vec<String>) -> Result<Vec<bool>> {
        self.locked(|state| state.attention_ask(&keys))
    }

    pub(crate) fn badge_add(&self, key: &str) -> Result<Option<i64>> {
        self.locked(|state| state.badge_add(key))
    }

    pub(crate) fn mana_pool(
        &self,
        keys: &ManaKeys,
        rates: ManaRates,
        cost: i64,
        spend: ManaSpend,
        sender: Option<&str>,
        sent_kept_ms: i64,
    ) -> Result<ManaSpent> {
        self.locked(|state| state.mana_pool(keys, rates, cost, spend, sender, sent_kept_ms))
    }

    pub(crate) fn mana_allot(
        &self,
        keys: &ManaKeys,
        rates: ManaRates,
        allot: ManaAllot,
        sent_kept_ms: i64,
        turns: &[ManaTurn],
    ) -> Result<ManaAllotted> {
        self.locked(|state| state.mana_allot(keys, rates, allot, sent_kept_ms, turns))
    }

    pub(crate) fn mana_top(
        &self,
        cache: &str,
        minutes: Vec<String>,
        count: usize,
        cached_for: Duration,
    ) -> Result<Vec<(String, f64)>> {
        self.locked(|state| state.mana_top(cache, &minutes, count, cached_for))
    }
}

/// What the tests read and write directly, as they do Valkey's hashes and sorted sets.
#[cfg(test)]
impl Memory {
    pub(crate) fn hget(&self, key: &str, field: &str) -> Result<Option<String>> {
        self.locked(|state| {
            Ok(state
                .hash(key)?
                .and_then(|fields| fields.get(field).cloned()))
        })
    }

    pub(crate) fn hset(&self, key: &str, field: &str, value: &str) -> Result<()> {
        self.locked(|state| state.hset(key, &[(field, value.to_string())]))
    }

    pub(crate) fn zincrby(&self, key: &str, by: f64, member: &str) -> Result<f64> {
        self.locked(|state| state.zincrby(key, by, member))
    }

    /// Every member and score, lowest first.
    pub(crate) fn zrange(&self, key: &str) -> Result<Vec<(String, f64)>> {
        self.locked(|state| {
            Ok(state.sorted(key)?.map_or_else(Vec::new, |sorted| {
                sorted
                    .order
                    .iter()
                    .map(|(score, member)| (member.clone(), score.0))
                    .collect()
            }))
        })
    }
}

/// A GCRA script's wait, as [`crate::valkey`] reads it.
fn waited(wait: i64) -> Option<Duration> {
    (wait > 0).then(|| Duration::from_millis(wait.unsigned_abs()))
}

#[derive(Clone, Copy)]
enum Unit {
    Seconds,
    Millis,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gcra {
    Take,
    Peek,
}

impl State {
    /// Reads the clock and removes up to [`SWEEP`] keys past their deadline.
    fn begin(&mut self) -> Result<()> {
        let micros = self.clock.micros()?;
        self.now = micros.div_euclid(1000);
        self.lua_now = micros.div_euclid(1_000_000) as f64 * 1000.0
            + (micros.rem_euclid(1_000_000) as f64 / 1000.0).floor();
        for _ in 0..SWEEP {
            let Some((deadline, key)) = self.deadlines.first() else {
                break;
            };
            if *deadline >= self.now {
                break;
            }
            let key = key.clone();
            self.take_entry(&key);
        }
        Ok(())
    }

    // What every change goes through, keeping the counts and the deadline index.

    fn take_entry(&mut self, key: &str) -> Option<Entry> {
        let entry = self.entries.remove(key)?;
        self.bytes -= entry.bytes;
        if let Some(deadline) = entry.deadline {
            self.deadlines.remove(&(deadline, key.to_string()));
            self.volatile_bytes -= entry.bytes;
        }
        Some(entry)
    }

    /// Writes `entry` at `key`, which holds nothing.
    fn insert_entry(&mut self, key: &str, entry: Entry) {
        self.bytes += entry.bytes;
        if let Some(deadline) = entry.deadline {
            self.deadlines.insert((deadline, key.to_string()));
            self.volatile_bytes += entry.bytes;
        }
        self.entries.insert(key.to_string(), entry);
    }

    fn set_deadline(&mut self, key: &str, deadline: Option<i64>) -> Option<i64> {
        let entry = self.entries.get_mut(key)?;
        let bytes = entry.bytes;
        let prior = std::mem::replace(&mut entry.deadline, deadline);
        if let Some(prior) = prior {
            self.deadlines.remove(&(prior, key.to_string()));
            self.volatile_bytes -= bytes;
        }
        if let Some(deadline) = deadline {
            self.deadlines.insert((deadline, key.to_string()));
            self.volatile_bytes += bytes;
        }
        prior
    }

    /// Counts a change of `key`'s data from `before` bytes to `after`.
    fn resized(&mut self, key: &str, before: usize, after: usize) {
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        entry.bytes = entry.bytes + after - before;
        self.bytes = self.bytes + after - before;
        if entry.deadline.is_some() {
            self.volatile_bytes = self.volatile_bytes + after - before;
        }
    }

    /// Gives the hash `key`'s `field` `value`, or removes it for `None`, answering its value
    /// before. Nothing when `key` holds no hash.
    fn set_field(&mut self, key: &str, field: &str, value: Option<String>) -> Option<String> {
        let Some(Entry {
            data: Data::Hash(fields),
            ..
        }) = self.entries.get_mut(key)
        else {
            return None;
        };
        let after = value
            .as_deref()
            .map_or(0, |value| field_bytes(field, value));
        let prior = match value {
            Some(value) => fields.insert(field.to_string(), value),
            None => fields.remove(field),
        };
        let before = prior
            .as_deref()
            .map_or(0, |prior| field_bytes(field, prior));
        self.resized(key, before, after);
        prior
    }

    /// As [`set_field`](Self::set_field), for a sorted set's member.
    fn set_score(&mut self, key: &str, member: &str, score: Option<f64>) -> Option<f64> {
        let Some(Entry {
            data: Data::Sorted(sorted),
            ..
        }) = self.entries.get_mut(key)
        else {
            return None;
        };
        let prior = sorted.put(member, score);
        let size = sorted_member_bytes(member);
        let before = if prior.is_some() { size } else { 0 };
        let after = if score.is_some() { size } else { 0 };
        self.resized(key, before, after);
        self.record(Undo::Score {
            key: key.to_string(),
            member: member.to_string(),
            prior,
        });
        prior
    }

    /// Adds `member` to the set `key`, or removes it, answering whether it was there.
    fn set_member(&mut self, key: &str, member: &str, present: bool) -> bool {
        let Some(Entry {
            data: Data::Set(members),
            ..
        }) = self.entries.get_mut(key)
        else {
            return false;
        };
        let was = if present {
            !members.insert(member.to_string())
        } else {
            members.remove(member)
        };
        let size = set_member_bytes(member);
        self.resized(
            key,
            if was { size } else { 0 },
            if present { size } else { 0 },
        );
        was
    }

    // Undoing.

    fn record(&mut self, undo: Undo) {
        if let Some(script) = &mut self.script {
            script.undo.push(undo);
        }
    }

    /// Runs `body` as the script naming `keys`, undoing every change it made when it is refused
    /// for room.
    fn script<T>(&mut self, keys: &[&str], body: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.script = Some(Script {
            keys: keys.iter().map(|key| key.to_string()).collect(),
            undo: Vec::new(),
        });
        let result = body(self);
        let script = self.script.take();
        if let (Err(Error::Full { .. }), Some(script)) = (&result, script) {
            for undo in script.undo.into_iter().rev() {
                self.undo(undo);
            }
        }
        result
    }

    fn undo(&mut self, undo: Undo) {
        match undo {
            Undo::Key { key, prior } => {
                self.take_entry(&key);
                if let Some(prior) = prior {
                    self.insert_entry(&key, prior);
                }
            }
            Undo::Field { key, field, prior } => {
                self.set_field(&key, &field, prior);
            }
            Undo::Score { key, member, prior } => {
                self.set_score(&key, &member, prior);
            }
            Undo::Member { key, member } => {
                self.set_member(&key, &member, false);
            }
            Undo::Deadline { key, prior } => {
                self.set_deadline(&key, prior);
            }
        }
    }

    // Room.

    /// Makes room for `key` to grow by `grow` bytes, evicting keys with a deadline, soonest
    /// first, but never `key` or one the running script names; refused, evicting nothing, when
    /// all of those would not make room.
    fn reserve(&mut self, key: &str, grow: usize) -> Result<()> {
        let needed = (self.bytes.saturating_add(grow)).saturating_sub(self.max_bytes);
        if needed == 0 {
            return Ok(());
        }
        let script_keys = self
            .script
            .as_ref()
            .map_or(&[][..], |script| &script.keys[..]);
        let mut kept: Vec<&str> = Vec::with_capacity(1 + script_keys.len());
        for kept_key in std::iter::once(key).chain(script_keys.iter().map(String::as_str)) {
            if !kept.contains(&kept_key) {
                kept.push(kept_key);
            }
        }
        let kept_volatile: usize = kept
            .iter()
            .filter_map(|kept_key| self.entries.get(*kept_key))
            .filter(|entry| entry.deadline.is_some())
            .map(|entry| entry.bytes)
            .sum();
        if self.volatile_bytes - kept_volatile < needed {
            return Err(Error::Full {
                max_bytes: self.max_bytes,
            });
        }
        let mut freed = 0;
        let mut evicted = Vec::new();
        for (_, candidate) in &self.deadlines {
            if freed >= needed {
                break;
            }
            if kept.contains(&candidate.as_str()) {
                continue;
            }
            freed += self.entries.get(candidate).map_or(0, |entry| entry.bytes);
            evicted.push(candidate.clone());
        }
        for key in evicted {
            self.remove(&key);
        }
        Ok(())
    }

    // The commands, each as Valkey carries it out, for one command alone or a script's call.

    /// The entry at `key`, removing it first when it is past its deadline.
    fn lookup(&mut self, key: &str) -> Option<&mut Entry> {
        let now = self.now;
        if self
            .entries
            .get(key)
            .is_some_and(|entry| entry.deadline.is_some_and(|deadline| now > deadline))
        {
            self.remove(key);
            return None;
        }
        self.entries.get_mut(key)
    }

    /// Removes `key`, answering whether it was set (past its deadline or not).
    fn remove(&mut self, key: &str) -> bool {
        let Some(prior) = self.take_entry(key) else {
            return false;
        };
        self.record(Undo::Key {
            key: key.to_string(),
            prior: Some(prior),
        });
        true
    }

    /// Writes `entry` at `key`, replacing whatever it held, after making room.
    fn put(&mut self, key: &str, entry: Entry) -> Result<()> {
        let held = self.entries.get(key).map_or(0, |held| held.bytes);
        self.reserve(key, entry.bytes.saturating_sub(held))?;
        let prior = self.take_entry(key);
        self.record(Undo::Key {
            key: key.to_string(),
            prior,
        });
        self.insert_entry(key, entry);
        Ok(())
    }

    /// Makes room for `key` to grow by `grow` bytes, and makes it, holding `empty`, when it is not
    /// set; the caller has checked what kind it holds when it is.
    fn ready(&mut self, key: &str, grow: usize, empty: fn() -> Data) -> Result<()> {
        let exists = self.lookup(key).is_some();
        let grow = grow + if exists { 0 } else { key_bytes(key) };
        self.reserve(key, grow)?;
        if !exists {
            self.record(Undo::Key {
                key: key.to_string(),
                prior: None,
            });
            self.insert_entry(key, Entry::new(key, empty(), None));
        }
        Ok(())
    }

    /// Removes `key` when it is a set, sorted set, or hash left empty, as Valkey keeps none.
    fn drop_if_empty(&mut self, key: &str) {
        let empty = match self.entries.get(key).map(|entry| &entry.data) {
            Some(Data::Set(members)) => members.is_empty(),
            Some(Data::Sorted(sorted)) => sorted.scores.is_empty(),
            Some(Data::Hash(fields)) => fields.is_empty(),
            Some(Data::Text(_)) | None => false,
        };
        if empty {
            self.remove(key);
        }
    }

    fn text(&mut self, key: &str) -> Result<Option<&String>> {
        match self.lookup(key).map(|entry| &entry.data) {
            None => Ok(None),
            Some(Data::Text(text)) => Ok(Some(text)),
            Some(Data::Set(_) | Data::Sorted(_) | Data::Hash(_)) => Err(Refusal::WrongType.into()),
        }
    }

    fn sorted(&mut self, key: &str) -> Result<Option<&Sorted>> {
        match self.lookup(key).map(|entry| &entry.data) {
            None => Ok(None),
            Some(Data::Sorted(sorted)) => Ok(Some(sorted)),
            Some(Data::Text(_) | Data::Set(_) | Data::Hash(_)) => Err(Refusal::WrongType.into()),
        }
    }

    fn hash(&mut self, key: &str) -> Result<Option<&HashMap<String, String>>> {
        match self.lookup(key).map(|entry| &entry.data) {
            None => Ok(None),
            Some(Data::Hash(fields)) => Ok(Some(fields)),
            Some(Data::Text(_) | Data::Set(_) | Data::Sorted(_)) => Err(Refusal::WrongType.into()),
        }
    }

    fn get(&mut self, key: &str) -> Result<Answer> {
        Ok(self
            .text(key)?
            .map_or(Answer::Nil, |text| Answer::Text(text.clone())))
    }

    /// `SET`, answering whether it wrote and, when `previous`, the value before (`GET`).
    fn set(
        &mut self,
        key: &str,
        value: Value,
        expiry: Option<Expiry>,
        condition: Option<Condition>,
        previous: bool,
    ) -> Result<(bool, Answer)> {
        let deadline = expiry.map(|expiry| self.deadline_of(expiry)).transpose()?;
        let prior = if previous {
            self.get(key)?
        } else {
            Answer::Nil
        };
        let exists = self.lookup(key).is_some();
        let allowed = match condition {
            None => true,
            Some(Condition::Absent) => !exists,
            Some(Condition::Present) => exists,
        };
        if allowed {
            let text = match value {
                Value::Text(text) => text,
                Value::Integer(integer) => integer.to_string(),
            };
            self.put(key, Entry::new(key, Data::Text(text), deadline))?;
        }
        Ok((allowed, prior))
    }

    /// `SET`'s expiry as a deadline, refused as Valkey refuses it: not above zero, or past
    /// `i64`'s milliseconds.
    fn deadline_of(&self, expiry: Expiry) -> Result<i64> {
        let refused = || Error::from(Refusal::ExpireTime("set"));
        let (amount, scale, relative) = match expiry {
            Expiry::Seconds(seconds) => (seconds, 1000, true),
            Expiry::Millis(millis) => (millis, 1, true),
            Expiry::AtMillis(at) => (at, 1, false),
        };
        if amount <= 0 {
            return Err(refused());
        }
        let millis = amount.checked_mul(scale).ok_or_else(refused)?;
        if relative {
            millis.checked_add(self.now).ok_or_else(refused)
        } else {
            Ok(millis)
        }
    }

    fn del(&mut self, keys: &[String]) -> Result<i64> {
        if keys.is_empty() {
            return Err(Refusal::Arity("del").into());
        }
        let mut removed = 0;
        for key in keys {
            if self.lookup(key).is_some() {
                self.remove(key);
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// `DEL key` of one key, as a script calls it.
    fn del_one(&mut self, key: &str) {
        if self.lookup(key).is_some() {
            self.remove(key);
        }
    }

    fn exists(&mut self, key: &str) -> bool {
        self.lookup(key).is_some()
    }

    /// `INCRBY`, keeping the key's deadline.
    fn incr_by(&mut self, key: &str, by: i64) -> Result<i64> {
        let held = match self.text(key)? {
            None => 0,
            Some(text) => string2ll(text).ok_or(Refusal::NotInteger)?,
        };
        let integer = held.checked_add(by).ok_or(Refusal::Overflow)?;
        let deadline = self.entries.get(key).and_then(|entry| entry.deadline);
        self.put(
            key,
            Entry::new(key, Data::Text(integer.to_string()), deadline),
        )?;
        Ok(integer)
    }

    /// `EXPIRE` or `PEXPIRE`: a deadline not after now removes the key.
    fn expire(&mut self, key: &str, amount: i64, unit: Unit) -> Result<bool> {
        let command = match unit {
            Unit::Seconds => "expire",
            Unit::Millis => "pexpire",
        };
        let refused = || Error::from(Refusal::ExpireTime(command));
        let millis = match unit {
            Unit::Seconds => amount.checked_mul(1000).ok_or_else(refused)?,
            Unit::Millis => amount,
        };
        let deadline = millis.checked_add(self.now).ok_or_else(refused)?;
        if self.lookup(key).is_none() {
            return Ok(false);
        }
        if deadline <= self.now {
            self.remove(key);
        } else {
            let prior = self.set_deadline(key, Some(deadline));
            self.record(Undo::Deadline {
                key: key.to_string(),
                prior,
            });
        }
        Ok(true)
    }

    /// `TTL` (whole seconds, rounded to the nearest) or `PTTL`.
    fn ttl(&mut self, key: &str, unit: Unit) -> Ttl {
        let now = self.now;
        match self.lookup(key).map(|entry| entry.deadline) {
            None => Ttl::NoKey,
            Some(None) => Ttl::NoExpiry,
            Some(Some(deadline)) => {
                let left = (deadline - now).max(0).unsigned_abs();
                Ttl::Left(match unit {
                    Unit::Seconds => Duration::from_secs((left + 500) / 1000),
                    Unit::Millis => Duration::from_millis(left),
                })
            }
        }
    }

    /// `MGET`: what is not text reads as nothing.
    fn mget(&mut self, keys: &[String]) -> Result<Vec<Option<String>>> {
        if keys.is_empty() {
            return Err(Refusal::Arity("mget").into());
        }
        Ok(keys
            .iter()
            .map(|key| match self.lookup(key).map(|entry| &entry.data) {
                Some(Data::Text(text)) => Some(text.clone()),
                Some(Data::Set(_) | Data::Sorted(_) | Data::Hash(_)) | None => None,
            })
            .collect())
    }

    fn sadd(&mut self, key: &str, members: &[String]) -> Result<i64> {
        if members.is_empty() {
            return Err(Refusal::Arity("sadd").into());
        }
        let held = match self.lookup(key).map(|entry| &entry.data) {
            None => None,
            Some(Data::Set(held)) => Some(held),
            Some(Data::Text(_) | Data::Sorted(_) | Data::Hash(_)) => {
                return Err(Refusal::WrongType.into());
            }
        };
        let mut new: Vec<&String> = members
            .iter()
            .filter(|member| held.is_none_or(|held| !held.contains(*member)))
            .collect();
        new.sort();
        new.dedup();
        let grow = new.iter().map(|member| set_member_bytes(member)).sum();
        self.ready(key, grow, || Data::Set(HashSet::new()))?;
        for member in &new {
            self.set_member(key, member, true);
            self.record(Undo::Member {
                key: key.to_string(),
                member: member.to_string(),
            });
        }
        Ok(i64::try_from(new.len()).unwrap_or(i64::MAX))
    }

    /// `HSET` of each of `fields` in turn.
    fn hset(&mut self, key: &str, fields: &[(&str, String)]) -> Result<()> {
        let held = self.hash(key)?;
        let grow = fields
            .iter()
            .map(|(field, value)| {
                let before = held
                    .and_then(|held| held.get(*field))
                    .map_or(0, |prior| field_bytes(field, prior));
                field_bytes(field, value).saturating_sub(before)
            })
            .sum();
        self.ready(key, grow, || Data::Hash(HashMap::new()))?;
        for (field, value) in fields {
            let prior = self.set_field(key, field, Some(value.clone()));
            self.record(Undo::Field {
                key: key.to_string(),
                field: field.to_string(),
                prior,
            });
        }
        Ok(())
    }

    /// `ZADD` of each of `members` with its score, answering how many were not there.
    fn zadd(&mut self, key: &str, members: &[(f64, String)]) -> Result<i64> {
        if members.is_empty() {
            return Err(Refusal::Arity("zadd").into());
        }
        if members.iter().any(|(score, _)| score.is_nan()) {
            return Err(Refusal::NotFloat.into());
        }
        let held = self.sorted(key)?;
        let mut new: Vec<&String> = members
            .iter()
            .map(|(_, member)| member)
            .filter(|member| held.is_none_or(|held| !held.scores.contains_key(*member)))
            .collect();
        new.sort();
        new.dedup();
        let grow = new.iter().map(|member| sorted_member_bytes(member)).sum();
        self.ready(key, grow, || Data::Sorted(Sorted::default()))?;
        for (score, member) in members {
            self.set_score(key, member, Some(*score));
        }
        Ok(i64::try_from(new.len()).unwrap_or(i64::MAX))
    }

    /// `ZINCRBY`, answering the new score.
    fn zincrby(&mut self, key: &str, by: f64, member: &str) -> Result<f64> {
        if by.is_nan() {
            return Err(Refusal::NotFloat.into());
        }
        let held = self
            .sorted(key)?
            .and_then(|sorted| sorted.scores.get(member).copied());
        let score = held.unwrap_or(0.0) + by;
        if score.is_nan() {
            return Err(Refusal::NaN.into());
        }
        let grow = if held.is_some() {
            0
        } else {
            sorted_member_bytes(member)
        };
        self.ready(key, grow, || Data::Sorted(Sorted::default()))?;
        self.set_score(key, member, Some(score));
        Ok(score)
    }

    /// `ZREMRANGEBYSCORE`, answering how many it removed.
    fn zremrangebyscore(&mut self, key: &str, min: Bound, max: Bound) -> Result<i64> {
        let Some(sorted) = self.sorted(key)? else {
            return Ok(0);
        };
        let removed: Vec<String> = sorted
            .range(min, max)
            .map(|(_, member)| member.clone())
            .collect();
        for member in &removed {
            self.set_score(key, member, None);
        }
        self.drop_if_empty(key);
        Ok(i64::try_from(removed.len()).unwrap_or(i64::MAX))
    }

    /// `ZRANGEBYSCORE`, lowest first.
    fn zrangebyscore(&mut self, key: &str, min: Bound, max: Bound) -> Result<Vec<String>> {
        Ok(self.sorted(key)?.map_or_else(Vec::new, |sorted| {
            sorted
                .range(min, max)
                .map(|(_, member)| member.clone())
                .collect()
        }))
    }

    fn zcount(&mut self, key: &str, min: Bound, max: Bound) -> Result<usize> {
        Ok(self
            .sorted(key)?
            .map_or(0, |sorted| sorted.range(min, max).count()))
    }

    /// `ZUNIONSTORE destination` of `sources` with their scores summed (a set's members scored
    /// 1), answering how many members it holds; left empty, `destination` is removed. Valkey
    /// adds the sources smallest first, and so does this, those of one size in their order.
    fn zunionstore(&mut self, destination: &str, sources: &[&str]) -> Result<usize> {
        if sources.is_empty() {
            return Err(Refusal::NoInputKeys("zunionstore").into());
        }
        let mut read: Vec<Vec<(String, f64)>> = Vec::with_capacity(sources.len());
        for source in sources {
            read.push(match self.lookup(source).map(|entry| &entry.data) {
                None => Vec::new(),
                Some(Data::Sorted(sorted)) => sorted
                    .scores
                    .iter()
                    .map(|(member, score)| (member.clone(), *score))
                    .collect(),
                Some(Data::Set(members)) => {
                    members.iter().map(|member| (member.clone(), 1.0)).collect()
                }
                Some(Data::Text(_) | Data::Hash(_)) => return Err(Refusal::WrongType.into()),
            });
        }
        read.sort_by_key(Vec::len);
        let mut union = Sorted::default();
        for (member, score) in read.into_iter().flatten() {
            let sum = union.scores.get(&member).map_or(score, |held| held + score);
            // Valkey's convention for an infinity added to its opposite.
            union.put(&member, Some(if sum.is_nan() { 0.0 } else { sum }));
        }
        let members = union.scores.len();
        if members == 0 {
            self.remove(destination);
        } else {
            self.put(
                destination,
                Entry::new(destination, Data::Sorted(union), None),
            )?;
        }
        Ok(members)
    }

    /// `ZREVRANGE key start stop WITHSCORES`, ranks counted as Valkey counts them (from the end
    /// when below zero).
    fn zrevrange(&mut self, key: &str, start: i64, stop: i64) -> Result<Vec<(String, f64)>> {
        let Some(sorted) = self.sorted(key)? else {
            return Ok(Vec::new());
        };
        let length = i64::try_from(sorted.order.len()).unwrap_or(i64::MAX);
        let mut start = if start < 0 { length + start } else { start };
        let mut stop = if stop < 0 { length + stop } else { stop };
        start = start.max(0);
        if start > stop || start >= length {
            return Ok(Vec::new());
        }
        stop = stop.min(length - 1);
        Ok(sorted
            .order
            .iter()
            .rev()
            .skip(start.unsigned_abs() as usize)
            .take((stop - start + 1).unsigned_abs() as usize)
            .map(|(score, member)| (member.clone(), score.0))
            .collect())
    }

    /// One pipelined command.
    fn command(&mut self, command: Command) -> Result<Answer> {
        match command {
            Command::Set {
                key,
                value,
                expiry,
                condition,
                previous,
            } => {
                let (written, prior) = self.set(&key, value, expiry, condition, previous)?;
                Ok(match (previous, written) {
                    (true, _) => prior,
                    (false, true) => Answer::Text("OK".to_string()),
                    (false, false) => Answer::Nil,
                })
            }
            Command::Pttl(key) => Ok(Answer::Ttl(self.ttl(&key, Unit::Millis))),
            Command::Mget(keys) => self.mget(&keys).map(Answer::MaybeTexts),
            Command::Zadd { key, members } => self.zadd(&key, &members).map(Answer::Integer),
            Command::Expire { key, seconds } => self
                .expire(&key, seconds, Unit::Seconds)
                .map(|set| Answer::Integer(set.into())),
            Command::ZremRangeByScore { key, min, max } => self
                .zremrangebyscore(&key, Bound::included(min)?, Bound::included(max)?)
                .map(Answer::Integer),
            Command::ZrangeByScore { key, min, max } => self
                .zrangebyscore(&key, Bound::included(min)?, Bound::included(max)?)
                .map(Answer::Texts),
        }
    }

    // The scripts of `crate::valkey`, each step the call it makes there.

    /// `SET key <number> PX <number>` as a script calls it.
    fn set_px(&mut self, key: &str, value: f64, millis: f64) -> Result<()> {
        let millis = lua_integer_arg(millis)?;
        self.set(
            key,
            Value::Text(lua_text(value)),
            Some(Expiry::Millis(millis)),
            None,
            false,
        )?;
        Ok(())
    }

    /// `PEXPIRE key <number>` as a script calls it.
    fn pexpire(&mut self, key: &str, millis: f64) -> Result<bool> {
        let millis = lua_integer_arg(millis)?;
        self.expire(key, millis, Unit::Millis)
    }

    /// `GCRA` and `GCRA_PEEK`, answering the script's integer reply.
    fn gcra(&mut self, key: &str, rate: GcraRate, gcra: Gcra) -> Result<i64> {
        self.script(&[key], |state| {
            let now = state.lua_now;
            let emission = rate.emission_ms as f64;
            let tolerance = rate.tolerance_ms as f64;
            let tat = match state.text(key)? {
                Some(text) => tonumber(text),
                None => Some(now),
            };
            let Some(mut tat) = tat else {
                return Err(Refusal::Script("attempt to compare nil with number").into());
            };
            if tat < now {
                tat = now;
            }
            let allow_at = tat - tolerance;
            if now < allow_at {
                return Ok(lua_integer(allow_at - now));
            }
            if gcra == Gcra::Take {
                let next_tat = tat + emission;
                state.set_px(key, next_tat, next_tat - now)?;
            }
            Ok(0)
        })
    }

    /// `GCRA_REFUND`.
    fn gcra_refund(&mut self, key: &str, rate: GcraRate) -> Result<()> {
        self.script(&[key], |state| {
            let now = state.lua_now;
            let Some(tat) = state.text(key)?.and_then(|text| tonumber(text)) else {
                return Ok(());
            };
            let next_tat = tat - rate.emission_ms as f64;
            if next_tat <= now {
                state.del_one(key);
            } else {
                state.set_px(key, next_tat, next_tat - now)?;
            }
            Ok(())
        })
    }

    /// `COUNT_ATTEMPT`. The count passes through a Lua number, a double, on its way back.
    fn count_attempt(&mut self, key: &str, window_seconds: i64) -> Result<i64> {
        self.script(&[key], |state| {
            let count = state.incr_by(key, 1)? as f64;
            if count == 1.0 {
                state.expire(key, window_seconds, Unit::Seconds)?;
            }
            Ok(lua_integer(count))
        })
    }

    /// `ATTENTION_MARK`.
    fn attention_mark(&mut self, key: &str, member: &str, lasting: Duration) -> Result<()> {
        self.script(&[key], |state| {
            let now = state.lua_now;
            let lasting = lasting.as_millis() as f64;
            state.zremrangebyscore(
                key,
                Bound::included(f64::NEG_INFINITY)?,
                Bound::included(now)?,
            )?;
            state.zadd(key, &[(now + lasting, member.to_string())])?;
            state.pexpire(key, lasting)?;
            Ok(())
        })
    }

    /// `ATTENTION_ASK`. Lua writes `'(' .. now` with `%.14g`, which is `now`'s every digit until
    /// the year 5138.
    fn attention_ask(&mut self, keys: &[String]) -> Result<Vec<bool>> {
        let named: Vec<&str> = keys.iter().map(String::as_str).collect();
        self.script(&named, |state| {
            let now = state.lua_now;
            let mut found = Vec::with_capacity(keys.len());
            for key in keys {
                let count =
                    state.zcount(key, Bound::excluded(now)?, Bound::included(f64::INFINITY)?)?;
                found.push(lua_integer((count as f64).min(1.0)) > 0);
            }
            Ok(found)
        })
    }

    /// `BADGE_ADD`.
    fn badge_add(&mut self, key: &str) -> Result<Option<i64>> {
        self.script(&[key], |state| {
            if state.exists(key) {
                Ok(Some(lua_integer(state.incr_by(key, 1)? as f64)))
            } else {
                Ok(None)
            }
        })
    }

    /// A pool's `level` and `tick` as both mana scripts read them (`HMGET`, `tonumber`): full as
    /// of `tick` when either is missing, and earning `per` for each tick since.
    fn mana_level(&mut self, pool: &str, tick: f64, per: f64, max: f64) -> Result<f64> {
        let (level, last) = match self.hash(pool)? {
            None => (None, None),
            Some(fields) => (
                fields.get("level").and_then(|level| tonumber(level)),
                fields.get("tick").and_then(|last| tonumber(last)),
            ),
        };
        let (mut level, last) = match (level, last) {
            (Some(level), Some(last)) => (level, last),
            (None, _) | (_, None) => (max, tick),
        };
        if tick > last {
            level += (tick - last) * per;
        }
        Ok(level)
    }

    /// Both mana scripts' ending: a pool at `max` or above is removed, any other written and set
    /// to expire once it would be full again and a second more.
    fn mana_write(&mut self, pool: &str, level: f64, tick: f64, per: f64, max: f64) -> Result<()> {
        if level >= max {
            self.del_one(pool);
        } else {
            self.hset(
                pool,
                &[("level", lua_text(level)), ("tick", lua_text(tick))],
            )?;
            let ticks = ((max - level) / per).ceil();
            self.pexpire(pool, ticks * 200.0 + 1000.0)?;
        }
        Ok(())
    }

    /// `ZINCRBY sent <floor(cost / 1000)> sender` and `PEXPIRE sent kept`, as both mana scripts
    /// record what was spent.
    fn mana_sent(&mut self, sent: &str, cost: f64, sender: &str, kept_ms: f64) -> Result<()> {
        self.zincrby(sent, (cost / 1000.0).floor(), sender)?;
        self.pexpire(sent, kept_ms)?;
        Ok(())
    }

    /// `MANA_POOL`.
    fn mana_pool(
        &mut self,
        keys: &ManaKeys,
        rates: ManaRates,
        cost: i64,
        spend: ManaSpend,
        sender: Option<&str>,
        sent_kept_ms: i64,
    ) -> Result<ManaSpent> {
        self.script(&[&keys.pool, &keys.sent, &keys.credit], |state| {
            let now = state.lua_now;
            let tick = (now / 200.0).floor();
            let per = rates.per_tick as f64;
            let max = rates.max as f64;
            let cost = cost as f64;
            let floor = -(max / 2.0).floor();
            let mut level = state.mana_level(&keys.pool, tick, per, max)?;
            let mut held = 0.0;
            let credits: Vec<String> = state
                .hash(&keys.credit)?
                .map_or_else(Vec::new, |fields| fields.values().cloned().collect());
            for credit in credits {
                held += tonumber(&credit).ok_or(Refusal::Script(
                    "attempt to perform arithmetic on a nil value",
                ))?;
            }
            if level > max - held {
                level = max - held;
            }
            let mut spent = 0.0;
            let spends = match spend {
                ManaSpend::Peek => false,
                ManaSpend::Pay => level >= cost || level == max,
                ManaSpend::Bypass => true,
            };
            if spends {
                level = lua_max(level - cost, floor);
                spent = 1.0;
                let sender = sender.unwrap_or_default();
                if cost > 0.0 && !sender.is_empty() {
                    state.mana_sent(&keys.sent, cost, sender, sent_kept_ms as f64)?;
                }
            }
            state.mana_write(&keys.pool, level, tick, per, max)?;
            Ok(ManaSpent {
                spent: lua_integer(spent) == 1,
                level: lua_integer(level + held),
                at_ms: lua_integer(tick * 200.0),
            })
        })
    }

    /// `MANA_SHARE`.
    fn mana_allot(
        &mut self,
        keys: &ManaKeys,
        rates: ManaRates,
        allot: ManaAllot,
        sent_kept_ms: i64,
        turns: &[ManaTurn],
    ) -> Result<ManaAllotted> {
        self.script(&[&keys.pool, &keys.credit, &keys.sent], |state| {
            let now = state.lua_now;
            let tick = (now / 200.0).floor();
            let per = rates.per_tick as f64;
            let max = rates.max as f64;
            let floor = -(max / 2.0).floor();
            let mut level = state.mana_level(&keys.pool, tick, per, max)?;
            let mut credit: HashMap<String, f64> =
                state
                    .hash(&keys.credit)?
                    .map_or_else(HashMap::new, |fields| {
                        fields
                            .iter()
                            .filter_map(|(author, credit)| {
                                Some((author.clone(), tonumber(credit)?))
                            })
                            .collect()
                    });
            let costs: Vec<f64> = turns.iter().map(|turn| turn.cost as f64).collect();
            let weights: Vec<f64> = turns.iter().map(|turn| f64::from(turn.weight)).collect();
            let sum = |credit: &HashMap<String, f64>| credit.values().sum::<f64>();
            let payable = |credit: &HashMap<String, f64>, level: f64, i: usize| {
                let c = credit.get(&turns[i].author).copied().unwrap_or(0.0);
                c >= costs[i] || (level >= 0.0 && c >= (weights[i] * max / 3.0).floor())
            };
            let mut answer = Vec::with_capacity(turns.len());
            match allot {
                ManaAllot::Share => {
                    let top: HashSet<&str> =
                        turns.iter().map(|turn| turn.author.as_str()).collect();
                    credit.retain(|author, c| {
                        let kept = top.contains(author.as_str());
                        if !kept {
                            level += *c;
                        }
                        kept
                    });
                    let held = sum(&credit);
                    if level > max - held {
                        level = max - held;
                    }
                    if level > 0.0 && !turns.is_empty() {
                        let portion = (level / 3.0).floor();
                        for (i, turn) in turns.iter().enumerate() {
                            let mut add = weights[i] * portion;
                            if i == 0 {
                                add = add + level - 3.0 * portion;
                            }
                            *credit.entry(turn.author.clone()).or_insert(0.0) += add;
                        }
                        level = 0.0;
                    }
                    for i in 0..turns.len() {
                        answer.push(payable(&credit, level, i));
                    }
                }
                ManaAllot::Pay => {
                    let held = sum(&credit);
                    if level > max - held {
                        level = max - held;
                    }
                    let Some(turn) = turns.first() else {
                        return Err(Refusal::Script("attempt to compare number with nil").into());
                    };
                    let cost = costs[0];
                    let c = credit.get(&turn.author).copied().unwrap_or(0.0);
                    let mut spent = false;
                    if c >= cost {
                        credit.insert(turn.author.clone(), c - cost);
                        spent = true;
                    } else if payable(&credit, level, 0) {
                        level = lua_max(level - (cost - c), floor);
                        credit.insert(turn.author.clone(), 0.0);
                        spent = true;
                    }
                    if credit.get(&turn.author) == Some(&0.0) {
                        credit.remove(&turn.author);
                    }
                    if spent && cost > 0.0 {
                        state.mana_sent(&keys.sent, cost, &turn.author, sent_kept_ms as f64)?;
                    }
                    answer.push(spent);
                }
            }
            state.del_one(&keys.credit);
            let mut held = 0.0;
            let mut kept: Vec<(&str, String)> = Vec::new();
            for (author, c) in &credit {
                if *c > 0.0 {
                    kept.push((author, lua_text(*c)));
                    held += c;
                }
            }
            for (author, c) in kept {
                state.hset(&keys.credit, &[(author, c)])?;
            }
            if held > 0.0 {
                state.pexpire(&keys.credit, 60_000.0)?;
            }
            state.mana_write(&keys.pool, level, tick, per, max)?;
            Ok(ManaAllotted {
                level: lua_integer(level + held),
                at_ms: lua_integer(tick * 200.0),
                answers: answer,
            })
        })
    }

    /// `MANA_TOP`.
    fn mana_top(
        &mut self,
        cache: &str,
        minutes: &[String],
        count: usize,
        cached_for: Duration,
    ) -> Result<Vec<(String, f64)>> {
        let mut named = vec![cache];
        named.extend(minutes.iter().map(String::as_str));
        self.script(&named, |state| {
            if !state.exists(cache) && state.zunionstore(cache, &named[1..])? > 0 {
                state.pexpire(cache, cached_for.as_millis() as f64)?;
            }
            let stop = lua_integer_arg(count as f64 - 1.0)?;
            state.zrevrange(cache, 0, stop)
        })
    }
}
