//! An example Aspen plugin: forum boards, a kind of channel (`board`) whose posts and replies the
//! plugin keeps in storage scoped to the channel, which its view (`views/board.html`) shows.
//!
//! Its routes, which the view calls through the app as the person using it:
//!
//! - `GET boards/{channel}/posts?before={post}&limit={n}`: a page of the board's posts, newest
//!   first, as summaries without their bodies (`limit` 1 to 100, 50 when absent), and `next`, the
//!   `before` of the page after it, or null at the oldest.
//! - `POST boards/{channel}/posts` (`{title, body}`): a new post, for whoever may send
//!   messages in the channel.
//! - `GET boards/{channel}/posts/{post}?after={reply}`: a post with its body and a page of its
//!   replies, oldest first, and `next`, the `after` of the page after it, or null at the newest.
//! - `POST boards/{channel}/posts/{post}/replies` (`{body}`): a reply.
//! - `DELETE boards/{channel}/posts/{post}`: deletes a post and its replies, for its author and
//!   whoever may manage messages.
//!
//! Each answers only for a channel that is a board (`kind-of`), and, reading as the caller, the
//! host answers nothing of a board they may not view. Each change is
//! published to the channel as the event `changed`, so every view of the board refreshes.
//!
//! A board's storage holds each post's summary under `summary:{newest-first key}` (`summary_key`),
//! so listing in key order reads newest first, its body under `body:{post}`, and each reply under
//! `reply:{post}:{reply}`. Every answer is a page whose size is bounded, however much a board
//! holds, so none comes near the host's limit on a route's answer.

wit_bindgen::generate!({
    path: "../../spec/plugin.wit",
    world: "plugin",
});

use aspen::plugin::host;
use aspen::plugin::types::{Audience, Context, Observed, Request, Response, Scope};
use exports::aspen::plugin::hooks::Guest;
use serde::{Deserialize, Serialize};
use std::hash::{BuildHasher, RandomState};
use std::time::{SystemTime, UNIX_EPOCH};

struct Forum;

const MAX_TITLE: usize = 200;
const MAX_BODY: usize = 20_000;
/// Posts on a page of the list, when the view does not say.
const DEFAULT_PAGE: usize = 50;
/// The most posts on a page of the list: the most one storage read returns.
const MAX_PAGE: usize = 100;
/// The bytes of replies one answer carries at most, beyond its first reply. With a post's body,
/// an answer stays well under the host's limit of 1 MiB.
const REPLY_BUDGET: usize = 512 << 10;
/// How long one request spends moving posts kept in the shape of version 1.1.0 into the present
/// one before it answers (`upgrade`).
const UPGRADE_BUDGET_MS: u64 = 1000;

/// A post as the list shows it: everything but its body.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Summary {
    id: String,
    author: String,
    title: String,
    created_at: u64,
    replies: u64,
}

/// A post with its body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Post {
    #[serde(flatten)]
    summary: Summary,
    body: String,
}

/// A post as version 1.1.0 kept it, under `post:{post}`, body and all.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyPost {
    id: String,
    author: String,
    title: String,
    body: String,
    created_at: u64,
    replies: u64,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Reply {
    id: String,
    author: String,
    body: String,
    created_at: u64,
}

#[derive(Deserialize)]
struct NewPost {
    title: String,
    body: String,
}

#[derive(Deserialize)]
struct NewReply {
    body: String,
}

/// Milliseconds since the epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The largest time an id holds: thirteen digits of milliseconds.
const MAX_TIME: u64 = 9_999_999_999_999;

/// An id that sorts by when it was made: the time, then something random.
fn new_id() -> String {
    let random = RandomState::new().hash_one(now());
    format!("{:013}{:016x}", now().min(MAX_TIME), random)
}

/// The time and the random part of an id `new_id` made, or none for anything else.
fn parse_id(id: &str) -> Option<(u64, u64)> {
    if id.len() != 29 || !id.is_ascii() {
        return None;
    }
    let (time, random) = id.split_at(13);
    if !time.bytes().all(|b| b.is_ascii_digit())
        || !random
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    Some((time.parse().ok()?, u64::from_str_radix(random, 16).ok()?))
}

/// The key of a post's summary: its id with both parts inverted, so that keys in order run from
/// the newest post to the oldest. None for something that is not an id.
fn summary_key(post: &str) -> Option<String> {
    let (time, random) = parse_id(post)?;
    Some(format!("summary:{:013}{:016x}", MAX_TIME - time, !random))
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

fn no_content() -> Response {
    Response {
        status: 204,
        content_type: None,
        body: Vec::new(),
    }
}

fn channel_scope(channel: &str) -> Scope {
    Scope::Channel(channel.to_string())
}

/// The bytes under `key`, or none when it is absent or cannot be read.
fn load_bytes(channel: &str, key: &str) -> Option<Vec<u8>> {
    host::storage_get(&channel_scope(channel), key)
        .ok()
        .flatten()
}

fn load<T: for<'de> Deserialize<'de>>(channel: &str, key: &str) -> Option<T> {
    serde_json::from_slice(&load_bytes(channel, key)?).ok()
}

fn save<T: Serialize>(channel: &str, key: &str, value: &T) -> Result<(), Response> {
    host::storage_set(
        &channel_scope(channel),
        key,
        &serde_json::to_vec(value).unwrap_or_default(),
    )
    .map_err(|_| problem(500, "failed"))
}

/// One read of the keys after `after` beginning with `prefix`, at most `limit`.
fn page(
    channel: &str,
    prefix: &str,
    after: Option<&str>,
    limit: usize,
) -> Result<Vec<(String, Vec<u8>)>, Response> {
    host::storage_list(&channel_scope(channel), prefix, after, limit as u32)
        .map_err(|_| problem(404, "notFound"))
}

/// Every key beginning with `prefix`.
fn keys(channel: &str, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    while let Ok(found) = page(channel, prefix, after.as_deref(), MAX_PAGE) {
        let full = found.len() == MAX_PAGE;
        out.extend(found.into_iter().map(|(key, _)| key));
        match out.last() {
            Some(last) if full => after = Some(last.clone()),
            _ => break,
        }
    }
    out
}

/// A query parameter's value.
fn query<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// Moves posts kept as version 1.1.0 kept them, whole under `post:{post}`, into a summary and a
/// body, for as long as `UPGRADE_BUDGET_MS` allows; a board left with more finishes on the
/// requests after. A board made by this version has none, and costs one read.
fn upgrade(channel: &str) {
    let scope = channel_scope(channel);
    let started = now();
    while now().saturating_sub(started) < UPGRADE_BUDGET_MS {
        let Ok(found) = page(channel, "post:", None, MAX_PAGE) else {
            return;
        };
        if found.is_empty() {
            return;
        }
        for (key, value) in found {
            if let Ok(old) = serde_json::from_slice::<LegacyPost>(&value)
                && let Some(summary) = summary_key(&old.id)
            {
                let moved = save(channel, &format!("body:{}", old.id), &old.body).and_then(|()| {
                    save(
                        channel,
                        &summary,
                        &Summary {
                            id: old.id,
                            author: old.author,
                            title: old.title,
                            created_at: old.created_at,
                            replies: old.replies,
                        },
                    )
                });
                if moved.is_err() {
                    return;
                }
            }
            if host::storage_delete(&scope, &key).is_err() {
                return;
            }
        }
    }
}

/// Adds `by` to a post's count of replies, reading again when another request changed the
/// summary between the read and the write.
fn count_replies(channel: &str, key: &str, by: i64) -> Result<(), Response> {
    let scope = channel_scope(channel);
    for _ in 0..8 {
        let Some(before) = load_bytes(channel, key) else {
            return Err(problem(404, "notFound"));
        };
        let Ok(mut summary) = serde_json::from_slice::<Summary>(&before) else {
            return Err(problem(500, "failed"));
        };
        summary.replies = summary.replies.saturating_add_signed(by);
        let after = serde_json::to_vec(&summary).unwrap_or_default();
        match host::storage_swap(&scope, key, Some(&before), Some(&after)) {
            Ok(true) => return Ok(()),
            Ok(false) => continue,
            Err(_) => return Err(problem(500, "failed")),
        }
    }
    Err(problem(503, "failed"))
}

fn changed(channel: &str) {
    let _ = host::publish(&Audience::Channel(channel.to_string()), "changed", "{}");
}

/// The kind of channel a board is, as the manifest names it beneath the plugin's id.
const BOARD: &str = "org.aspenchat.forum:board";

/// Whether `channel` is a board. Read as the caller, a channel they may not view is none.
fn is_board(channel: &str) -> bool {
    host::kind_of(channel).is_ok_and(|kind| kind.plugin_type.as_deref() == Some(BOARD))
}

fn may(channel: &str, permission: &str) -> bool {
    host::caller_may(channel, permission).unwrap_or(false)
}

/// A page of the board's posts, newest first, older than `before` when it is given.
fn list_posts(channel: &str, request: &Request) -> Response {
    let after = match query(request, "before") {
        Some(before) => match summary_key(before) {
            Some(key) => Some(key),
            None => return problem(400, "failed"),
        },
        None => None,
    };
    let limit = match query(request, "limit").map(str::parse::<usize>) {
        None => DEFAULT_PAGE,
        Some(Ok(limit)) if (1..=MAX_PAGE).contains(&limit) => limit,
        Some(_) => return problem(400, "failed"),
    };
    let found = match page(channel, "summary:", after.as_deref(), limit) {
        Ok(found) => found,
        Err(answer) => return answer,
    };
    let full = found.len() == limit;
    let posts: Vec<Summary> = found
        .iter()
        .filter_map(|(_, value)| serde_json::from_slice(value).ok())
        .collect();
    let next = if full {
        posts.last().map(|post| post.id.clone())
    } else {
        None
    };
    json(200, serde_json::json!({ "posts": posts, "next": next }))
}

/// A post with its body and a page of its replies, oldest first, newer than `after` when it is
/// given: as many as fit in `REPLY_BUDGET`, and always one when there is one.
fn show_post(channel: &str, post: &str, request: &Request) -> Response {
    let Some(summary) = summary_key(post).and_then(|key| load::<Summary>(channel, &key)) else {
        return problem(404, "notFound");
    };
    let body = load::<String>(channel, &format!("body:{post}")).unwrap_or_default();
    let prefix = format!("reply:{post}:");
    let after = match query(request, "after") {
        Some(reply) if parse_id(reply).is_some() => Some(format!("{prefix}{reply}")),
        Some(_) => return problem(400, "failed"),
        None => None,
    };
    let found = match page(channel, &prefix, after.as_deref(), MAX_PAGE) {
        Ok(found) => found,
        Err(answer) => return answer,
    };
    let full = found.len() == MAX_PAGE;
    let mut replies: Vec<Reply> = Vec::new();
    let mut bytes = 0;
    let mut cut = false;
    for (_, value) in &found {
        if !replies.is_empty() && bytes + value.len() > REPLY_BUDGET {
            cut = true;
            break;
        }
        if let Ok(reply) = serde_json::from_slice(value) {
            bytes += value.len();
            replies.push(reply);
        }
    }
    let next = if full || cut {
        replies.last().map(|reply| reply.id.clone())
    } else {
        None
    };
    json(
        200,
        serde_json::json!({
            "post": Post { summary, body },
            "replies": replies,
            "next": next,
        }),
    )
}

fn route(request: &Request) -> Response {
    let parts: Vec<&str> = request.path.split('/').collect();
    let caller = request.caller.clone();
    // Posts are kept only on boards, not on any other channel the plugin runs beside.
    if let ["boards", channel, ..] = parts.as_slice() {
        if !is_board(channel) {
            return problem(404, "notFound");
        }
        upgrade(channel);
    }
    match (request.method.as_str(), parts.as_slice()) {
        ("GET", ["boards", channel, "posts"]) => {
            if !may(channel, "viewChannel") {
                return problem(404, "notFound");
            }
            list_posts(channel, request)
        }
        ("POST", ["boards", channel, "posts"]) => {
            if !may(channel, "sendMessages") {
                return problem(403, "cannotPost");
            }
            let Ok(new) = serde_json::from_slice::<NewPost>(&request.body) else {
                return problem(400, "failed");
            };
            let title = new.title.trim().to_string();
            if title.is_empty()
                || title.chars().count() > MAX_TITLE
                || new.body.chars().count() > MAX_BODY
            {
                return problem(400, "failed");
            }
            let summary = Summary {
                id: new_id(),
                author: caller,
                title,
                created_at: now(),
                replies: 0,
            };
            let Some(key) = summary_key(&summary.id) else {
                return problem(500, "failed");
            };
            // The body first, so a summary the list shows always has one.
            if let Err(answer) = save(channel, &format!("body:{}", summary.id), &new.body)
                .and_then(|()| save(channel, &key, &summary))
            {
                return answer;
            }
            changed(channel);
            json(
                201,
                Post {
                    summary,
                    body: new.body,
                },
            )
        }
        ("GET", ["boards", channel, "posts", post]) => show_post(channel, post, request),
        ("POST", ["boards", channel, "posts", post, "replies"]) => {
            if !may(channel, "sendMessages") {
                return problem(403, "cannotPost");
            }
            let Some(key) = summary_key(post).filter(|key| load_bytes(channel, key).is_some())
            else {
                return problem(404, "notFound");
            };
            let Ok(new) = serde_json::from_slice::<NewReply>(&request.body) else {
                return problem(400, "failed");
            };
            if new.body.trim().is_empty() || new.body.chars().count() > MAX_BODY {
                return problem(400, "failed");
            }
            let reply = Reply {
                id: new_id(),
                author: caller,
                body: new.body,
                created_at: now(),
            };
            if let Err(answer) = save(channel, &format!("reply:{post}:{}", reply.id), &reply)
                .and_then(|()| count_replies(channel, &key, 1))
            {
                return answer;
            }
            changed(channel);
            json(201, reply)
        }
        ("DELETE", ["boards", channel, "posts", post]) => {
            let Some(key) = summary_key(post) else {
                return problem(404, "notFound");
            };
            let Some(found) = load::<Summary>(channel, &key) else {
                return problem(404, "notFound");
            };
            if found.author != caller && !may(channel, "manageMessages") {
                return problem(403, "failed");
            }
            let scope = channel_scope(channel);
            // The summary first, so the list stops showing the post before its parts go.
            if host::storage_delete(&scope, &key).is_err() {
                return problem(500, "failed");
            }
            let _ = host::storage_delete(&scope, &format!("body:{post}"));
            for reply in keys(channel, &format!("reply:{post}:")) {
                let _ = host::storage_delete(&scope, &reply);
            }
            changed(channel);
            no_content()
        }
        _ => problem(404, "notFound"),
    }
}

impl Guest for Forum {
    fn intercept(
        _draft: aspen::plugin::types::Draft,
        _context: Context,
    ) -> aspen::plugin::types::Verdict {
        aspen::plugin::types::Verdict::Allow
    }

    fn observe(_event: Observed, _context: Context) -> Result<(), String> {
        Ok(())
    }

    fn route(request: Request, _context: Context) -> Response {
        route(&request)
    }
}

export!(Forum);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_by_when_they_were_made() {
        let first = new_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert!(new_id() > first);
    }

    #[test]
    fn summaries_sort_newest_first() {
        let first = new_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = new_id();
        assert!(summary_key(&second).unwrap() < summary_key(&first).unwrap());
    }

    #[test]
    fn only_ids_have_summaries() {
        assert!(summary_key(&new_id()).is_some());
        for id in [
            "",
            "post",
            "1759000000000zzzzzzzzzzzzzzzz",
            "175900000000a0000000000000000",
        ] {
            assert!(summary_key(id).is_none(), "{id}");
        }
    }
}
