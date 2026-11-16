//! Watching one ring (`spec/push.md`, Rings): a phone woken by a `call` push shows the call from
//! the push alone, and this is how it learns that the ring ended anywhere else, since nothing is
//! pushed to say so. The watch opens the deployment's event stream as the account (`identify`
//! with the session token, renewed through `Credentials` when the API server refuses it) and
//! reads it until one of these ends the ring, each reported once to the `RingObserver` and the
//! watch over:
//!
//! - a `voiceParticipant` `create` for the ring's session and the account's own user: answered on
//!   another device (the session, not the channel, since a first connection replays the stream's
//!   last minute, which may hold the user joining an earlier call of the same DM);
//! - a `voiceRing` `delete` for the ring's session and user: declined elsewhere (joining
//!   publishes the participant's `create` before it, so an answer is seen as one);
//! - a `voiceSession` `delete` for the session, or `voiceSessionEnded` naming it: the call ended;
//! - the ring's `until`, judged on the deployment's clock from `ready`'s `serverTime`:
//!   unanswered, as every client judges a ring's end. Until `ready` the device's own clock stands
//!   in, given at least `READY_TIMEOUT`, so a device whose clock runs ahead still asks the
//!   deployment before giving a ring up;
//! - no session to identify with, and none to be renewed: signed out.
//!
//! A socket that drops is opened again with `resumeAfter` the last sequence read, after a short
//! pause, so the frames meanwhile are replayed; a first connection replays the stream's last
//! minute, which covers a ring ended just before the push arrived. When a resume is refused
//! (`resumed: false`, the position having left the stream) the ring is read once over REST
//! (`GET /channels/{channel}/voice`), which tells a call gone or answered from one still ringing.
//! Everything runs on one task; the platform's `RingWatch` (`ring_bindings`) runs it on a thread
//! of its own, and `stop` ends it with no report, when this phone answered or declined.

use crate::api::{API_PREFIX, Credentials};
use crate::signalling::{InsecureAddress, signalling_url};
use crate::transport::{Http, HttpRequest, Received, SocketFactory};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::watch;
use tokio::time::Instant;
use uuid::Uuid;

/// How long a dropped stream, or one that could not be opened, waits before it is opened again.
/// A ring is short, so the pause is too; the API server's own admission control
/// (`serverBusy`, `rateLimited`) says when to wait longer.
pub const RECONNECT_PAUSE: Duration = Duration::from_millis(500);
/// How long the API server has to answer `identify` with `ready`: its own `IDENTIFY_TIMEOUT` is
/// ten seconds, so a connection that gave no `ready` by then will give none.
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// The ring a `call` push described (`spec/push.md`, The pointer).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Ring {
    pub channel: Uuid,
    pub session: Uuid,
    /// When the ring ends, RFC 3339, on the deployment's clock.
    pub until: String,
}

/// Why the ring ended, as the platform ends its incoming call: CallKit's `CXCallEndedReason`
/// and Telecom's `DisconnectCause` each have a word for every one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RingEnd {
    AnsweredElsewhere,
    Declined,
    /// The call itself ended: the caller hung up before anyone answered, or the session was
    /// ended.
    CallEnded,
    /// The ring's `until` passed, on the deployment's clock.
    TimedOut,
    /// The account has no session, and none could be renewed: the ring cannot be watched, and
    /// answering could not join.
    SignedOut,
}

/// Hears the ring's end, once, on the watch's thread; `stop` reports nothing.
pub trait RingObserver: Send + Sync {
    fn ended(&self, end: RingEnd);
}

/// The platform under a watch: the same HTTP and WebSockets a call runs on.
pub struct Platform {
    pub http: Arc<dyn Http>,
    pub sockets: Arc<dyn SocketFactory>,
}

pub struct Options {
    /// The deployment's `public_url`.
    pub api_url: String,
    pub credentials: Arc<dyn Credentials>,
    /// Whether a plain `http:` deployment at this machine's own addresses may be reached, as a
    /// development build allows.
    pub loopback_plain_text: bool,
}

/// The event stream's address: the deployment's `/api/v1/events` over the WebSocket scheme
/// matching its own, refused where a voice server's would be (`signalling_url`).
pub fn events_url(api_url: &str, loopback_plain_text: bool) -> Result<String, InsecureAddress> {
    // `signalling_url` checks the scheme and the host and answers the server's `/ws`, which is
    // the one path it writes; the stream lives beside it under the API prefix.
    let ws = signalling_url(api_url, loopback_plain_text)?;
    let base = ws.strip_suffix("/ws").unwrap_or(&ws);
    Ok(format!("{base}{API_PREFIX}/events"))
}

/// The milliseconds since the Unix epoch an RFC 3339 time names, written with `Z` or with a
/// numeric offset (`+01:00`); `None` for text that is not one. The deployment writes `Z`; the
/// offset form is RFC 3339's too, and read so that a server that wrote it is not taken for one
/// that wrote nothing.
pub fn unix_millis(rfc3339: &str) -> Option<i64> {
    let (without_offset, offset_millis) = split_offset(rfc3339)?;
    let at = humantime::parse_rfc3339_weak(without_offset).ok()?;
    let millis = match at.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok()?,
        Err(before) => -i64::try_from(before.duration().as_millis()).ok()?,
    };
    millis.checked_sub(offset_millis)
}

/// `rfc3339` with its numeric offset taken off, and that offset in milliseconds (`Z` and no
/// offset are zero); `None` for an offset that is not `[+-]HH:MM`.
fn split_offset(rfc3339: &str) -> Option<(&str, i64)> {
    let time_starts = rfc3339.find(['T', 't', ' '])?;
    let time = &rfc3339[time_starts..];
    let Some(sign_at) = time.rfind(['+', '-']) else {
        return Some((rfc3339, 0));
    };
    if time[..sign_at].ends_with(['Z', 'z']) {
        // `Z` and an offset both: not RFC 3339.
        return None;
    }
    let offset = &time[sign_at + 1..];
    let (hours, minutes) = offset.split_once(':')?;
    if hours.len() != 2 || minutes.len() != 2 {
        return None;
    }
    let hours: i64 = hours.parse().ok()?;
    let minutes: i64 = minutes.parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    let sign = if time.as_bytes()[sign_at] == b'-' {
        -1
    } else {
        1
    };
    let offset_millis = sign * (hours * 60 + minutes) * 60 * 1000;
    Some((&rfc3339[..time_starts + sign_at], offset_millis))
}

fn device_now_millis() -> i64 {
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

/// The frames the watch reads of the stream, leniently: what it does not know is `Other`.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Frame {
    #[serde(rename_all = "camelCase")]
    Ready {
        user_id: Uuid,
        resumed: bool,
        #[serde(default)]
        server_time: Option<String>,
    },
    Event {
        sequence: u64,
        event: Value,
    },
    #[serde(rename_all = "camelCase")]
    Error {
        code: String,
        #[serde(default)]
        retry_after_seconds: Option<u64>,
    },
    #[serde(other)]
    Other,
}

/// `GET /channels/{channel}/voice`, as far as the watch reads it.
#[derive(Deserialize)]
struct ChannelVoice {
    #[serde(default)]
    session: Option<SessionRef>,
    #[serde(default)]
    participants: Vec<ParticipantRef>,
}

#[derive(Deserialize)]
struct SessionRef {
    id: Uuid,
}

#[derive(Deserialize)]
struct ParticipantRef {
    user: Uuid,
}

/// Where one connection ended, and what the loop does next.
enum Connection {
    /// Open the stream again, resuming, after `pause`.
    Again {
        pause: Duration,
    },
    /// The server refused the token; `Watch::renewed` says whether a renewal is still to be
    /// tried.
    Refused,
    Ended(RingEnd),
}

struct Watch {
    platform: Platform,
    options: Options,
    ring: Ring,
    observer: Arc<dyn RingObserver>,
    /// The ring's end on this device's monotonic clock: from the device's own clock, and at least
    /// `READY_TIMEOUT` away, until `ready` tells the server's, which it then follows exactly.
    deadline: Instant,
    /// The ring's end as the deployment stamped it.
    until_millis: i64,
    /// The last `sequence` read, which the next connection resumes after.
    last_sequence: Option<u64>,
    /// The account's user, from `ready`.
    user: Option<Uuid>,
    /// Whether the token presented was renewed after a refusal and has not been taken since; a
    /// refusal of it is being signed out, rather than a renewal every half second.
    renewed: bool,
}

/// Runs the watch until the ring ends or `stop` says so.
pub async fn watch(
    platform: Platform,
    options: Options,
    ring: Ring,
    observer: Arc<dyn RingObserver>,
    mut stop: watch::Receiver<bool>,
) {
    let Some(until_millis) = unix_millis(&ring.until) else {
        // A ring whose end cannot be read is over as far as this phone can tell.
        observer.ended(RingEnd::TimedOut);
        return;
    };
    // By the device's clock, which may run ahead of the deployment's: a ring it says is over is
    // still given one connection's worth of time to hear the deployment's own clock, since the
    // push reached the phone only while the ring stood by that one.
    let remaining = Duration::from_millis(
        u64::try_from(until_millis - device_now_millis()).unwrap_or_default(),
    );
    let mut watch = Watch {
        platform,
        options,
        ring,
        observer,
        deadline: Instant::now() + remaining.max(READY_TIMEOUT),
        until_millis,
        last_sequence: None,
        user: None,
        renewed: false,
    };
    let ended = tokio::select! {
        end = watch.run() => Some(end),
        _ = stop.wait_for(|stopped| *stopped) => None,
    };
    if let Some(end) = ended {
        watch.observer.ended(end);
    }
}

impl Watch {
    async fn run(&mut self) -> RingEnd {
        loop {
            match self.connection().await {
                Connection::Ended(end) => return end,
                Connection::Refused => {
                    if self.renewed {
                        return RingEnd::SignedOut;
                    }
                    self.renewed = true;
                }
                Connection::Again { pause } => {
                    if Instant::now() + pause >= self.deadline {
                        return RingEnd::TimedOut;
                    }
                    tokio::time::sleep(pause).await;
                }
            }
        }
    }

    /// The session token to identify with: the one kept, or, after a refusal, one renewed from
    /// the refresh token.
    async fn token(&mut self) -> Option<String> {
        let credentials = &self.options.credentials;
        let kept = credentials.session_token().await?;
        if self.renewed {
            credentials.renewed_session_token(kept).await
        } else {
            Some(kept)
        }
    }

    /// One connection to the stream, from opening it to its close or the ring's end.
    async fn connection(&mut self) -> Connection {
        let Some(token) = self.token().await else {
            return Connection::Ended(RingEnd::SignedOut);
        };
        let url = match events_url(&self.options.api_url, self.options.loopback_plain_text) {
            Ok(url) => url,
            // Nothing can be watched at an address the engine refuses; the clock is all that is
            // left to end the ring by.
            Err(_) => return self.timed_out().await,
        };
        let opening = self.platform.sockets.connect(url, None);
        let socket = match tokio::time::timeout_at(self.deadline, opening).await {
            Ok(Ok(socket)) => socket,
            Ok(Err(_)) => {
                return Connection::Again {
                    pause: RECONNECT_PAUSE,
                };
            }
            Err(_) => return Connection::Ended(RingEnd::TimedOut),
        };
        let identify = serde_json::json!({
            "type": "identify",
            "sessionToken": token,
            "resumeAfter": self.last_sequence,
        });
        if socket.send_text(identify.to_string()).await.is_err() {
            return Connection::Again {
                pause: RECONNECT_PAUSE,
            };
        }
        let resuming = self.last_sequence.is_some();
        let mut ready_by = Some(Instant::now() + READY_TIMEOUT);
        loop {
            let received = match ready_by {
                Some(by) => {
                    match tokio::time::timeout_at(by.min(self.deadline), socket.receive()).await {
                        Ok(received) => received,
                        Err(_) if Instant::now() >= self.deadline => {
                            socket.close();
                            return Connection::Ended(RingEnd::TimedOut);
                        }
                        Err(_) => {
                            // No `ready` in time: the connection is dead, whatever the server
                            // thinks of it.
                            socket.close();
                            return Connection::Again {
                                pause: RECONNECT_PAUSE,
                            };
                        }
                    }
                }
                None => match tokio::time::timeout_at(self.deadline, socket.receive()).await {
                    Ok(received) => received,
                    Err(_) => {
                        socket.close();
                        return Connection::Ended(RingEnd::TimedOut);
                    }
                },
            };
            let text = match received {
                Received::Text(text) => text,
                Received::Closed => {
                    return Connection::Again {
                        pause: RECONNECT_PAUSE,
                    };
                }
            };
            let Ok(frame) = serde_json::from_str::<Frame>(&text) else {
                continue;
            };
            match frame {
                Frame::Ready {
                    user_id,
                    resumed,
                    server_time,
                } => {
                    ready_by = None;
                    // The token worked: a refusal later in the ring (a sign-in ending, a token
                    // expiring) is renewed once more.
                    self.renewed = false;
                    self.user = Some(user_id);
                    if let Some(server_now) = server_time.as_deref().and_then(unix_millis) {
                        // The ring ends when the deployment's clock says so, however far this
                        // device's is from it.
                        let remaining = self.until_millis - server_now;
                        self.deadline = Instant::now()
                            + Duration::from_millis(u64::try_from(remaining).unwrap_or_default());
                    }
                    if resuming && !resumed {
                        // The frames missed have left the stream; what they would have said is
                        // read from the channel itself.
                        if let Some(end) = self.read_channel().await {
                            socket.close();
                            return Connection::Ended(end);
                        }
                    }
                }
                Frame::Event { sequence, event } => {
                    self.last_sequence = Some(sequence);
                    if let Some(end) = self.ring_ended_by(&event) {
                        socket.close();
                        return Connection::Ended(end);
                    }
                }
                Frame::Error {
                    code,
                    retry_after_seconds,
                } => {
                    socket.close();
                    // A refusal that may pass is tried again after what the server asked, or the
                    // usual pause; the ring's end bounds the trying.
                    let again = Connection::Again {
                        pause: retry_after_seconds
                            .map_or(RECONNECT_PAUSE, Duration::from_secs)
                            .max(RECONNECT_PAUSE),
                    };
                    // The API server's `EventStreamErrorCode`s, each by name.
                    return match code.as_str() {
                        "unauthorized" => Connection::Refused,
                        // The account may not hold a stream at all; no renewal would change it.
                        "banned" | "twoFactorEnrollmentRequired" | "emailVerificationRequired" => {
                            Connection::Ended(RingEnd::SignedOut)
                        }
                        "badRequest"
                        | "identifyTimeout"
                        | "tooManyStreams"
                        | "tooManyStreamsFromAddress"
                        | "serverBusy"
                        | "rateLimited"
                        | "internal" => again,
                        // A code this build does not know: the safe answer is the one for a
                        // refusal that may pass, since the ring ends the trying either way.
                        _ => again,
                    };
                }
                Frame::Other => {}
            }
        }
    }

    /// Waits the ring out, for when nothing of it can be watched.
    async fn timed_out(&self) -> Connection {
        tokio::time::sleep_until(self.deadline).await;
        Connection::Ended(RingEnd::TimedOut)
    }

    /// What one event says of the ring: its end, when it is of this ring and ends it.
    fn ring_ended_by(&self, event: &Value) -> Option<RingEnd> {
        let field = |name: &str| {
            event
                .get(name)
                .and_then(Value::as_str)
                .and_then(|text| Uuid::parse_str(text).ok())
        };
        let change = event.get("type").and_then(Value::as_str);
        match event.get("serverEvent").and_then(Value::as_str)? {
            "voiceParticipant" if change == Some("create") => {
                (field("session") == Some(self.ring.session) && field("user") == self.user)
                    .then_some(RingEnd::AnsweredElsewhere)
            }
            "voiceRing" if change == Some("delete") => {
                (field("session") == Some(self.ring.session) && field("user") == self.user)
                    .then_some(RingEnd::Declined)
            }
            "voiceSession" if change == Some("delete") => {
                (field("id") == Some(self.ring.session)).then_some(RingEnd::CallEnded)
            }
            "voiceSessionEnded" => {
                (field("id") == Some(self.ring.session)).then_some(RingEnd::CallEnded)
            }
            _ => None,
        }
    }

    /// Reads the ring's channel once, for what a lost stretch of the stream would have said:
    /// `Some` when the ring is over, `None` when it still stands or the read failed, the watch
    /// then going on to the clock.
    async fn read_channel(&mut self) -> Option<RingEnd> {
        let token = self.options.credentials.session_token().await?;
        let mut response = self.get_channel_voice(&token).await?;
        if response.status == 401 {
            let renewed = self
                .options
                .credentials
                .renewed_session_token(token)
                .await?;
            response = self.get_channel_voice(&renewed).await?;
        }
        if !(200..300).contains(&response.status) {
            return None;
        }
        let voice: ChannelVoice = serde_json::from_str(&response.body).ok()?;
        match voice.session {
            Some(session) if session.id == self.ring.session => {
                let me = self.user?;
                voice
                    .participants
                    .iter()
                    .any(|p| p.user == me)
                    .then_some(RingEnd::AnsweredElsewhere)
            }
            _ => Some(RingEnd::CallEnded),
        }
    }

    async fn get_channel_voice(&self, token: &str) -> Option<crate::transport::HttpResponse> {
        self.platform
            .http
            .request(HttpRequest {
                method: "GET".to_string(),
                url: format!(
                    "{}{API_PREFIX}/channels/{}/voice",
                    self.options.api_url.trim_end_matches('/'),
                    self.ring.channel
                ),
                headers: vec![
                    ("Authorization".to_string(), format!("Bearer {token}")),
                    ("Accept".to_string(), "application/json".to_string()),
                ],
                body: None,
            })
            .await
            .ok()
    }
}

/// A watch on a thread of its own, as the platform layers hold it: `start` runs it, `stop` ends
/// it with no report (the ring was answered or declined on this phone), and dropping it stops it
/// and waits for the thread.
pub struct RingWatch {
    stop: watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RingWatch {
    /// Starts watching `ring`. `observer.ended` is called on the watch's thread, once at most.
    pub fn start(
        platform: Platform,
        options: Options,
        ring: Ring,
        observer: Arc<dyn RingObserver>,
    ) -> Self {
        let (stop, stopped) = watch::channel(false);
        let thread = std::thread::Builder::new()
            .name("aspen-ring".to_string())
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("the ring watch's runtime builds")
                    .block_on(watch(platform, options, ring, observer, stopped));
            })
            .expect("the ring watch's thread starts");
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Ends the watch without a report. Idempotent; the observer is not called after it returns
    /// unless its call had already begun.
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

impl Drop for RingWatch {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_events_url_lies_beside_the_api_and_follows_its_scheme() {
        assert_eq!(
            events_url("https://chat.example.org", false).as_deref(),
            Ok("wss://chat.example.org/api/v1/events")
        );
        assert_eq!(
            events_url("https://chat.example.org:8443/", false).as_deref(),
            Ok("wss://chat.example.org:8443/api/v1/events")
        );
        assert_eq!(
            events_url("http://127.0.0.1:8000", true).as_deref(),
            Ok("ws://127.0.0.1:8000/api/v1/events")
        );
        assert_eq!(
            events_url("http://chat.example.org", true),
            Err(InsecureAddress("http://chat.example.org".to_string()))
        );
    }

    #[test]
    fn rfc3339_is_read_to_the_millisecond_and_nothing_else_is() {
        assert_eq!(unix_millis("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(
            unix_millis("2026-11-15T12:00:30.250Z"),
            Some(1_794_744_030_250)
        );
        assert_eq!(
            unix_millis("2026-11-15T12:00:30.123456789Z"),
            Some(1_794_744_030_123)
        );
        for bad in ["", "soon", "2026-11-15", "12:00:30Z"] {
            assert_eq!(unix_millis(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_offset_names_the_same_instant_as_its_z_form() {
        let noon = unix_millis("2026-11-15T12:00:30Z").unwrap();
        assert_eq!(unix_millis("2026-11-15T13:00:30+01:00"), Some(noon));
        assert_eq!(
            unix_millis("2026-11-15T06:30:30.250-05:30"),
            Some(noon + 250)
        );
        assert_eq!(unix_millis("2026-11-15T12:00:30+00:00"), Some(noon));
        assert_eq!(unix_millis("2026-11-16T11:59:30+23:59"), Some(noon));
        // The date's own hyphens are not an offset.
        assert_eq!(unix_millis("2026-11-15T12:00:30Z"), Some(noon));
        for bad in [
            "2026-11-15T12:00:30+1:00",
            "2026-11-15T12:00:30+01",
            "2026-11-15T12:00:30+24:00",
            "2026-11-15T12:00:30+01:60",
            "2026-11-15T12:00:30+0a:00",
            "2026-11-15T12:00:30Z+01:00",
        ] {
            assert_eq!(unix_millis(bad), None, "{bad}");
        }
    }
}
