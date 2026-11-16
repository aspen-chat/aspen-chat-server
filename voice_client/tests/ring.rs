//! Scenarios of the ring watch against a scripted event stream and API server, on a paused clock.

use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;
use voice_client::api::Credentials;
use voice_client::ring::{
    Options, Platform, READY_TIMEOUT, RECONNECT_PAUSE, Ring, RingEnd, RingObserver, RingWatch,
};
use voice_client::transport::{
    Http, HttpRequest, HttpResponse, Received, Socket, SocketFactory, TransportError,
};

const API_URL: &str = "https://chat.example.org";

/// Lets every task the watch spawned run to its next wait.
async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

async fn advance(by: Duration) {
    tokio::time::advance(by).await;
    settle().await;
}

fn rfc3339(at: SystemTime) -> String {
    humantime::format_rfc3339_millis(at).to_string()
}

/// A ring ending `seconds` from now on this device's clock.
fn ring_for(seconds: u64) -> Ring {
    Ring {
        channel: Uuid::now_v7(),
        session: Uuid::now_v7(),
        until: rfc3339(SystemTime::now() + Duration::from_secs(seconds)),
    }
}

// --- The event stream -------------------------------------------------------------------------------

struct Connection {
    to_client: mpsc::UnboundedSender<Received>,
    closed: AtomicBool,
    /// The frames the client sent, as text.
    sent: Mutex<Vec<String>>,
}

/// How the API server takes the next connection.
#[derive(Clone, Copy)]
enum Behaviour {
    Accept,
    Refuse,
    /// Never answers the upgrade.
    Hang,
}

#[derive(Default)]
struct FakeStream {
    behaviours: Mutex<VecDeque<Behaviour>>,
    connections: Mutex<Vec<Arc<Connection>>>,
    urls: Mutex<Vec<String>>,
}

impl FakeStream {
    fn next_connection(&self, behaviour: Behaviour) {
        self.behaviours.lock().unwrap().push_back(behaviour);
    }

    fn connections(&self) -> usize {
        self.connections.lock().unwrap().len()
    }

    fn latest(&self) -> Arc<Connection> {
        self.connections
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a connection")
    }

    /// What the client sent on the latest connection, parsed.
    fn sent(&self) -> Vec<serde_json::Value> {
        self.latest()
            .sent
            .lock()
            .unwrap()
            .iter()
            .map(|text| serde_json::from_str(text).unwrap())
            .collect()
    }

    fn send(&self, frame: serde_json::Value) {
        let connection = self.latest();
        assert!(
            !connection.closed.load(Ordering::SeqCst),
            "the latest connection is closed"
        );
        let _ = connection.to_client.send(Received::Text(frame.to_string()));
    }

    fn ready(&self, user: Uuid, resumed: bool, server_time: Option<SystemTime>) {
        let mut ready = serde_json::json!({
            "type": "ready",
            "userId": user,
            "resumed": resumed,
            "replayComplete": true,
            "replayedSince": null,
            "catchUpFollows": true,
        });
        if let Some(at) = server_time {
            ready["serverTime"] = serde_json::json!(rfc3339(at));
        }
        self.send(ready);
    }

    fn event(&self, sequence: u64, event: serde_json::Value) {
        self.send(serde_json::json!({ "type": "event", "sequence": sequence, "event": event }));
    }

    /// Closes the latest connection from the server's side.
    fn close(&self) {
        let connection = self.latest();
        if !connection.closed.swap(true, Ordering::SeqCst) {
            let _ = connection.to_client.send(Received::Closed);
        }
    }

    fn is_closed(&self) -> bool {
        self.latest().closed.load(Ordering::SeqCst)
    }
}

struct FakeSocket {
    connection: Arc<Connection>,
    from_server: tokio::sync::Mutex<mpsc::UnboundedReceiver<Received>>,
}

#[async_trait]
impl Socket for FakeSocket {
    async fn send_text(&self, text: String) -> Result<(), TransportError> {
        if self.connection.closed.load(Ordering::SeqCst) {
            return Err(TransportError("closed".to_string()));
        }
        self.connection.sent.lock().unwrap().push(text);
        Ok(())
    }

    async fn receive(&self) -> Received {
        self.from_server
            .lock()
            .await
            .recv()
            .await
            .unwrap_or(Received::Closed)
    }

    fn close(&self) {
        if !self.connection.closed.swap(true, Ordering::SeqCst) {
            let _ = self.connection.to_client.send(Received::Closed);
        }
    }
}

#[async_trait]
impl SocketFactory for FakeStream {
    async fn connect(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> Result<Arc<dyn Socket>, TransportError> {
        assert_eq!(bearer, None, "the stream identifies in a frame");
        self.urls.lock().unwrap().push(url);
        let behaviour = self
            .behaviours
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Behaviour::Accept);
        match behaviour {
            Behaviour::Accept => {}
            Behaviour::Refuse => return Err(TransportError("refused".to_string())),
            Behaviour::Hang => std::future::pending().await,
        }
        let (to_client, from_server) = mpsc::unbounded_channel();
        let connection = Arc::new(Connection {
            to_client,
            closed: AtomicBool::new(false),
            sent: Mutex::new(Vec::new()),
        });
        self.connections.lock().unwrap().push(connection.clone());
        Ok(Arc::new(FakeSocket {
            connection,
            from_server: tokio::sync::Mutex::new(from_server),
        }))
    }
}

// --- The API server ---------------------------------------------------------------------------------

#[derive(Default)]
struct FakeHttp {
    /// Answers to `GET /channels/{channel}/voice`, in order; the last is repeated.
    answers: Mutex<Vec<HttpResponse>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl FakeHttp {
    fn answer(&self, status: u16, body: &str) {
        self.answers.lock().unwrap().push(HttpResponse {
            status,
            headers: Vec::new(),
            body: body.to_string(),
        });
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl Http for FakeHttp {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let served = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            requests.len()
        };
        let answers = self.answers.lock().unwrap();
        answers
            .get(served.min(answers.len()).saturating_sub(1))
            .cloned()
            .ok_or_else(|| TransportError("no answer scripted".to_string()))
    }
}

/// A session token, the one a renewal gives, and every token refused.
struct Token {
    session: Mutex<Option<String>>,
    renewed: Mutex<Option<String>>,
    rejected: Mutex<Vec<String>>,
}

impl Token {
    fn new(session: Option<&str>, renewed: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            session: Mutex::new(session.map(str::to_string)),
            renewed: Mutex::new(renewed.map(str::to_string)),
            rejected: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl Credentials for Token {
    async fn session_token(&self) -> Option<String> {
        self.session.lock().unwrap().clone()
    }

    /// Answers the renewal every time it is asked, as a platform with a refresh token does, so a
    /// watch that kept renewing would never run out of tokens.
    async fn renewed_session_token(&self, rejected: String) -> Option<String> {
        self.rejected.lock().unwrap().push(rejected);
        let renewed = self.renewed.lock().unwrap().clone();
        if let Some(renewed) = &renewed {
            *self.session.lock().unwrap() = Some(renewed.clone());
        }
        renewed
    }
}

// --- The observer and the world ---------------------------------------------------------------------

#[derive(Default)]
struct Ends(Mutex<Vec<RingEnd>>);

impl RingObserver for Ends {
    fn ended(&self, end: RingEnd) {
        self.0.lock().unwrap().push(end);
    }
}

struct World {
    stream: Arc<FakeStream>,
    http: Arc<FakeHttp>,
    ends: Arc<Ends>,
    stop: watch::Sender<bool>,
    ring: Ring,
    user: Uuid,
}

impl World {
    /// A watch of `ring` on the test's runtime, with the given credentials.
    fn watching(ring: Ring, token: Arc<Token>) -> Self {
        let stream = Arc::new(FakeStream::default());
        let http = Arc::new(FakeHttp::default());
        let ends = Arc::new(Ends::default());
        let (stop, stopped) = watch::channel(false);
        tokio::spawn(voice_client::ring::watch(
            Platform {
                http: http.clone(),
                sockets: stream.clone(),
            },
            Options {
                api_url: API_URL.to_string(),
                credentials: token.clone(),
                loopback_plain_text: false,
            },
            ring.clone(),
            ends.clone(),
            stopped,
        ));
        Self {
            stream,
            http,
            ends,
            stop,
            ring,
            user: Uuid::now_v7(),
        }
    }

    fn new(ring: Ring) -> Self {
        Self::watching(ring, Token::new(Some("session-token"), None))
    }

    fn ends(&self) -> Vec<RingEnd> {
        self.ends.0.lock().unwrap().clone()
    }

    /// Lets the watch connect and identify, and answers `ready` as a first connection.
    async fn admitted(&self) {
        settle().await;
        self.stream.ready(self.user, false, None);
        settle().await;
    }

    fn participant_joined(&self, user: Uuid, session: Uuid) -> serde_json::Value {
        serde_json::json!({
            "serverEvent": "voiceParticipant", "type": "create",
            "session": session, "user": user, "channel": self.ring.channel,
            "joinedAt": "2026-11-15T12:00:00Z", "muted": false, "deafened": false,
            "sharingScreen": false, "held": false,
        })
    }

    fn ring_deleted(&self, session: Uuid, user: Uuid) -> serde_json::Value {
        serde_json::json!({ "serverEvent": "voiceRing", "type": "delete", "session": session, "user": user })
    }
}

fn identify(frame: &serde_json::Value) -> (&str, Option<u64>) {
    assert_eq!(frame["type"], "identify");
    (
        frame["sessionToken"].as_str().unwrap(),
        frame["resumeAfter"].as_u64(),
    )
}

// --- Scenarios --------------------------------------------------------------------------------------

#[test]
fn the_timeouts_are_as_documented() {
    assert_eq!(RECONNECT_PAUSE, Duration::from_millis(500));
    assert_eq!(READY_TIMEOUT, Duration::from_secs(10));
}

#[tokio::test(start_paused = true)]
async fn the_stream_is_opened_beside_the_api_and_identified_with_the_session() {
    let w = World::new(ring_for(30));
    settle().await;
    assert_eq!(
        *w.stream.urls.lock().unwrap(),
        ["wss://chat.example.org/api/v1/events"]
    );
    let sent = w.stream.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(identify(&sent[0]), ("session-token", None));
    assert!(w.ends().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_participant_create_for_the_user_in_the_session_is_answered_elsewhere() {
    let w = World::new(ring_for(30));
    w.admitted().await;
    // Someone else joining, the user joining another call of the same DM (an earlier one, which
    // a first connection's replay of the last minute may hold), and a participant record of the
    // user's that is no joining end nothing.
    w.stream
        .event(1, w.participant_joined(Uuid::now_v7(), w.ring.session));
    w.stream
        .event(2, w.participant_joined(w.user, Uuid::now_v7()));
    let mut left = w.participant_joined(w.user, w.ring.session);
    left["type"] = serde_json::json!("delete");
    w.stream.event(3, left);
    settle().await;
    assert!(w.ends().is_empty());
    w.stream
        .event(4, w.participant_joined(w.user, w.ring.session));
    settle().await;
    assert_eq!(w.ends(), [RingEnd::AnsweredElsewhere]);
    assert!(w.stream.is_closed(), "the watch lets the stream go");
    // A delete after it is not a second end.
    advance(Duration::from_secs(31)).await;
    assert_eq!(w.ends(), [RingEnd::AnsweredElsewhere]);
}

#[tokio::test(start_paused = true)]
async fn a_ring_delete_for_the_user_is_declined_and_for_anyone_else_is_nothing() {
    let w = World::new(ring_for(30));
    w.admitted().await;
    w.stream
        .event(1, w.ring_deleted(w.ring.session, Uuid::now_v7()));
    w.stream.event(2, w.ring_deleted(Uuid::now_v7(), w.user));
    settle().await;
    assert!(w.ends().is_empty());
    w.stream.event(3, w.ring_deleted(w.ring.session, w.user));
    settle().await;
    assert_eq!(w.ends(), [RingEnd::Declined]);
}

#[tokio::test(start_paused = true)]
async fn the_sessions_end_ends_the_ring_by_either_event() {
    for event in [
        serde_json::json!({ "serverEvent": "voiceSession", "type": "delete", "id": "SESSION" }),
        serde_json::json!({ "serverEvent": "voiceSessionEnded", "id": "SESSION", "channel": "CHANNEL", "reason": "empty" }),
    ] {
        let w = World::new(ring_for(30));
        w.admitted().await;
        let other = event
            .to_string()
            .replace("SESSION", &Uuid::now_v7().to_string());
        w.stream.event(1, serde_json::from_str(&other).unwrap());
        settle().await;
        assert!(w.ends().is_empty(), "{event}");
        let ours = event
            .to_string()
            .replace("SESSION", &w.ring.session.to_string())
            .replace("CHANNEL", &w.ring.channel.to_string());
        w.stream.event(2, serde_json::from_str(&ours).unwrap());
        settle().await;
        assert_eq!(w.ends(), [RingEnd::CallEnded], "{event}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_ring_times_out_by_the_servers_clock_not_the_devices() {
    // The device's clock runs ten seconds behind the server's: the ring, thirty seconds off by
    // the device, is twenty by the server.
    let w = World::new(ring_for(30));
    settle().await;
    w.stream.ready(
        w.user,
        false,
        Some(SystemTime::now() + Duration::from_secs(10)),
    );
    settle().await;
    advance(Duration::from_secs(19)).await;
    assert!(w.ends().is_empty());
    advance(Duration::from_millis(1100)).await;
    assert_eq!(w.ends(), [RingEnd::TimedOut]);
    assert!(w.stream.is_closed());
}

#[tokio::test(start_paused = true)]
async fn before_ready_the_devices_clock_stands_in_given_at_least_the_ready_timeout() {
    // A ring of twenty seconds by the device, never answered: it ends by that clock.
    let w = World::new(ring_for(20));
    w.stream.next_connection(Behaviour::Hang);
    settle().await;
    advance(Duration::from_millis(19900)).await;
    assert!(w.ends().is_empty());
    advance(Duration::from_millis(200)).await;
    assert_eq!(w.ends(), [RingEnd::TimedOut]);

    // A ring of five by the device is given the ready timeout's ten before it is given up: the
    // device's clock may run ahead of the deployment's.
    let w = World::new(ring_for(5));
    w.stream.next_connection(Behaviour::Hang);
    settle().await;
    advance(Duration::from_millis(9900)).await;
    assert!(w.ends().is_empty());
    advance(Duration::from_millis(200)).await;
    assert_eq!(w.ends(), [RingEnd::TimedOut]);
}

#[tokio::test(start_paused = true)]
async fn a_ring_over_by_the_devices_clock_still_asks_the_deployment_and_rings_by_its_clock() {
    // The device's clock runs a minute ahead: the ring, thirty seconds off by the server, is
    // thirty seconds gone by the device.
    let w = World::new(Ring {
        channel: Uuid::now_v7(),
        session: Uuid::now_v7(),
        until: rfc3339(SystemTime::now() - Duration::from_secs(30)),
    });
    settle().await;
    assert_eq!(
        w.stream.connections(),
        1,
        "the stream is opened all the same"
    );
    assert!(w.ends().is_empty());
    w.stream.ready(
        w.user,
        false,
        Some(SystemTime::now() - Duration::from_secs(60)),
    );
    settle().await;
    advance(Duration::from_secs(29)).await;
    assert!(w.ends().is_empty(), "the ring rings by the server's clock");
    advance(Duration::from_millis(1100)).await;
    assert_eq!(w.ends(), [RingEnd::TimedOut]);

    // One the server too says is over ends as soon as `ready` says so.
    let w = World::new(Ring {
        channel: Uuid::now_v7(),
        session: Uuid::now_v7(),
        until: rfc3339(SystemTime::now() - Duration::from_secs(30)),
    });
    settle().await;
    w.stream.ready(w.user, false, Some(SystemTime::now()));
    settle().await;
    assert_eq!(w.ends(), [RingEnd::TimedOut]);
}

#[tokio::test(start_paused = true)]
async fn a_ring_whose_end_cannot_be_read_is_over() {
    let w = World::new(Ring {
        channel: Uuid::now_v7(),
        session: Uuid::now_v7(),
        until: "soon".to_string(),
    });
    settle().await;
    assert_eq!(w.ends(), [RingEnd::TimedOut]);
    assert_eq!(w.stream.connections(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_stream_is_resumed_after_the_pause_from_the_last_sequence() {
    let w = World::new(ring_for(30));
    w.admitted().await;
    w.stream
        .event(7, w.ring_deleted(Uuid::now_v7(), Uuid::now_v7()));
    settle().await;
    w.stream.close();
    settle().await;
    assert_eq!(w.stream.connections(), 1, "not before the pause");
    advance(RECONNECT_PAUSE).await;
    assert_eq!(w.stream.connections(), 2);
    let sent = w.stream.sent();
    assert_eq!(identify(&sent[0]), ("session-token", Some(7)));
    w.stream.ready(w.user, true, None);
    w.stream.event(8, w.ring_deleted(w.ring.session, w.user));
    settle().await;
    assert_eq!(w.ends(), [RingEnd::Declined]);
}

#[tokio::test(start_paused = true)]
async fn a_connection_refused_is_tried_again_until_the_ring_ends() {
    // A ring of twelve seconds by the device (past the ready timeout's floor), refused every
    // time: tried every half second, twenty-four times.
    let w = World::new(ring_for(12));
    for _ in 0..40 {
        w.stream.next_connection(Behaviour::Refuse);
    }
    settle().await;
    assert_eq!(w.stream.urls.lock().unwrap().len(), 1);
    advance(RECONNECT_PAUSE).await;
    assert_eq!(w.stream.urls.lock().unwrap().len(), 2);
    advance(RECONNECT_PAUSE).await;
    assert_eq!(w.stream.urls.lock().unwrap().len(), 3);
    for _ in 0..20 {
        advance(RECONNECT_PAUSE).await;
    }
    assert_eq!(w.stream.urls.lock().unwrap().len(), 23);
    assert!(w.ends().is_empty());
    advance(RECONNECT_PAUSE).await;
    // The twenty-fourth refusal, half a second before the ring's end: the ring is over as far
    // as this phone is concerned, with no half-second wait for nothing.
    assert_eq!(w.stream.urls.lock().unwrap().len(), 24);
    assert_eq!(w.ends(), [RingEnd::TimedOut]);
    advance(Duration::from_secs(2)).await;
    assert_eq!(
        w.stream.urls.lock().unwrap().len(),
        24,
        "nothing more is tried"
    );
}

#[tokio::test(start_paused = true)]
async fn an_upgrade_that_hangs_ends_with_the_ring() {
    let w = World::new(ring_for(12));
    w.stream.next_connection(Behaviour::Hang);
    settle().await;
    advance(Duration::from_millis(11900)).await;
    assert!(w.ends().is_empty());
    advance(Duration::from_millis(200)).await;
    assert_eq!(w.ends(), [RingEnd::TimedOut]);
}

#[tokio::test(start_paused = true)]
async fn no_ready_in_time_opens_the_stream_again() {
    let w = World::new(ring_for(30));
    settle().await;
    advance(READY_TIMEOUT).await;
    assert!(w.stream.connections() >= 1 && w.stream.latest().closed.load(Ordering::SeqCst));
    advance(RECONNECT_PAUSE).await;
    assert_eq!(w.stream.connections(), 2);
    w.stream.ready(w.user, false, None);
    w.stream.event(1, w.ring_deleted(w.ring.session, w.user));
    settle().await;
    assert_eq!(w.ends(), [RingEnd::Declined]);
}

#[tokio::test(start_paused = true)]
async fn a_refused_token_is_renewed_once_and_a_second_refusal_is_signed_out() {
    let token = Token::new(Some("stale"), Some("fresh"));
    let w = World::watching(ring_for(30), token.clone());
    settle().await;
    assert_eq!(identify(&w.stream.sent()[0]), ("stale", None));
    w.stream
        .send(serde_json::json!({ "type": "error", "code": "unauthorized", "detail": "Expired." }));
    settle().await;
    assert_eq!(
        w.stream.connections(),
        2,
        "opened again at once with the renewed token"
    );
    assert_eq!(identify(&w.stream.sent()[0]), ("fresh", None));
    assert_eq!(*token.rejected.lock().unwrap(), ["stale"]);
    w.stream
        .send(serde_json::json!({ "type": "error", "code": "unauthorized", "detail": "Expired." }));
    settle().await;
    assert_eq!(w.ends(), [RingEnd::SignedOut]);
    assert_eq!(
        w.stream.connections(),
        2,
        "a renewed token refused is not renewed again"
    );
    assert_eq!(
        *token.rejected.lock().unwrap(),
        ["stale"],
        "the token renewed and refused is given up, not renewed again"
    );
}

#[tokio::test(start_paused = true)]
async fn no_session_at_all_is_signed_out() {
    let w = World::watching(ring_for(30), Token::new(None, None));
    settle().await;
    assert_eq!(w.ends(), [RingEnd::SignedOut]);
    assert_eq!(w.stream.connections(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_banned_account_is_signed_out_and_a_busy_server_is_waited_for() {
    let w = World::new(ring_for(30));
    settle().await;
    w.stream.send(serde_json::json!({
        "type": "error", "code": "serverBusy", "detail": "Busy.", "retryAfterSeconds": 3
    }));
    settle().await;
    advance(Duration::from_millis(2900)).await;
    assert_eq!(w.stream.connections(), 1);
    advance(Duration::from_millis(200)).await;
    assert_eq!(w.stream.connections(), 2);
    w.stream
        .send(serde_json::json!({ "type": "error", "code": "banned", "detail": "Banned." }));
    settle().await;
    assert_eq!(w.ends(), [RingEnd::SignedOut]);
}

#[tokio::test(start_paused = true)]
async fn every_refusal_that_may_pass_is_tried_again_by_name_and_so_is_one_unknown() {
    for code in [
        "badRequest",
        "identifyTimeout",
        "tooManyStreams",
        "tooManyStreamsFromAddress",
        "serverBusy",
        "rateLimited",
        "internal",
        "someCodeOfALaterServer",
    ] {
        let w = World::new(ring_for(30));
        settle().await;
        w.stream
            .send(serde_json::json!({ "type": "error", "code": code, "detail": "No." }));
        settle().await;
        assert_eq!(w.stream.connections(), 1, "{code}: not before the pause");
        assert!(w.stream.is_closed(), "{code}");
        advance(RECONNECT_PAUSE).await;
        assert_eq!(w.stream.connections(), 2, "{code}: opened again");
        assert!(w.ends().is_empty(), "{code}");
        w.stream.ready(w.user, false, None);
        w.stream.event(1, w.ring_deleted(w.ring.session, w.user));
        settle().await;
        assert_eq!(w.ends(), [RingEnd::Declined], "{code}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_resume_refused_reads_the_channel_and_a_call_gone_ends_the_ring() {
    let w = World::new(ring_for(30));
    w.admitted().await;
    w.stream
        .event(3, w.ring_deleted(Uuid::now_v7(), Uuid::now_v7()));
    settle().await;
    w.stream.close();
    settle().await;
    advance(RECONNECT_PAUSE).await;
    w.http.answer(200, r#"{"session":null,"participants":[]}"#);
    w.stream.ready(w.user, false, None);
    settle().await;
    let requests = w.http.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].url,
        format!("{API_URL}/api/v1/channels/{}/voice", w.ring.channel)
    );
    assert!(requests[0].headers.contains(&(
        "Authorization".to_string(),
        "Bearer session-token".to_string()
    )));
    assert_eq!(w.ends(), [RingEnd::CallEnded]);
}

#[tokio::test(start_paused = true)]
async fn a_resume_refused_finds_the_user_in_the_call_or_the_ring_still_standing() {
    let session = Uuid::now_v7();
    for (body, expected) in [
        (
            format!(r#"{{"session":{{"id":"{session}"}},"participants":[{{"user":"USER"}}]}}"#),
            Some(RingEnd::AnsweredElsewhere),
        ),
        (
            format!(r#"{{"session":{{"id":"{session}"}},"participants":[]}}"#),
            None,
        ),
        (
            format!(
                r#"{{"session":{{"id":"{}"}},"participants":[]}}"#,
                Uuid::now_v7()
            ),
            Some(RingEnd::CallEnded),
        ),
    ] {
        let w = World::new(Ring {
            channel: Uuid::now_v7(),
            session,
            until: ring_for(30).until,
        });
        w.admitted().await;
        w.stream.event(1, w.ring_deleted(Uuid::now_v7(), w.user));
        settle().await;
        w.stream.close();
        settle().await;
        advance(RECONNECT_PAUSE).await;
        w.http
            .answer(200, &body.replace("USER", &w.user.to_string()));
        w.stream.ready(w.user, false, None);
        settle().await;
        assert_eq!(w.ends(), expected.into_iter().collect::<Vec<_>>(), "{body}");
        if expected.is_none() {
            // The ring stands: the watch goes on, and the clock ends it.
            assert!(!w.stream.is_closed());
            w.stream.event(2, w.ring_deleted(session, w.user));
            settle().await;
            assert_eq!(w.ends(), [RingEnd::Declined]);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_resume_refused_renews_a_refused_token_for_the_read_and_a_failed_read_watches_on() {
    let token = Token::new(Some("stale"), Some("fresh"));
    let w = World::watching(ring_for(30), token.clone());
    w.admitted().await;
    w.stream.event(1, w.ring_deleted(Uuid::now_v7(), w.user));
    settle().await;
    w.stream.close();
    settle().await;
    advance(RECONNECT_PAUSE).await;
    w.http.answer(
        401,
        r#"{"code":"unauthorized","title":"Unauthorized","status":401}"#,
    );
    w.http.answer(503, "");
    w.stream.ready(w.user, false, None);
    settle().await;
    let bearers: Vec<String> = w
        .http
        .requests()
        .iter()
        .map(|r| r.headers[0].1.clone())
        .collect();
    assert_eq!(bearers, ["Bearer stale", "Bearer fresh"]);
    assert!(w.ends().is_empty(), "a read that fails ends nothing");
    assert!(!w.stream.is_closed());
}

#[tokio::test(start_paused = true)]
async fn a_first_connection_never_reads_the_channel() {
    let w = World::new(ring_for(30));
    settle().await;
    // `resumed` is false on every first connection; nothing was missed.
    w.stream.ready(w.user, false, None);
    settle().await;
    assert!(w.http.requests().is_empty());
    assert!(w.ends().is_empty());
}

#[tokio::test(start_paused = true)]
async fn stopping_ends_the_watch_with_no_report() {
    let w = World::new(ring_for(30));
    w.admitted().await;
    let _ = w.stop.send(true);
    settle().await;
    w.stream.event(1, w.ring_deleted(w.ring.session, w.user));
    advance(Duration::from_secs(31)).await;
    assert!(w.ends().is_empty());
}

#[tokio::test(start_paused = true)]
async fn frames_the_watch_does_not_know_are_skipped() {
    let w = World::new(ring_for(30));
    w.admitted().await;
    w.stream.send(serde_json::json!({ "type": "caughtUp" }));
    w.stream
        .send(serde_json::json!({ "type": "ephemeral", "event": { "type": "typing" } }));
    w.stream
        .send(serde_json::json!({ "type": "someNewFrame", "sequence": "x" }));
    w.stream.send(serde_json::json!("not an object"));
    w.stream.event(
        1,
        serde_json::json!({ "serverEvent": "message", "type": "create", "id": "x" }),
    );
    w.stream.event(
        2,
        serde_json::json!({ "serverEvent": "voiceRing", "type": "delete", "session": 7 }),
    );
    settle().await;
    assert!(w.ends().is_empty());
    w.stream.event(3, w.ring_deleted(w.ring.session, w.user));
    settle().await;
    assert_eq!(w.ends(), [RingEnd::Declined]);
}

#[test]
fn the_ring_watch_runs_on_its_own_thread_and_stops_when_dropped() {
    let stream = Arc::new(FakeStream::default());
    let ends = Arc::new(Ends::default());
    let watch = RingWatch::start(
        Platform {
            http: Arc::new(FakeHttp::default()),
            sockets: stream.clone(),
        },
        Options {
            api_url: API_URL.to_string(),
            credentials: Token::new(Some("session-token"), None),
            loopback_plain_text: false,
        },
        ring_for(30),
        ends.clone(),
    );
    let started = std::time::Instant::now();
    while stream.connections() == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "never connected"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    watch.stop();
    drop(watch);
    assert!(ends.0.lock().unwrap().is_empty());
}
