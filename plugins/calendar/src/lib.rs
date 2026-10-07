//! An example Aspen plugin: an event calendar, a kind of channel (`calendar`) whose events the
//! plugin keeps in storage scoped to the channel, which its view (`views/calendar.html`) shows.
//!
//! - Adding an event (`POST calendars/{channel}/events`, `{title, start}`), for whoever may send
//!   messages in the channel, counts them as going, and, where the community's settings name an
//!   `announceChannel`, has the plugin's account post the event there as a card: when it is, how
//!   many are going, and a button to say you are going or not.
//! - Going or not (`POST calendars/{channel}/events/{event}/rsvp`, or the card's button, which
//!   reaches `aspen/cards/{message}/rsvp`) updates the card.
//! - Ten minutes before an event starts, a timer tells everyone going (`notify`), as Aspen tells
//!   them of a message that tags them.
//! - `POST calendars/{channel}/feed` gives the caller a private address of their own
//!   (`capability-path`), which a calendar app follows (`aspen/capabilities/feed:{channel}`) and
//!   reads as iCalendar, as that person may still see it.

wit_bindgen::generate!({
    path: "../../spec/plugin.wit",
    world: "plugin",
});

use aspen::plugin::host;
use aspen::plugin::types::{
    Audience, ButtonStyle, Card, CardButton, CardField, CardValue, Context, Observed, Request,
    Response, Scope, Text, Timer,
};
use exports::aspen::plugin::hooks::Guest;
use serde::{Deserialize, Serialize};
use std::hash::{BuildHasher, RandomState};
use std::time::{SystemTime, UNIX_EPOCH};

struct Calendar;

/// How long before an event starts its people are told.
const REMIND_BEFORE_MS: i64 = 10 * 60 * 1000;
const MAX_TITLE: usize = 200;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Event {
    id: String,
    title: String,
    /// When it starts, in milliseconds since the epoch.
    start: i64,
    creator: String,
    going: Vec<String>,
    /// Its card, where it was announced.
    #[serde(default)]
    card: Option<String>,
}

/// Where an announced event's card leads: its channel and event.
#[derive(Serialize, Deserialize)]
struct CardOf {
    channel: String,
    event: String,
}

#[derive(Deserialize)]
struct NewEvent {
    title: String,
    start: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct CommunitySettings {
    announce_channel: Option<String>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn new_id() -> String {
    let random = RandomState::new().hash_one(now_ms());
    format!("{:016x}", random)
}

// Dates, without a date library: days since 1970-01-01 to a civil date and back (Howard
// Hinnant's algorithms).

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

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

/// Milliseconds since the epoch of an RFC 3339 time (`2026-10-04T18:30:00Z`, or with an offset).
fn parse_time(text: &str) -> Option<i64> {
    let number = |s: &str| s.parse::<i64>().ok();
    let (date, rest) = text.split_once(['T', 't', ' '])?;
    let mut date = date.split('-');
    let (y, m, d) = (
        number(date.next()?)?,
        number(date.next()?)?,
        number(date.next()?)?,
    );
    let (clock, offset_minutes) = if let Some(clock) = rest.strip_suffix(['Z', 'z']) {
        (clock, 0)
    } else {
        let at = rest.rfind(['+', '-'])?;
        let (clock, offset) = rest.split_at(at);
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let (oh, om) = offset[1..].split_once(':')?;
        (clock, sign * (number(oh)? * 60 + number(om)?))
    };
    let mut clock = clock.split(':');
    let h = number(clock.next()?)?;
    let min = number(clock.next()?)?;
    let s: f64 = clock.next().unwrap_or("0").parse().ok()?;
    let seconds =
        days_from_civil(y, m, d) * 86_400 + h * 3600 + min * 60 + s as i64 - offset_minutes * 60;
    Some(seconds * 1000)
}

/// An RFC 3339 time in UTC.
fn format_time(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let (y, m, d) = civil_from_days(seconds.div_euclid(86_400));
    let s = seconds.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

/// An iCalendar time in UTC (`20261004T183000Z`).
fn format_ics(ms: i64) -> String {
    format_time(ms).replace(['-', ':'], "")
}

fn text(key: &str, args: &[(&str, &str)]) -> Text {
    Text {
        key: key.into(),
        args: args
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

fn json(status: u16, body: impl Serialize) -> Response {
    Response {
        status,
        content_type: Some("application/json".into()),
        body: serde_json::to_vec(&body).unwrap_or_default(),
    }
}

fn problem(status: u16, key: &str) -> Response {
    json(status, serde_json::json!({ "error": key }))
}

fn scope(channel: &str) -> Scope {
    Scope::Channel(channel.to_string())
}

fn load_event(channel: &str, id: &str) -> Option<Event> {
    match host::storage_get(&scope(channel), &format!("event:{id}")) {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).ok(),
        _ => None,
    }
}

fn save_event(channel: &str, event: &Event) -> bool {
    host::storage_set(
        &scope(channel),
        &format!("event:{}", event.id),
        &serde_json::to_vec(event).unwrap_or_default(),
    )
    .is_ok()
}

fn events(channel: &str) -> Option<Vec<Event>> {
    let page = host::storage_list(&scope(channel), "event:", None, 100).ok()?;
    let mut events: Vec<Event> = page
        .iter()
        .filter_map(|(_, v)| serde_json::from_slice(v).ok())
        .collect();
    events.sort_by_key(|e| e.start);
    Some(events)
}

fn card(event: &Event) -> Card {
    Card {
        title: Some(text("cardTitle", &[("title", &event.title)])),
        fields: vec![
            CardField {
                label: text("when", &[]),
                value: CardValue::Time(format_time(event.start)),
            },
            CardField {
                label: text("going", &[]),
                value: CardValue::Count(event.going.len() as u64),
            },
        ],
        buttons: vec![CardButton {
            id: "rsvp".into(),
            label: text("rsvp", &[]),
            style: ButtonStyle::Primary,
        }],
    }
}

fn changed(channel: &str) {
    let _ = host::publish(&Audience::Channel(channel.to_string()), "changed", "{}");
}

/// Counts `who` going to `event` or not, and updates its card.
fn toggle(channel: &str, mut event: Event, who: &str) -> Response {
    match event.going.iter().position(|g| g == who) {
        Some(at) => {
            event.going.remove(at);
        }
        None => event.going.push(who.to_string()),
    }
    if !save_event(channel, &event) {
        return problem(500, "failed");
    }
    if let Some(message) = &event.card {
        let _ = host::update_card(message, Some(&card(&event)));
    }
    changed(channel);
    json(200, &event)
}

/// The kind of channel a calendar is, as the manifest names it beneath the plugin's id.
const CALENDAR: &str = "org.aspenchat.calendar:calendar";

/// Whether `channel` is a calendar. Read as the caller, a channel they may not view is none.
fn is_calendar(channel: &str) -> bool {
    host::kind_of(channel).is_ok_and(|kind| kind.plugin_type.as_deref() == Some(CALENDAR))
}

fn route(request: &Request) -> Response {
    let parts: Vec<&str> = request.path.split('/').collect();
    let caller = request.caller.as_str();
    // Events are kept only on calendars, not on any other channel the plugin runs beside.
    if let ["calendars", channel, ..] = parts.as_slice()
        && !is_calendar(channel)
    {
        return problem(404, "notFound");
    }
    let may =
        |channel: &str, permission: &str| host::caller_may(channel, permission).unwrap_or(false);
    match (request.method.as_str(), parts.as_slice()) {
        ("GET", ["calendars", channel, "events"]) => match events(channel) {
            Some(events) => json(200, events),
            None => problem(404, "notFound"),
        },
        ("POST", ["calendars", channel, "events"]) => {
            if !may(channel, "sendMessages") {
                return problem(403, "cannotAdd");
            }
            let Ok(new) = serde_json::from_slice::<NewEvent>(&request.body) else {
                return problem(400, "failed");
            };
            let title = new.title.trim().to_string();
            let Some(start) = parse_time(&new.start) else {
                return problem(400, "failed");
            };
            if title.is_empty() || title.chars().count() > MAX_TITLE {
                return problem(400, "failed");
            }
            let mut event = Event {
                id: new_id(),
                title,
                start,
                creator: caller.to_string(),
                going: vec![caller.to_string()],
                card: None,
            };
            // A route is told of no community; the calendar's own is where it announces.
            let settings: CommunitySettings = host::place_of(channel)
                .ok()
                .and_then(|place| place.community)
                .and_then(|community| host::community_settings(&community).ok())
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            if let Some(announce) = settings.announce_channel
                && let Ok(Some(message)) = host::send_card(&announce, "", &card(&event))
            {
                let _ = host::storage_set(
                    &Scope::Deployment,
                    &format!("card:{message}"),
                    &serde_json::to_vec(&CardOf {
                        channel: channel.to_string(),
                        event: event.id.clone(),
                    })
                    .unwrap_or_default(),
                );
                event.card = Some(message);
            }
            if !save_event(channel, &event) {
                return problem(500, "failed");
            }
            let remind_at = (event.start - REMIND_BEFORE_MS).max(now_ms());
            if event.start > now_ms() {
                let _ = host::set_timer(
                    &format!("remind:{}", event.id),
                    &format_time(remind_at),
                    &serde_json::to_string(&CardOf {
                        channel: channel.to_string(),
                        event: event.id.clone(),
                    })
                    .unwrap_or_default(),
                );
            }
            changed(channel);
            json(201, &event)
        }
        ("POST", ["calendars", channel, "events", id, "rsvp"]) => {
            if !may(channel, "viewChannel") {
                return problem(404, "notFound");
            }
            match load_event(channel, id) {
                Some(event) => toggle(channel, event, caller),
                None => problem(404, "notFound"),
            }
        }
        // A card's button, pressed by someone who may read the card's message.
        ("POST", ["aspen", "cards", message, "rsvp"]) => {
            let Ok(Some(bytes)) = host::storage_get(&Scope::Deployment, &format!("card:{message}"))
            else {
                return problem(404, "notFound");
            };
            let Ok(of) = serde_json::from_slice::<CardOf>(&bytes) else {
                return problem(404, "notFound");
            };
            // Read as the presser: only someone who may view the calendar counts as going.
            match load_event(&of.channel, &of.event) {
                Some(event) => toggle(&of.channel, event, caller),
                None => problem(404, "notFound"),
            }
        }
        ("POST", ["calendars", channel, "feed"]) => {
            if !may(channel, "viewChannel") {
                return problem(404, "notFound");
            }
            match host::capability_path(&format!("feed:{channel}")) {
                Ok(path) => json(200, serde_json::json!({ "path": path })),
                Err(_) => problem(500, "failed"),
            }
        }
        ("GET", ["aspen", "capabilities", name]) => {
            let Some(channel) = name.strip_prefix("feed:").filter(|c| is_calendar(c)) else {
                return problem(404, "notFound");
            };
            let Some(events) = events(channel) else {
                return problem(404, "notFound");
            };
            Response {
                status: 200,
                content_type: Some("text/calendar; charset=utf-8".into()),
                body: ics(&events).into_bytes(),
            }
        }
        _ => problem(404, "notFound"),
    }
}

/// `events` as an iCalendar feed, each an hour long.
fn ics(events: &[Event]) -> String {
    let escape = |s: &str| {
        s.replace('\\', "\\\\")
            .replace(';', "\\;")
            .replace(',', "\\,")
            .replace('\n', "\\n")
    };
    let mut out =
        String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Aspen//Calendar plugin//EN\r\n");
    for event in events {
        out.push_str(&format!(
            "BEGIN:VEVENT\r\nUID:{}@aspen-calendar\r\nDTSTAMP:{}\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:{}\r\nEND:VEVENT\r\n",
            event.id,
            format_ics(now_ms()),
            format_ics(event.start),
            format_ics(event.start + 3_600_000),
            escape(&event.title),
        ));
    }
    out.push_str("END:VCALENDAR\r\n");
    out
}

/// A reminder fell due: everyone going is told, where their settings would tell them.
fn remind(timer: Timer) -> Result<(), String> {
    let of: CardOf = serde_json::from_str(&timer.payload).map_err(|e| e.to_string())?;
    let Some(event) = load_event(&of.channel, &of.event) else {
        return Ok(());
    };
    for who in &event.going {
        let _ = host::notify(
            who,
            &of.channel,
            &text("startingSoon", &[("title", &event.title)]),
            None,
        );
    }
    Ok(())
}

impl Guest for Calendar {
    fn intercept(
        _draft: aspen::plugin::types::Draft,
        _context: Context,
    ) -> aspen::plugin::types::Verdict {
        aspen::plugin::types::Verdict::Allow
    }

    fn observe(event: Observed, _context: Context) -> Result<(), String> {
        match event {
            Observed::TimerFired(timer) => remind(timer),
            _ => Ok(()),
        }
    }

    fn route(request: Request, _context: Context) -> Response {
        route(&request)
    }
}

export!(Calendar);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_go_both_ways() {
        let at = parse_time("2026-10-04T18:30:00Z").unwrap();
        assert_eq!(format_time(at), "2026-10-04T18:30:00Z");
        assert_eq!(parse_time("2026-10-04T20:30:00+02:00"), Some(at));
        assert_eq!(format_ics(at), "20261004T183000Z");
        assert_eq!(
            parse_time("2024-02-29T00:00:00Z").map(format_time).unwrap(),
            "2024-02-29T00:00:00Z"
        );
    }
}
