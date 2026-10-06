//! An example Aspen plugin: blackjack tables, a kind of channel (`table`) where members play
//! against the dealer for chips that are not money, which its view (`views/table.html`) shows.
//! The rules are in `game`.
//!
//! Its routes, which the view calls through the app as the person using it:
//!
//! - `GET tables/{channel}`: the table as everyone there sees it, and the caller's chips.
//! - `POST tables/{channel}/bet` (`{amount}`): a seat in the next round, for whoever may send
//!   messages in the channel.
//! - `DELETE tables/{channel}/bet`: the seat given up and the bet returned, before the deal.
//! - `POST tables/{channel}/ready`: deal without waiting out the betting, once everyone seated
//!   has said so.
//! - `POST tables/{channel}/actions` (`{action, version}`): `hit`, `stand`, `double`, or `split`
//!   on the hand whose turn it is, refused when the table has changed since `version`, so a tap
//!   that arrives twice counts once.
//!
//! Each answers `{table, chips}`, and each change is published to the channel as the event
//! `table`, carrying the table as everyone there sees it.
//!
//! The table is kept in storage scoped to its channel, so who may view the channel decides who
//! may watch, and deleting the channel deletes the table. It is written only with
//! `storage-swap`: a request and the table's timer, or two requests on two servers, that read the
//! same table cannot both write it, and the one that loses reads it again and decides again.
//!
//! A player's chips are kept in storage scoped to them, under `wallet:{community}`, so each
//! community has its own economy and deleting an account deletes them. A request may write only
//! its caller's wallet, so the transfers to and from anyone else (another player's payout, say)
//! are applied by the table's timer (`table:{channel}`), which runs as the plugin and is set for
//! whenever the table next needs attention: at once while transfers are pending or the dealer
//! is to play, and otherwise at the end of the betting or of a turn.

wit_bindgen::generate!({
    inline: r#"
        package aspen:blackjack;

        world blackjack {
            include aspen:plugin/plugin@1.0.0;
            import wasi:random/random@0.2.0;
        }
    "#,
    path: ["../../spec/plugin.wit", "wit/random.wit"],
    world: "aspen:blackjack/blackjack",
    generate_all,
});

mod game;

use aspen::plugin::host;
use aspen::plugin::types::{Audience, Context, Error, Level, Observed, Request, Response, Scope};
use exports::aspen::plugin::hooks::Guest;
use game::{Action, Random, Refusal, Table, TransferState, Wallet};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

struct Blackjack;

/// The table's key in its channel's storage.
const TABLE: &str = "table";
/// How many times a write is tried when another keeps changing what it read.
const ATTEMPTS: usize = 8;
/// How long past due a table may be before reading it sets its timer again.
const OVERDUE_MS: i64 = 5_000;

/// The host's secure randomness, which a shuffle needs: nobody may predict the cards.
struct Secure;

impl Random for Secure {
    fn below(&mut self, n: usize) -> usize {
        let n = n as u64;
        // Values below 2^64 mod n are drawn again, leaving a whole number of runs of n, so every
        // remainder is equally likely.
        let threshold = n.wrapping_neg() % n;
        loop {
            let x = wasi::random::random::get_random_u64();
            if x >= threshold {
                return (x % n) as usize;
            }
        }
    }
}

/// Milliseconds since the epoch.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The day, counted from the epoch in UTC.
fn today() -> i64 {
    now().div_euclid(86_400_000)
}

/// Year, month, and day of a day counted from the epoch (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// An RFC 3339 time in UTC, rounded up to the second, so a timer never falls due before `ms`.
fn format_time(ms: i64) -> String {
    let seconds = (ms + 999).div_euclid(1000);
    let (y, m, d) = civil_from_days(seconds.div_euclid(86_400));
    let s = seconds.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

/// Why a request was not done.
#[derive(Debug)]
enum Failure {
    Refused(Refusal),
    NotEnoughChips,
    CannotPlay,
    NotFound,
    Unavailable,
}

impl From<Refusal> for Failure {
    fn from(refusal: Refusal) -> Self {
        Failure::Refused(refusal)
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        match error {
            Error::NotFound | Error::Denied(_) => Failure::NotFound,
            other => {
                host::log(Level::Warn, &format!("a host call failed: {other:?}"));
                Failure::Unavailable
            }
        }
    }
}

impl Failure {
    fn response(self) -> Response {
        let (status, key) = match self {
            Failure::Refused(Refusal::BetRange) => (400, Refusal::BetRange.key()),
            Failure::Refused(refusal) => (409, refusal.key()),
            Failure::NotEnoughChips => (409, "notEnoughChips"),
            Failure::CannotPlay => (403, "cannotPlay"),
            Failure::NotFound => (404, "notFound"),
            Failure::Unavailable => (503, "failed"),
        };
        json(status, serde_json::json!({ "error": key }))
    }
}

fn json(status: u16, body: impl Serialize) -> Response {
    Response {
        status,
        content_type: Some("application/json".into()),
        body: serde_json::to_vec(&body).unwrap_or_default(),
    }
}

fn table_scope(channel: &str) -> Scope {
    Scope::Channel(channel.to_string())
}

fn load_table(channel: &str) -> Result<(Option<Vec<u8>>, Table), Failure> {
    let raw = host::storage_get(&table_scope(channel), TABLE)?;
    let table = match &raw {
        None => Table::default(),
        Some(bytes) => serde_json::from_slice(bytes).map_err(|e| {
            host::log(
                Level::Error,
                &format!("the table of {channel} is unreadable: {e}"),
            );
            Failure::Unavailable
        })?,
    };
    Ok((raw, table))
}

/// Changes the table with `change`, which is run again on the table as it then is whenever
/// another write came first, and answers what it answered with the table as written. A refusal
/// writes nothing.
fn change<T>(
    channel: &str,
    mut change: impl FnMut(&mut Table) -> Result<T, Refusal>,
) -> Result<(T, Table), Failure> {
    for _ in 0..ATTEMPTS {
        let (raw, mut table) = load_table(channel)?;
        let answer = change(&mut table)?;
        let bytes = serde_json::to_vec(&table).map_err(|_| Failure::Unavailable)?;
        if host::storage_swap(&table_scope(channel), TABLE, raw.as_deref(), Some(&bytes))? {
            return Ok((answer, table));
        }
    }
    Err(Failure::Unavailable)
}

fn wallet_key(community: &str) -> String {
    format!("wallet:{community}")
}

fn load_wallet(user: &str, community: &str) -> Result<(Option<Vec<u8>>, Wallet), Failure> {
    let raw = host::storage_get(&Scope::User(user.to_string()), &wallet_key(community))?;
    let mut wallet = raw
        .as_deref()
        .and_then(|bytes| serde_json::from_slice(bytes).ok())
        .unwrap_or_else(|| Wallet::new(today()));
    wallet.top_up(today());
    Ok((raw, wallet))
}

/// The caller's chips, without writing their wallet: a top-up due is counted, and written by
/// the next transfer.
fn chips(user: &str, community: &str) -> Result<i64, Failure> {
    Ok(load_wallet(user, community)?.1.chips)
}

/// Applies a transfer to a wallet, at most once whatever how often it is asked, answering
/// whether it was applied.
fn apply(user: &str, community: &str, id: &str, amount: i64) -> Result<bool, Failure> {
    let scope = Scope::User(user.to_string());
    for _ in 0..ATTEMPTS {
        let (raw, mut wallet) = load_wallet(user, community)?;
        let applied = wallet.apply(id, amount);
        let bytes = serde_json::to_vec(&wallet).map_err(|_| Failure::Unavailable)?;
        if host::storage_swap(&scope, &wallet_key(community), raw.as_deref(), Some(&bytes))? {
            return Ok(applied);
        }
    }
    Err(Failure::Unavailable)
}

/// Applies the table's pending transfers this call may write, the caller's alone (`only`) in a
/// request and everyone's in the timer, and records at the table what became of them. One that
/// fails stays pending, for the timer to finish.
fn settle(
    channel: &str,
    community: &str,
    table: Table,
    only: Option<&str>,
) -> Result<Table, Failure> {
    let mut outcomes = Vec::new();
    for transfer in table.pending() {
        if only.is_some_and(|user| user != transfer.user) {
            continue;
        }
        // Every table of the community writes to the same wallets, so the wallet knows each
        // transfer by its table's channel too.
        let id = format!("{channel}:{}", transfer.id);
        match apply(&transfer.user, community, &id, transfer.amount) {
            Ok(applied) => outcomes.push((transfer.id.clone(), applied)),
            Err(failure) => host::log(
                Level::Warn,
                &format!("transfer {id} is still pending: {failure:?}"),
            ),
        }
    }
    if outcomes.is_empty() {
        return Ok(table);
    }
    let now = now();
    match change(channel, |table| {
        let mut changed = false;
        for (id, applied) in &outcomes {
            changed |= table.resolve(id, *applied, now, &mut Secure);
        }
        if changed {
            Ok(())
        } else {
            Err(Refusal::NotNow)
        }
    }) {
        Ok(((), table)) => Ok(table),
        // Someone else recorded them first.
        Err(Failure::Refused(Refusal::NotNow)) => Ok(load_table(channel)?.1),
        Err(failure) => Err(failure),
    }
}

/// Sets the table's timer for when it next needs attention, or cancels it.
fn arm(channel: &str, table: &Table) {
    let key = format!("table:{channel}");
    let result = match table.next_due(now()) {
        Some(due) => host::set_timer(&key, &format_time(due), channel),
        None => host::cancel_timer(&key),
    };
    if let Err(error) = result {
        host::log(
            Level::Warn,
            &format!("the timer of {channel} was not set: {error:?}"),
        );
    }
}

/// Tells everyone watching the table how it now stands.
fn publish(channel: &str, table: &Table) {
    let payload = serde_json::to_string(&table.public()).unwrap_or_default();
    let _ = host::publish(&Audience::Channel(channel.to_string()), "table", &payload);
}

/// The community of the table in `channel`, which the caller may view, and, when they are to
/// play, may send messages in.
fn community_of(channel: &str, playing: bool) -> Result<String, Failure> {
    let place = host::place_of(channel)?;
    let community = place.community.ok_or(Failure::NotFound)?;
    if playing && !host::caller_may(channel, "sendMessages")? {
        return Err(Failure::CannotPlay);
    }
    Ok(community)
}

fn answer(table: &Table, chips: i64) -> Response {
    json(
        200,
        serde_json::json!({ "table": table.public(), "chips": chips }),
    )
}

/// Whether the transfer `id` was refused for want of chips.
fn refused(table: &Table, id: &str) -> bool {
    table
        .ledger
        .iter()
        .any(|t| t.id == id && t.state == TransferState::Refused)
}

#[derive(Deserialize)]
struct NewBet {
    amount: i64,
}

#[derive(Deserialize)]
struct Decision {
    action: Action,
    version: u64,
}

fn show(channel: &str, caller: &str) -> Result<Response, Failure> {
    let community = community_of(channel, false)?;
    let (_, mut table) = load_table(channel)?;
    if table.pending().any(|t| t.user == caller) {
        table = settle(channel, &community, table, Some(caller))?;
    }
    if table
        .next_due(now())
        .is_some_and(|due| due < now() - OVERDUE_MS)
    {
        arm(channel, &table);
    }
    Ok(answer(&table, chips(caller, &community)?))
}

/// Runs a player's change, applies the caller's transfers it made, and tells everyone.
fn play<T>(
    channel: &str,
    community: &str,
    caller: &str,
    change_table: impl FnMut(&mut Table) -> Result<T, Refusal>,
) -> Result<(T, Table), Failure> {
    let (answer, table) = change(channel, change_table)?;
    let table = settle(channel, community, table, Some(caller))?;
    arm(channel, &table);
    publish(channel, &table);
    Ok((answer, table))
}

fn bet(channel: &str, caller: &str, body: &[u8]) -> Result<Response, Failure> {
    let NewBet { amount } = serde_json::from_slice(body).map_err(|_| Refusal::BetRange)?;
    let community = community_of(channel, true)?;
    // Refused early, before anyone sees a seat taken and given up again.
    if chips(caller, &community)? < amount {
        return Err(Failure::NotEnoughChips);
    }
    let now = now();
    let (id, table) = play(channel, &community, caller, |t| t.bet(caller, amount, now))?;
    if refused(&table, &id) {
        return Err(Failure::NotEnoughChips);
    }
    Ok(answer(&table, chips(caller, &community)?))
}

fn leave(channel: &str, caller: &str) -> Result<Response, Failure> {
    let community = community_of(channel, true)?;
    let (_, table) = play(channel, &community, caller, |t| t.leave(caller))?;
    Ok(answer(&table, chips(caller, &community)?))
}

fn ready(channel: &str, caller: &str) -> Result<Response, Failure> {
    let community = community_of(channel, true)?;
    let ((), table) = play(channel, &community, caller, |t| t.ready(caller))?;
    Ok(answer(&table, chips(caller, &community)?))
}

fn act(channel: &str, caller: &str, body: &[u8]) -> Result<Response, Failure> {
    let Decision { action, version } = serde_json::from_slice(body).map_err(|_| Refusal::NotNow)?;
    let community = community_of(channel, true)?;
    let now = now();
    let (id, table) = play(channel, &community, caller, |t| {
        t.act(caller, version, action, now, &mut Secure)
    })?;
    if id.is_some_and(|id| refused(&table, &id)) {
        return Err(Failure::NotEnoughChips);
    }
    Ok(answer(&table, chips(caller, &community)?))
}

fn route(request: &Request) -> Response {
    let parts: Vec<&str> = request.path.split('/').collect();
    let caller = request.caller.as_str();
    let result = match (request.method.as_str(), parts.as_slice()) {
        ("GET", ["tables", channel]) => show(channel, caller),
        ("POST", ["tables", channel, "bet"]) => bet(channel, caller, &request.body),
        ("DELETE", ["tables", channel, "bet"]) => leave(channel, caller),
        ("POST", ["tables", channel, "ready"]) => ready(channel, caller),
        ("POST", ["tables", channel, "actions"]) => act(channel, caller, &request.body),
        _ => Err(Failure::NotFound),
    };
    result.unwrap_or_else(Failure::response)
}

/// The table's timer fell due: applies every pending transfer, then does whatever is due (the
/// deal, a turn run out, the dealer's hand), as many times as one leads to the next.
fn tend(channel: &str) -> Result<(), Failure> {
    let community = match host::place_of(channel) {
        Ok(place) => place.community.ok_or(Failure::NotFound)?,
        // The channel is gone, or the plugin no longer runs there: nothing is left to tend.
        Err(Error::NotFound) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let (_, mut table) = load_table(channel)?;
    let version = table.version;
    // A deal, a dealer's hand, and its payouts follow one another at most this many times.
    for _ in 0..4 {
        table = settle(channel, &community, table, None)?;
        let now = now();
        match change(channel, |t| {
            if t.step(now, &mut Secure) {
                Ok(())
            } else {
                Err(Refusal::NotNow)
            }
        }) {
            Ok(((), stepped)) => table = stepped,
            Err(Failure::Refused(Refusal::NotNow)) => break,
            Err(failure) => return Err(failure),
        }
    }
    arm(channel, &table);
    if table.version != version {
        publish(channel, &table);
    }
    Ok(())
}

impl Guest for Blackjack {
    fn intercept(
        _draft: aspen::plugin::types::Draft,
        _context: Context,
    ) -> aspen::plugin::types::Verdict {
        aspen::plugin::types::Verdict::Allow
    }

    fn observe(event: Observed, _context: Context) -> Result<(), String> {
        match event {
            Observed::TimerFired(timer) => tend(&timer.payload).map_err(|f| format!("{f:?}")),
            _ => Ok(()),
        }
    }

    fn route(request: Request, _context: Context) -> Response {
        route(&request)
    }
}

export!(Blackjack);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timers_never_fall_due_early() {
        assert_eq!(format_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_time(1), "1970-01-01T00:00:01Z");
        assert_eq!(format_time(1_791_400_000_000), "2026-10-07T19:06:40Z");
    }
}
