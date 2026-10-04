//! An example Aspen plugin: forum boards, a kind of channel (`board`) whose posts and replies the
//! plugin keeps in storage scoped to the channel, which its view (`views/board.html`) shows.
//!
//! Its routes, which the view calls through the app as the person using it:
//!
//! - `GET boards/{channel}/posts`: the board's posts, newest first.
//! - `POST boards/{channel}/posts` (`{title, body}`): a new post, for whoever may send
//!   messages in the channel.
//! - `GET boards/{channel}/posts/{post}`: a post with its replies, oldest first.
//! - `POST boards/{channel}/posts/{post}/replies` (`{body}`): a reply.
//! - `DELETE boards/{channel}/posts/{post}`: deletes a post and its replies, for its author and
//!   whoever may manage messages.
//!
//! Reading as the caller, the host answers nothing of a board they may not view. Each change is
//! published to the channel as the event `changed`, so every view of the board refreshes.

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

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Post {
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

/// An id that sorts by when it was made: the time, then something random.
fn new_id() -> String {
    let random = RandomState::new().hash_one(now());
    format!("{:013}{:016x}", now(), random)
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

fn channel_scope(channel: &str) -> Scope {
    Scope::Channel(channel.to_string())
}

fn load<T: for<'de> Deserialize<'de>>(channel: &str, key: &str) -> Option<T> {
    match host::storage_get(&channel_scope(channel), key) {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).ok(),
        _ => None,
    }
}

fn save<T: Serialize>(channel: &str, key: &str, value: &T) -> Result<(), Response> {
    host::storage_set(
        &channel_scope(channel),
        key,
        &serde_json::to_vec(value).unwrap_or_default(),
    )
    .map_err(|_| problem(500, "failed"))
}

fn list<T: for<'de> Deserialize<'de>>(channel: &str, prefix: &str) -> Result<Vec<T>, Response> {
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = host::storage_list(&channel_scope(channel), prefix, after.as_deref(), 100)
            .map_err(|_| problem(404, "notFound"))?;
        let last = page.last().map(|(key, _)| key.clone());
        out.extend(
            page.iter()
                .filter_map(|(_, v)| serde_json::from_slice(v).ok()),
        );
        match last {
            Some(last) if page.len() == 100 => after = Some(last),
            _ => return Ok(out),
        }
    }
}

fn changed(channel: &str) {
    let _ = host::publish(&Audience::Channel(channel.to_string()), "changed", "{}");
}

fn may(channel: &str, permission: &str) -> bool {
    host::caller_may(channel, permission).unwrap_or(false)
}

fn route(request: &Request) -> Response {
    let parts: Vec<&str> = request.path.split('/').collect();
    let caller = request.caller.clone();
    match (request.method.as_str(), parts.as_slice()) {
        ("GET", ["boards", channel, "posts"]) => {
            if !may(channel, "viewChannel") {
                return problem(404, "notFound");
            }
            match list::<Post>(channel, "post:") {
                Ok(mut posts) => {
                    posts.reverse();
                    json(200, posts)
                }
                Err(answer) => answer,
            }
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
            let post = Post {
                id: new_id(),
                author: caller,
                title,
                body: new.body,
                created_at: now(),
                replies: 0,
            };
            if let Err(answer) = save(channel, &format!("post:{}", post.id), &post) {
                return answer;
            }
            changed(channel);
            json(201, post)
        }
        ("GET", ["boards", channel, "posts", post]) => {
            let Some(found) = load::<Post>(channel, &format!("post:{post}")) else {
                return problem(404, "notFound");
            };
            match list::<Reply>(channel, &format!("reply:{post}:")) {
                Ok(replies) => json(
                    200,
                    serde_json::json!({ "post": found, "replies": replies }),
                ),
                Err(answer) => answer,
            }
        }
        ("POST", ["boards", channel, "posts", post, "replies"]) => {
            if !may(channel, "sendMessages") {
                return problem(403, "cannotPost");
            }
            let Some(mut found) = load::<Post>(channel, &format!("post:{post}")) else {
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
            if let Err(answer) = save(channel, &format!("reply:{post}:{}", reply.id), &reply) {
                return answer;
            }
            found.replies += 1;
            if let Err(answer) = save(channel, &format!("post:{post}"), &found) {
                return answer;
            }
            changed(channel);
            json(201, reply)
        }
        ("DELETE", ["boards", channel, "posts", post]) => {
            let Some(found) = load::<Post>(channel, &format!("post:{post}")) else {
                return problem(404, "notFound");
            };
            if found.author != caller && !may(channel, "manageMessages") {
                return problem(403, "failed");
            }
            let scope = channel_scope(channel);
            let replies = list::<Reply>(channel, &format!("reply:{post}:")).unwrap_or_default();
            for reply in replies {
                let _ = host::storage_delete(&scope, &format!("reply:{post}:{}", reply.id));
            }
            let _ = host::storage_delete(&scope, &format!("post:{post}"));
            changed(channel);
            Response {
                status: 204,
                content_type: None,
                body: Vec::new(),
            }
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
}
