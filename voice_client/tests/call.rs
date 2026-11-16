//! Scenarios of the call against scripted voice servers, the API server, and a peer connection
//! that records what it is asked, on a paused clock.

mod support;

use std::time::Duration;
use support::*;
use uuid::Uuid;
use voice_client::call::{
    CONNECT_TIMEOUT, Command, DEAD_SERVER_RETRY, ICE_RECOVERY_LIMIT, ICE_RESTART_INTERVAL,
    PING_TIMEOUT, READY_TIMEOUT, RESUME_GRACE,
};
use voice_client::peer_connection::{ConnectionState, Direction, TrackId};
use voice_client::state::{CallStatus, EndedReason, ErrorKind, SessionEndReason};
use voice_protocol::signal::Grants;
use voice_protocol::signal::{
    ClientMessage, ConnectionKind, KickReason, MediaSource, ServerMessage, SlotAssignment,
    SourceInfo, WatchedVideo,
};

const A: &str = "https://a.example.org";
const B: &str = "https://b.example.org";
const C: &str = "https://c.example.org";

/// The pause a `World::new(0.5)` takes before a rejoin or a resume retry.
const PAUSE: Duration = Duration::from_millis(500);

fn mic() -> TrackId {
    World::microphone()
}

fn error(detail: &str, fatal: bool, retry: Option<u64>, refused: Option<&str>) -> ServerMessage {
    ServerMessage::Error {
        detail: detail.to_string(),
        fatal,
        retry_after_seconds: retry,
        refused: refused.map(str::to_string),
    }
}

// --- Joining --------------------------------------------------------------------------------

#[test]
fn the_timeouts_are_as_documented() {
    assert_eq!(READY_TIMEOUT, Duration::from_secs(6));
    assert_eq!(PING_TIMEOUT, Duration::from_secs(3));
    assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(15));
    assert_eq!(voice_client::call::REJOIN_DELAY_MAX, Duration::from_secs(1));
    assert_eq!(DEAD_SERVER_RETRY, Duration::from_secs(60));
    assert_eq!(RESUME_GRACE, Duration::from_secs(20));
    assert_eq!(ICE_RESTART_INTERVAL, Duration::from_secs(4));
    assert_eq!(ICE_RECOVERY_LIMIT, Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn candidates_are_tried_nearest_first_and_one_that_refuses_is_reported() {
    let w = World::new(0.5);
    w.http.health.lock().unwrap().extend([
        (A.to_string(), Health::After(Duration::from_millis(50))),
        (B.to_string(), Health::After(Duration::from_millis(10))),
        (C.to_string(), Health::Silent),
    ]);
    let (a, b, c) = (
        w.sockets.server(A),
        w.sockets.server(B),
        w.sockets.server(C),
    );
    b.set(Behaviour::Refuse);
    let offer = Offer::to(w.channel, &[A, B, C]);
    w.join(&offer).await;
    assert_eq!(w.state().status, CallStatus::Joining);
    assert_eq!(w.http.health_asked().len(), 3);
    assert!(
        w.sockets.connected.lock().unwrap().is_empty(),
        "ranking first"
    );
    advance(Duration::from_millis(10)).await;
    advance(Duration::from_millis(40)).await;
    assert!(
        w.sockets.connected.lock().unwrap().is_empty(),
        "the silent one is waited for"
    );
    advance(PING_TIMEOUT - Duration::from_millis(50)).await;
    assert_eq!(
        *w.sockets.connected.lock().unwrap(),
        vec!["wss://b.example.org/ws", "wss://a.example.org/ws"]
    );
    assert_eq!(w.http.failures_reported(), vec![offer.candidates[1].0]);
    w.admit(&a).await;
    assert_eq!(c.connections(), 0);
    assert_eq!(w.sockets.connected.lock().unwrap().len(), 2);
    let state = w.state();
    assert_eq!(state.session, Some(w.session));
    assert_eq!(state.user, Some(w.user));
    assert_eq!(state.audio_slots, 16);
    assert_eq!(state.slots[0].track, Some(w.pcs.latest().received("0")));
    assert_eq!(state.microphone, Some(mic()));
    assert_eq!(
        state.grants,
        Grants {
            speak: true,
            share_screen: true,
            camera: true,
            transfer_files: false
        }
    );
}

#[tokio::test(start_paused = true)]
async fn one_candidate_is_tried_without_ranking() {
    let w = World::new(0.5);
    let (_, _) = w.connected(A).await;
    assert!(w.http.health_asked().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_server_that_does_not_answer_ready_in_time_is_reported_and_the_next_tried() {
    let w = World::new(0.5);
    w.http
        .health
        .lock()
        .unwrap()
        .insert(B.to_string(), Health::After(Duration::from_millis(5)));
    let (a, b) = (w.sockets.server(A), w.sockets.server(B));
    let offer = Offer::to(w.channel, &[A, B]);
    w.join(&offer).await;
    advance(PING_TIMEOUT).await;
    assert_eq!(a.connections(), 1);
    assert!(a.received().contains(&ClientMessage::Identify {
        token: "join-token".to_string()
    }));
    advance(READY_TIMEOUT - Duration::from_millis(1)).await;
    assert_eq!(b.connections(), 0);
    advance(Duration::from_millis(1)).await;
    assert!(a.is_closed());
    assert_eq!(w.http.failures_reported(), vec![offer.candidates[0].0]);
    assert_eq!(b.connections(), 1);
    w.admit(&b).await;
    // Nothing of the first candidate's late answer reaches the call.
    assert_eq!(w.state().status, CallStatus::Connected);
}

#[tokio::test(start_paused = true)]
async fn a_connection_that_hangs_is_the_servers_failure_too() {
    let w = World::new(0.5);
    let (a, b) = (w.sockets.server(A), w.sockets.server(B));
    a.set(Behaviour::Hang);
    let offer = Offer::to(w.channel, &[A, B]);
    w.join(&offer).await;
    advance(PING_TIMEOUT + READY_TIMEOUT).await;
    assert_eq!(w.http.failures_reported(), vec![offer.candidates[0].0]);
    w.admit(&b).await;
}

#[tokio::test(start_paused = true)]
async fn media_that_never_connects_times_out_and_the_next_candidate_is_tried() {
    let w = World::new(0.5);
    let (a, b) = (w.sockets.server(A), w.sockets.server(B));
    let offer = Offer::to(w.channel, &[A, B]);
    w.join(&offer).await;
    advance(PING_TIMEOUT).await;
    a.received();
    a.ready(w.session, w.user, Vec::new(), "secret");
    settle().await;
    a.offer(1, vec![slot("0", 0)]);
    settle().await;
    assert!(matches!(
        a.received().as_slice(),
        [
            ClientMessage::Send {
                source: MediaSource::Microphone,
                external: false
            },
            ClientMessage::Answer { revision: 1, .. }
        ]
    ));
    assert_eq!(w.state().status, CallStatus::Joining);
    w.pcs.latest().report(ConnectionState::Disconnected);
    settle().await;
    assert!(
        a.received().is_empty(),
        "no ICE restart is asked for while joining"
    );
    advance(CONNECT_TIMEOUT).await;
    assert_eq!(a.received(), vec![ClientMessage::Leave]);
    assert!(a.is_closed());
    assert!(w.pcs.latest().is_closed());
    assert_eq!(w.http.failures_reported(), vec![offer.candidates[0].0]);
    assert_eq!(w.state().session, None);
    w.admit(&b).await;
    assert_eq!(w.pcs.count(), 2);
    assert_eq!(w.state().microphone, Some(mic()));
}

#[tokio::test(start_paused = true)]
async fn media_that_fails_while_joining_tries_the_next_candidate() {
    let w = World::new(0.5);
    let (a, b) = (w.sockets.server(A), w.sockets.server(B));
    let offer = Offer::to(w.channel, &[A, B]);
    w.join(&offer).await;
    advance(PING_TIMEOUT).await;
    a.ready(w.session, w.user, Vec::new(), "secret");
    settle().await;
    a.offer(1, vec![slot("0", 0)]);
    settle().await;
    w.pcs.latest().report(ConnectionState::Failed);
    settle().await;
    assert_eq!(w.http.failures_reported(), vec![offer.candidates[0].0]);
    w.admit(&b).await;
}

#[tokio::test(start_paused = true)]
async fn a_refusal_for_coming_too_fast_fails_the_join_and_blames_no_server() {
    let w = World::new(0.5);
    let (a, b) = (w.sockets.server(A), w.sockets.server(B));
    let offer = Offer::to(w.channel, &[A, B]);
    w.join(&offer).await;
    advance(PING_TIMEOUT).await;
    a.send(&error("too many identify frames", true, Some(30), None));
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Failed);
    assert_eq!(state.error_kind, Some(ErrorKind::Refused));
    assert_eq!(state.retry_after_seconds, Some(30));
    assert_eq!(state.error.as_deref(), Some("too many identify frames"));
    assert_eq!(state.channel, Some(w.channel));
    assert_eq!(
        state.microphone, None,
        "a failed call releases the microphone"
    );
    assert!(w.http.failures_reported().is_empty());
    assert_eq!(b.connections(), 0);
    assert!(a.is_closed());
}

#[tokio::test(start_paused = true)]
async fn a_fatal_error_without_a_wait_is_the_servers_failure() {
    let w = World::new(0.5);
    let (a, b) = (w.sockets.server(A), w.sockets.server(B));
    let offer = Offer::to(w.channel, &[A, B]);
    w.join(&offer).await;
    advance(PING_TIMEOUT).await;
    a.send(&error("the token is not for this server", true, None, None));
    settle().await;
    assert_eq!(w.http.failures_reported(), vec![offer.candidates[0].0]);
    w.admit(&b).await;
}

#[tokio::test(start_paused = true)]
async fn every_candidate_failing_fails_the_call_with_the_last_reason() {
    let w = World::new(0.5);
    let (a, b) = (w.sockets.server(A), w.sockets.server(B));
    a.set(Behaviour::Refuse);
    let offer = Offer::to(w.channel, &[A, B]);
    w.join(&offer).await;
    advance(PING_TIMEOUT).await;
    b.send(&error("the token expired", true, None, None));
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Failed);
    assert_eq!(state.error_kind, Some(ErrorKind::Server));
    assert_eq!(state.error.as_deref(), Some("the token expired"));
    assert_eq!(
        w.http.failures_reported(),
        vec![offer.candidates[0].0, offer.candidates[1].0]
    );
}

#[tokio::test(start_paused = true)]
async fn a_join_offer_the_api_server_refuses_fails_at_once() {
    let w = World::new(0.5);
    w.http
        .answer_join(problem(403, "blocked", "A block stands between you."));
    w.send(Command::SetMicrophone(Some(mic())));
    w.send(Command::Join { channel: w.channel });
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Failed);
    assert_eq!(state.error_kind, Some(ErrorKind::Offer));
    assert_eq!(state.error.as_deref(), Some("A block stands between you."));
    assert!(w.sockets.connected.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_speaking_join_without_a_microphone_fails_before_any_server() {
    let w = World::new(0.5);
    w.http.answer_join(Offer::to(w.channel, &[A]).response());
    w.sockets.server(A);
    w.send(Command::Join { channel: w.channel });
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Failed);
    assert_eq!(state.error_kind, Some(ErrorKind::Microphone));
    assert!(w.sockets.connected.lock().unwrap().is_empty());
    assert!(w.http.failures_reported().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_listener_joins_without_a_microphone_and_lets_a_held_one_go() {
    let w = World::new(0.5);
    let mut offer = Offer::to(w.channel, &[A]);
    offer.speak = false;
    let a = w.sockets.server(A);
    w.join(&offer).await;
    assert_eq!(w.state().microphone, None);
    w.admit(&a).await;
    assert!(
        !a.received()
            .iter()
            .any(|f| matches!(f, ClientMessage::Send { .. }))
    );
    assert!(!w.state().grants.speak);
}

#[tokio::test(start_paused = true)]
async fn a_server_not_reached_over_https_is_never_tried_nor_blamed() {
    let w = World::new(0.5);
    let offer = Offer::to(w.channel, &["http://voice.example.org"]);
    w.join(&offer).await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Failed);
    assert_eq!(state.error_kind, Some(ErrorKind::Server));
    assert_eq!(
        state.error.as_deref(),
        Some("http://voice.example.org is not reached over HTTPS")
    );
    assert!(w.sockets.connected.lock().unwrap().is_empty());
    assert!(w.http.failures_reported().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_peer_connection_that_cannot_be_made_is_this_devices_fault() {
    let w = World::new(0.5);
    w.pcs
        .fail_create
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let a = w.sockets.server(A);
    w.join(&Offer::to(w.channel, &[A])).await;
    a.ready(w.session, w.user, Vec::new(), "secret");
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Failed);
    assert_eq!(state.error_kind, Some(ErrorKind::Device));
    assert!(w.http.failures_reported().is_empty());
    assert!(a.received().contains(&ClientMessage::Leave));
    assert!(a.is_closed());
}

#[tokio::test(start_paused = true)]
async fn an_offer_that_cannot_be_applied_while_joining_is_this_devices_fault() {
    let w = World::new(0.5);
    let a = w.sockets.server(A);
    w.join(&Offer::to(w.channel, &[A])).await;
    a.ready(w.session, w.user, Vec::new(), "secret");
    settle().await;
    w.pcs.latest().fail_next_remote();
    a.offer(1, vec![slot("0", 0)]);
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Failed);
    assert_eq!(state.error_kind, Some(ErrorKind::Device));
    assert!(w.http.failures_reported().is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_offer_arriving_while_media_connects_is_answered() {
    let w = World::new(0.5);
    let a = w.sockets.server(A);
    w.join(&Offer::to(w.channel, &[A])).await;
    a.ready(w.session, w.user, Vec::new(), "secret");
    settle().await;
    a.offer(1, vec![slot("0", 0)]);
    settle().await;
    a.send(&ServerMessage::Slots {
        assignments: vec![SlotAssignment {
            slot: 1,
            user: Some(Uuid::now_v7()),
        }],
    });
    a.offer(2, vec![slot("0", 0), slot("1", 1)]);
    settle().await;
    let answers: Vec<u64> = a
        .received()
        .into_iter()
        .filter_map(|f| match f {
            ClientMessage::Answer { revision, .. } => Some(revision),
            _ => None,
        })
        .collect();
    assert_eq!(answers, vec![1, 2]);
    assert_eq!(w.state().status, CallStatus::Joining);
    assert_eq!(w.state().slots.len(), 2);
    w.pcs.latest().report(ConnectionState::Connected);
    settle().await;
    assert_eq!(w.state().status, CallStatus::Connected);
}

#[tokio::test(start_paused = true)]
async fn joining_the_call_already_joined_changes_nothing() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.send(Command::Join { channel: w.channel });
    settle().await;
    assert_eq!(w.http.offers_asked(), 1);
    assert_eq!(a.connections(), 1);
    assert_eq!(w.state().status, CallStatus::Connected);
}

#[tokio::test(start_paused = true)]
async fn moving_to_another_call_leaves_the_first_and_keeps_the_microphone() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.send(Command::SetWatched(vec![WatchedVideo {
        user: Uuid::now_v7(),
        source: MediaSource::Camera,
    }]));
    settle().await;
    a.received();
    let other = Uuid::now_v7();
    let b = w.sockets.server(B);
    w.http.answer_join(Offer::to(other, &[B]).response());
    w.send(Command::Join { channel: other });
    settle().await;
    assert_eq!(a.received(), vec![ClientMessage::Leave]);
    assert!(a.is_closed());
    assert!(w.pcs.latest().is_closed());
    let state = w.state();
    assert_eq!(state.status, CallStatus::Joining);
    assert_eq!(state.channel, Some(other));
    assert_eq!(state.microphone, Some(mic()));
    assert!(
        state.watching.is_empty(),
        "what was watched belongs to the first call"
    );
    assert!(state.slots.is_empty() && state.participants.is_empty());
    w.admit(&b).await;
    assert!(b.received().is_empty());
    assert_eq!(w.state().microphone, Some(mic()));
}

#[tokio::test(start_paused = true)]
async fn leaving_sends_leave_and_resets_the_state() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.send(Command::SetMuted(true));
    settle().await;
    a.received();
    w.send(Command::Leave);
    settle().await;
    assert_eq!(a.received(), vec![ClientMessage::Leave]);
    assert!(a.is_closed());
    assert!(w.pcs.latest().is_closed());
    let state = w.state();
    assert_eq!(state.status, CallStatus::Idle);
    assert_eq!(state.channel, None);
    assert!(!state.muted, "the mute is lifted with the call");
    assert_eq!(state.microphone, None);
    // Leaving again changes nothing, so no snapshot is made of it.
    let snapshots = w.snapshots.0.lock().unwrap().len();
    w.send(Command::Leave);
    settle().await;
    assert_eq!(w.state().status, CallStatus::Idle);
    assert_eq!(w.snapshots.0.lock().unwrap().len(), snapshots);
}

// --- Mute and deafen ------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn the_microphone_is_sent_once_its_section_is_offered_and_muting_takes_it_off_the_sender() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    pc.ops();
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    assert_eq!(
        pc.ops(),
        vec![
            Op::SetRemote("v=0 mids=0,1".to_string()),
            Op::SetDirection("1".to_string(), Direction::SendOnly),
            Op::ReplaceTrack("1".to_string(), Some(mic())),
            Op::CreateAnswer,
            Op::SetLocal("answer mids=0,1".to_string()),
        ]
    );
    assert_eq!(
        a.received(),
        vec![ClientMessage::Answer {
            connection: ConnectionKind::Call,
            revision: 2,
            sdp: "answer mids=0,1".to_string()
        }]
    );
    assert_eq!(w.state().sources.len(), 1);
    assert!(!w.state().sources[0].live);
    a.send(&ServerMessage::Sending {
        source: MediaSource::Microphone,
    });
    settle().await;
    assert!(w.state().sources[0].live);

    w.send(Command::SetMuted(true));
    settle().await;
    assert!(w.state().muted, "muting shows at once");
    assert_eq!(
        a.received(),
        vec![ClientMessage::SetState {
            muted: true,
            deafened: false,
            held: false
        }]
    );
    assert_eq!(pc.ops(), vec![Op::ReplaceTrack("1".to_string(), None)]);

    w.send(Command::SetMuted(false));
    settle().await;
    assert!(w.state().muted, "unmuting waits for the server");
    assert_eq!(
        a.received(),
        vec![ClientMessage::SetState {
            muted: false,
            deafened: false,
            held: false
        }]
    );
    assert!(pc.ops().is_empty());
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: false,
        deafened: false,
        held: false,
    });
    settle().await;
    assert!(!w.state().muted);
    assert_eq!(
        pc.ops(),
        vec![Op::ReplaceTrack("1".to_string(), Some(mic()))]
    );
}

#[tokio::test(start_paused = true)]
async fn the_servers_word_never_shows_the_user_less_silenced_than_they_asked() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    w.send(Command::SetMuted(true));
    w.send(Command::SetDeafened(true));
    settle().await;
    w.pcs.latest().ops();
    // An answer to the earlier, unmuted request.
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: false,
        deafened: false,
        held: false,
    });
    settle().await;
    let state = w.state();
    assert!(state.muted && state.deafened);
    assert!(w.pcs.latest().ops().is_empty());
    // Someone else's state is theirs.
    let bob = Uuid::now_v7();
    a.send(&ServerMessage::ParticipantJoined {
        user: bob,
        muted: false,
        deafened: false,
    });
    a.send(&ServerMessage::ParticipantState {
        user: bob,
        muted: true,
        deafened: false,
        held: false,
    });
    settle().await;
    assert!(w.state().participants[0].muted);
    assert!(w.state().muted);
}

#[tokio::test(start_paused = true)]
async fn a_moderators_mute_shows_on_the_users_own_controls() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    w.pcs.latest().ops();
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: true,
        deafened: false,
        held: false,
    });
    settle().await;
    assert!(w.state().muted);
    assert_eq!(
        w.pcs.latest().ops(),
        vec![Op::ReplaceTrack("1".to_string(), None)]
    );
    assert!(!w.state().server_muted);
    w.send(Command::SetServerMuted(true));
    settle().await;
    assert!(w.state().server_muted);
}

#[tokio::test(start_paused = true)]
async fn a_refused_unmute_leaves_the_user_as_shown() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.send(Command::SetMuted(true));
    settle().await;
    w.send(Command::SetMuted(false));
    settle().await;
    a.received();
    a.send(&error(
        "too many setState frames",
        false,
        Some(7),
        Some("setState"),
    ));
    settle().await;
    let state = w.state();
    assert!(state.muted);
    let refusal = state.state_refused.clone().expect("the refusal is kept");
    assert_eq!(refusal.retry_after_seconds, Some(7));
    assert_eq!(refusal.detail, "too many setState frames");
    // The next deafen is asked for with the mute as it stands, not the lifted one.
    w.send(Command::SetDeafened(true));
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::SetState {
            muted: true,
            deafened: true,
            held: false
        }]
    );
    assert_eq!(w.state().state_refused, None);
    a.send(&error(
        "too many setState frames",
        false,
        None,
        Some("setState"),
    ));
    settle().await;
    assert!(w.state().state_refused.is_some());
    w.send(Command::ClearStateRefusal);
    settle().await;
    assert_eq!(w.state().state_refused, None);
}

#[tokio::test(start_paused = true)]
async fn deafening_shows_at_once_and_undeafening_waits_for_the_server() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    a.received();
    w.send(Command::SetDeafened(true));
    settle().await;
    assert!(w.state().deafened);
    w.send(Command::SetDeafened(false));
    settle().await;
    assert!(w.state().deafened);
    assert_eq!(
        a.received(),
        vec![
            ClientMessage::SetState {
                muted: false,
                deafened: true,
                held: false
            },
            ClientMessage::SetState {
                muted: false,
                deafened: false,
                held: false
            }
        ]
    );
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: false,
        deafened: false,
        held: false,
    });
    settle().await;
    assert!(!w.state().deafened);
}

#[tokio::test(start_paused = true)]
async fn outside_a_call_the_mute_simply_is_the_state() {
    let w = World::new(0.5);
    w.send(Command::SetMuted(true));
    settle().await;
    assert!(w.state().muted);
    w.send(Command::SetMuted(false));
    settle().await;
    assert!(!w.state().muted, "nothing to wait for outside a call");
    w.send(Command::SetMuted(true));
    settle().await;
    let offer = Offer::to(w.channel, &[A]);
    let a = w.sockets.server(A);
    w.join(&offer).await;
    let frames = w.admit(&a).await;
    assert!(frames.contains(&ClientMessage::SetState {
        muted: true,
        deafened: false,
        held: false
    }));
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    assert!(
        w.pcs
            .latest()
            .ops()
            .contains(&Op::ReplaceTrack("1".to_string(), None)),
        "a muted microphone's sender carries nothing from the start"
    );
}

// --- Slots, volume, videos, sources ---------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn slots_grow_with_the_call_and_a_persons_volume_follows_the_slot_they_are_in() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    let (bob, carol) = (Uuid::now_v7(), Uuid::now_v7());
    a.send(&ServerMessage::Slots {
        assignments: vec![SlotAssignment {
            slot: 0,
            user: Some(bob),
        }],
    });
    settle().await;
    assert_eq!(w.state().slots[0].user, Some(bob));
    // A new slot is told of before the offer that adds its section.
    a.send(&ServerMessage::Slots {
        assignments: vec![SlotAssignment {
            slot: 1,
            user: Some(carol),
        }],
    });
    settle().await;
    assert_eq!(w.state().slots.len(), 2);
    assert_eq!(w.state().slots[1].user, Some(carol));
    assert_eq!(w.state().slots[1].track, None);
    pc.ops();
    a.offer(2, vec![slot("0", 0), slot("1", 1)]);
    settle().await;
    assert_eq!(w.state().slots[1].track, Some(pc.received("1")));
    assert!(
        !pc.ops().iter().any(|op| matches!(op, Op::SetVolume(..))),
        "a volume of 1 is not set"
    );

    w.send(Command::SetUserVolume {
        user: carol,
        source: MediaSource::Microphone,
        gain: 2.0,
    });
    settle().await;
    assert_eq!(pc.ops(), vec![Op::SetVolume("1".to_string(), 2.0)]);
    // Carol moves to slot 0, Bob is muted and his slot freed: her volume follows her.
    a.send(&ServerMessage::Slots {
        assignments: vec![
            SlotAssignment {
                slot: 0,
                user: Some(carol),
            },
            SlotAssignment {
                slot: 1,
                user: None,
            },
        ],
    });
    settle().await;
    assert_eq!(pc.ops(), vec![Op::SetVolume("0".to_string(), 2.0)]);
    a.send(&ServerMessage::Slots {
        assignments: vec![SlotAssignment {
            slot: 1,
            user: Some(bob),
        }],
    });
    settle().await;
    assert_eq!(
        pc.ops(),
        vec![Op::SetVolume("1".to_string(), 1.0)],
        "a slot's volume returns to 1 for its next occupant"
    );
    w.send(Command::SetUserMuted {
        user: carol,
        source: MediaSource::Microphone,
        muted: true,
    });
    settle().await;
    assert_eq!(pc.ops(), vec![Op::SetVolume("0".to_string(), 0.0)]);
    w.send(Command::SetUserMuted {
        user: carol,
        source: MediaSource::Microphone,
        muted: false,
    });
    settle().await;
    assert_eq!(pc.ops(), vec![Op::SetVolume("0".to_string(), 2.0)]);
    w.send(Command::SetUserVolume {
        user: carol,
        source: MediaSource::Microphone,
        gain: 40.0,
    });
    settle().await;
    assert_eq!(pc.ops(), vec![Op::SetVolume("0".to_string(), 10.0)]);
}

#[tokio::test(start_paused = true)]
async fn watching_gives_a_section_for_each_video_and_a_shares_sound_has_its_own_volume() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    let bob = Uuid::now_v7();
    w.send(Command::SetUserVolume {
        user: bob,
        source: MediaSource::ScreenAudio,
        gain: 0.5,
    });
    let watched = vec![
        WatchedVideo {
            user: bob,
            source: MediaSource::Screen,
        },
        WatchedVideo {
            user: bob,
            source: MediaSource::Camera,
        },
    ];
    w.send(Command::SetWatched(watched.clone()));
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::Watch {
            video: watched.clone()
        }]
    );
    assert_eq!(w.state().watching, watched);
    pc.ops();
    a.offer(
        2,
        vec![
            slot("0", 0),
            video("1", bob, MediaSource::Screen),
            share_audio("2", bob),
            video("3", bob, MediaSource::Camera),
        ],
    );
    settle().await;
    let state = w.state();
    assert_eq!(state.videos.len(), 2);
    assert_eq!(state.videos[0].source, MediaSource::Screen);
    assert_eq!(state.videos[0].track, pc.received("1"));
    assert_eq!(state.videos[1].track, pc.received("3"));
    assert_eq!(state.share_audio[0].user, bob);
    assert_eq!(state.share_audio[0].track, pc.received("2"));
    assert!(pc.ops().contains(&Op::SetVolume("2".to_string(), 0.5)));
    w.send(Command::SetUserMuted {
        user: bob,
        source: MediaSource::ScreenAudio,
        muted: true,
    });
    settle().await;
    assert_eq!(pc.ops(), vec![Op::SetVolume("2".to_string(), 0.0)]);
    // Bob stops sharing: the server takes the sections away.
    a.offer(3, vec![slot("0", 0), video("3", bob, MediaSource::Camera)]);
    settle().await;
    let state = w.state();
    assert_eq!(state.videos.len(), 1);
    assert!(state.share_audio.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_camera_is_sent_with_its_encodings_and_stopped() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    let camera = TrackId("camera-1".to_string());
    w.send(Command::StartSource {
        source: MediaSource::Camera,
        track: camera.clone(),
    });
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::Send {
            source: MediaSource::Camera,
            external: false
        }]
    );
    pc.ops();
    a.offer(
        2,
        vec![
            slot("0", 0),
            sending("1", MediaSource::Microphone),
            sending("2", MediaSource::Camera),
        ],
    );
    settle().await;
    let ops = pc.ops();
    assert!(ops.contains(&Op::ReplaceTrack("2".to_string(), Some(camera.clone()))));
    let (local, encoded) = (
        ops.iter()
            .position(|op| matches!(op, Op::SetLocal(_)))
            .unwrap(),
        ops.iter()
            .position(|op| *op == Op::SetEncoding("2".to_string(), Some(4_000_000), Some(30.0)))
            .expect("the camera's encoding is bounded"),
    );
    assert!(
        local < encoded,
        "bounded once the answer is applied: {ops:?}"
    );
    assert!(
        !ops.iter()
            .any(|op| matches!(op, Op::SetEncoding(mid, ..) if mid == "1"))
    );
    a.send(&ServerMessage::Sending {
        source: MediaSource::Camera,
    });
    settle().await;
    assert!(
        w.state()
            .sources
            .iter()
            .any(|s| s.source == MediaSource::Camera && s.live)
    );
    // Another offer does not set the encodings again.
    a.offer(
        3,
        vec![
            slot("0", 0),
            sending("1", MediaSource::Microphone),
            sending("2", MediaSource::Camera),
            slot("3", 1),
        ],
    );
    settle().await;
    assert!(!pc.ops().iter().any(|op| matches!(op, Op::SetEncoding(..))));
    a.received();
    w.send(Command::StopSource(MediaSource::Camera));
    settle().await;
    assert_eq!(pc.ops(), vec![Op::ReplaceTrack("2".to_string(), None)]);
    assert_eq!(
        a.received(),
        vec![ClientMessage::StopSending {
            source: MediaSource::Camera
        }]
    );
    assert!(
        !w.state()
            .sources
            .iter()
            .any(|s| s.source == MediaSource::Camera)
    );
    // Stopping it again, or something never sent, is nothing.
    w.send(Command::StopSource(MediaSource::Camera));
    settle().await;
    assert!(a.received().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_screen_is_sent_as_picture_and_sound_and_a_source_without_its_grant_is_refused_here() {
    let w = World::new(0.5);
    let mut offer = Offer::to(w.channel, &[A]);
    offer.camera = false;
    let a = w.sockets.server(A);
    w.join(&offer).await;
    w.admit(&a).await;
    a.received();
    w.send(Command::StartSource {
        source: MediaSource::Camera,
        track: TrackId("camera-1".to_string()),
    });
    settle().await;
    assert!(a.received().is_empty());
    let (source, refusal) = w.state().source_refused.clone().expect("refused");
    assert_eq!(source, MediaSource::Camera);
    assert_eq!(refusal.detail, "this call does not let you send it");
    w.send(Command::StartSource {
        source: MediaSource::Screen,
        track: TrackId("screen-1".to_string()),
    });
    w.send(Command::StartSource {
        source: MediaSource::ScreenAudio,
        track: TrackId("screen-audio-1".to_string()),
    });
    settle().await;
    assert_eq!(w.state().source_refused, None);
    assert_eq!(a.received().len(), 2);
    w.send(Command::StartSource {
        source: MediaSource::Screen,
        track: TrackId("screen-2".to_string()),
    });
    settle().await;
    assert_eq!(
        w.state().source_refused.clone().map(|(_, r)| r.detail),
        Some("already sending it".to_string())
    );
    w.pcs.latest().ops();
    a.offer(
        2,
        vec![
            slot("0", 0),
            sending("1", MediaSource::Screen),
            sending("2", MediaSource::ScreenAudio),
        ],
    );
    settle().await;
    let ops = w.pcs.latest().ops();
    assert!(ops.contains(&Op::SetEncoding(
        "1".to_string(),
        Some(25_000_000),
        Some(60.0)
    )));
    assert!(ops.contains(&Op::ReplaceTrack(
        "2".to_string(),
        Some(TrackId("screen-audio-1".to_string()))
    )));
    // The server takes the sections away (the grant gone): the sources go with them.
    a.offer(3, vec![slot("0", 0)]);
    settle().await;
    assert!(
        w.state()
            .sources
            .iter()
            .all(|s| s.source == MediaSource::Microphone),
        "the microphone, still asked for, waits for its section"
    );
    w.send(Command::ClearSourceRefusal);
    settle().await;
    assert_eq!(w.state().source_refused, None);
}

#[tokio::test(start_paused = true)]
async fn a_source_the_server_refuses_is_dropped() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    w.send(Command::StartSource {
        source: MediaSource::Camera,
        track: TrackId("camera-1".to_string()),
    });
    settle().await;
    a.send(&error("too many send frames", false, Some(3), Some("send")));
    settle().await;
    let state = w.state();
    assert_eq!(
        state.sources.len(),
        1,
        "the microphone, asked for first, is kept"
    );
    assert_eq!(state.sources[0].source, MediaSource::Microphone);
    let (source, refusal) = state.source_refused.expect("refused");
    assert_eq!(source, MediaSource::Camera);
    assert_eq!(refusal.retry_after_seconds, Some(3));
}

#[tokio::test(start_paused = true)]
async fn grants_lost_stop_sources_and_a_grant_gained_sends_the_microphone() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    w.send(Command::StartSource {
        source: MediaSource::Camera,
        track: TrackId("camera-1".to_string()),
    });
    settle().await;
    a.offer(
        2,
        vec![
            slot("0", 0),
            sending("1", MediaSource::Microphone),
            sending("2", MediaSource::Camera),
        ],
    );
    settle().await;
    a.received();
    pc.ops();
    a.send(&ServerMessage::GrantsChanged {
        grants: Grants {
            speak: false,
            share_screen: true,
            camera: false,
            transfer_files: false,
        },
    });
    settle().await;
    let state = w.state();
    assert!(state.sources.is_empty());
    assert_eq!(state.microphone, None);
    assert!(!state.grants.speak && !state.grants.camera);
    let ops = pc.ops();
    assert!(ops.contains(&Op::ReplaceTrack("2".to_string(), None)));
    assert!(ops.contains(&Op::ReplaceTrack("1".to_string(), None)));
    assert_eq!(a.received().len(), 2);
    // The server takes the sections away.
    a.offer(3, vec![slot("0", 0)]);
    settle().await;
    // Speak is granted again: nothing is sent until a microphone is given.
    a.send(&ServerMessage::GrantsChanged {
        grants: Grants {
            speak: true,
            share_screen: true,
            camera: false,
            transfer_files: false,
        },
    });
    settle().await;
    a.received();
    assert!(w.state().sources.is_empty());
    w.send(Command::SetMicrophone(Some(TrackId("mic-2".to_string()))));
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::Send {
            source: MediaSource::Microphone,
            external: false
        }]
    );
    assert_eq!(w.state().sources[0].track, TrackId("mic-2".to_string()));
}

#[tokio::test(start_paused = true)]
async fn the_microphone_can_be_replaced_or_closed_while_sent() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    pc.ops();
    a.received();
    let other = TrackId("mic-2".to_string());
    w.send(Command::SetMicrophone(Some(other.clone())));
    settle().await;
    assert_eq!(
        pc.ops(),
        vec![Op::ReplaceTrack("1".to_string(), Some(other.clone()))]
    );
    assert!(
        a.received().is_empty(),
        "replacing a track needs no signalling"
    );
    assert_eq!(w.state().sources[0].track, other);
    // Replaced while muted, the new one waits for the unmute.
    w.send(Command::SetMuted(true));
    settle().await;
    pc.ops();
    w.send(Command::SetMicrophone(Some(TrackId("mic-3".to_string()))));
    settle().await;
    assert!(pc.ops().is_empty());
    a.received();
    w.send(Command::SetMicrophone(None));
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::StopSending {
            source: MediaSource::Microphone
        }]
    );
    assert!(w.state().sources.is_empty());
    assert_eq!(w.state().microphone, None);
}

#[tokio::test(start_paused = true)]
async fn participants_follow_the_frames() {
    let w = World::new(0.5);
    let (bob, carol) = (Uuid::now_v7(), Uuid::now_v7());
    let offer = Offer::to(w.channel, &[A]);
    let a = w.sockets.server(A);
    w.join(&offer).await;
    a.ready(
        w.session,
        w.user,
        vec![voice_protocol::signal::ParticipantInfo {
            user: bob,
            muted: true,
            deafened: false,
            held: false,
            speaking: false,
            sources: vec![SourceInfo {
                source: MediaSource::Microphone,
                paused: true,
            }],
        }],
        "secret",
    );
    settle().await;
    assert_eq!(w.state().participants.len(), 1);
    assert!(w.state().participants[0].muted);
    a.send(&ServerMessage::ParticipantJoined {
        user: carol,
        muted: false,
        deafened: true,
    });
    a.send(&ServerMessage::Speaking {
        user: carol,
        speaking: true,
    });
    a.send(&ServerMessage::Sources {
        user: carol,
        sources: vec![SourceInfo {
            source: MediaSource::Camera,
            paused: false,
        }],
    });
    settle().await;
    let carol_state = w.state().participants[1].clone();
    assert!(carol_state.deafened && carol_state.speaking);
    assert_eq!(carol_state.sources[0].source, MediaSource::Camera);
    // Bob joins again from another device: his participant starts afresh.
    a.send(&ServerMessage::ParticipantJoined {
        user: bob,
        muted: false,
        deafened: false,
    });
    settle().await;
    let state = w.state();
    assert_eq!(state.participants.len(), 2);
    let bob_state = state.participants.iter().find(|p| p.user == bob).unwrap();
    assert!(!bob_state.muted && bob_state.sources.is_empty());
    a.send(&ServerMessage::ParticipantLeft { user: carol });
    settle().await;
    assert_eq!(w.state().participants.len(), 1);
    // Frames a newer server adds, and the desktop helper's connection, are skipped.
    a.send_text(r#"{"type":"handRaised","user":"x"}"#.to_string());
    a.send(&ServerMessage::Offer {
        connection: ConnectionKind::Capture,
        revision: 9,
        sdp: "v=0 mids=c".to_string(),
        sections: Vec::new(),
    });
    settle().await;
    assert_eq!(w.state().participants.len(), 1);
    assert!(
        a.received()
            .iter()
            .all(|f| !matches!(f, ClientMessage::Answer { revision: 9, .. }))
    );
}

// --- Ending ---------------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn each_kicked_reason_ends_or_rejoins_as_documented() {
    let ends = [
        (KickReason::Kicked, Some(EndedReason::Kicked)),
        (KickReason::AccessLost, Some(EndedReason::AccessLost)),
        (KickReason::Replaced, None),
        (KickReason::SignedOut, None),
    ];
    for (reason, ended) in ends {
        let w = World::new(0.5);
        let (_, a) = w.connected(A).await;
        a.received();
        a.send(&ServerMessage::Kicked { reason });
        settle().await;
        let state = w.state();
        assert_eq!(state.status, CallStatus::Idle, "{reason:?}");
        assert_eq!(state.ended_reason, ended, "{reason:?}");
        assert_eq!(state.channel, None);
        assert!(w.pcs.latest().is_closed());
        assert_eq!(a.received(), vec![ClientMessage::Leave]);
        assert_eq!(w.http.offers_asked(), 1, "{reason:?} is not rejoined");
        w.send(Command::AcknowledgeEnd);
        settle().await;
        assert_eq!(w.state().ended_reason, None);
    }
    for reason in [KickReason::ServerStopping, KickReason::ConnectionFailed] {
        let w = World::new(0.5);
        let (_, a) = w.connected(A).await;
        a.received();
        a.send(&ServerMessage::Kicked { reason });
        settle().await;
        let state = w.state();
        assert_eq!(state.status, CallStatus::Rejoining, "{reason:?}");
        assert_eq!(state.channel, Some(w.channel));
        assert_eq!(state.session, None);
        assert_eq!(
            state.microphone,
            Some(mic()),
            "a rejoin keeps the microphone"
        );
        assert_eq!(a.received(), vec![ClientMessage::Leave]);
        assert_eq!(w.http.offers_asked(), 1);
        advance(PAUSE - Duration::from_millis(1)).await;
        assert_eq!(w.http.offers_asked(), 1, "the pause is waited out");
        advance(Duration::from_millis(1)).await;
        assert_eq!(w.http.offers_asked(), 2, "{reason:?}");
        assert_eq!(a.connections(), 2);
        w.admit(&a).await;
        assert_eq!(w.pcs.count(), 2);
    }
}

#[tokio::test(start_paused = true)]
async fn the_api_servers_end_of_the_calls_own_session_is_followed() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let other = Uuid::now_v7();
    w.send(Command::SessionEnded {
        session: other,
        channel: w.channel,
        reason: SessionEndReason::ServerLost,
    });
    w.send(Command::SessionEnded {
        session: w.session,
        channel: other,
        reason: SessionEndReason::ServerLost,
    });
    w.send(Command::SessionEnded {
        session: w.session,
        channel: w.channel,
        reason: SessionEndReason::Empty,
    });
    settle().await;
    assert_eq!(w.state().status, CallStatus::Connected);
    w.send(Command::SessionEnded {
        session: w.session,
        channel: w.channel,
        reason: SessionEndReason::ServerRemoved,
    });
    settle().await;
    assert_eq!(w.state().status, CallStatus::Rejoining);
    advance(PAUSE).await;
    assert_eq!(w.http.offers_asked(), 2);
    w.admit(&a).await;
    // The end of the session the rejoin left behind changes nothing; an idle end ends it.
    w.send(Command::SessionEnded {
        session: w.session,
        channel: w.channel,
        reason: SessionEndReason::Idle,
    });
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Idle);
    assert_eq!(state.ended_reason, Some(EndedReason::Idle));
    assert_eq!(w.http.offers_asked(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_rejoin_whose_fresh_offer_fails_fails_the_call() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    a.set(Behaviour::Refuse);
    w.send(Command::SessionEnded {
        session: w.session,
        channel: w.channel,
        reason: SessionEndReason::ServerLost,
    });
    settle().await;
    advance(PAUSE).await;
    assert_eq!(w.http.offers_asked(), 2);
    let state = w.state();
    assert_eq!(state.status, CallStatus::Failed);
    assert_eq!(state.error_kind, Some(ErrorKind::Server));
    assert_eq!(state.microphone, None);
    advance(DEAD_SERVER_RETRY).await;
    assert_eq!(
        w.http.offers_asked(),
        2,
        "a fresh offer is not asked for again"
    );
}

const RESTART: ClientMessage = ClientMessage::RestartIce {
    connection: ConnectionKind::Call,
};

#[tokio::test(start_paused = true)]
async fn a_lost_connection_asks_for_ice_restarts_every_interval_until_it_is_back() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    a.received();
    // Disconnected: the first restart waits an interval, since the network may come back alone.
    pc.report(ConnectionState::Disconnected);
    settle().await;
    assert!(a.received().is_empty());
    advance(ICE_RESTART_INTERVAL - Duration::from_millis(1)).await;
    assert!(a.received().is_empty());
    advance(Duration::from_millis(1)).await;
    assert_eq!(a.received(), vec![RESTART]);
    // Failing while already lost starts no second loop.
    pc.report(ConnectionState::Failed);
    settle().await;
    assert!(a.received().is_empty());
    // The server's own restart offer, and ours, are applied through it.
    pc.ops();
    a.offer(2, vec![slot("0", 0)]);
    settle().await;
    assert!(matches!(
        a.received().as_slice(),
        [ClientMessage::Answer { revision: 2, .. }]
    ));
    assert!(!pc.ops().is_empty());
    advance(ICE_RESTART_INTERVAL).await;
    assert_eq!(a.received(), vec![RESTART]);
    pc.report(ConnectionState::Connecting);
    pc.report(ConnectionState::Connected);
    settle().await;
    advance(ICE_RESTART_INTERVAL * 3).await;
    assert!(a.received().is_empty(), "back: no more restarts");
    assert_eq!(w.state().status, CallStatus::Connected);
    // Failed outright: the first restart is asked for at once.
    pc.report(ConnectionState::Failed);
    settle().await;
    assert_eq!(a.received(), vec![RESTART]);
    // The socket drops with the network and is resumed: the restarts go on over the new one.
    a.close();
    settle().await;
    advance(ICE_RESTART_INTERVAL).await;
    a.received();
    a.ready(w.session, w.user, Vec::new(), "secret-2");
    settle().await;
    assert_eq!(w.state().status, CallStatus::Connected);
    advance(ICE_RESTART_INTERVAL).await;
    assert_eq!(a.received(), vec![RESTART]);
    pc.report(ConnectionState::Connected);
    advance(ICE_RESTART_INTERVAL * 2).await;
    assert!(a.received().is_empty());
    assert_eq!(w.http.offers_asked(), 1, "never rejoined");
}

#[tokio::test(start_paused = true)]
async fn a_connection_lost_for_the_limit_is_rejoined() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    a.received();
    pc.report(ConnectionState::Failed);
    settle().await;
    let mut restarts = a.received().len();
    let mut elapsed = Duration::ZERO;
    while elapsed + ICE_RESTART_INTERVAL < ICE_RECOVERY_LIMIT {
        advance(ICE_RESTART_INTERVAL).await;
        elapsed += ICE_RESTART_INTERVAL;
        restarts += a.received().iter().filter(|f| **f == RESTART).count();
        assert_eq!(w.state().status, CallStatus::Connected, "at {elapsed:?}");
    }
    assert_eq!(restarts, 8, "eight restarts in half a minute");
    advance(ICE_RESTART_INTERVAL).await;
    assert_eq!(w.state().status, CallStatus::Rejoining);
    assert_eq!(a.received(), vec![ClientMessage::Leave]);
    assert!(pc.is_closed());
    advance(PAUSE).await;
    assert_eq!(w.http.offers_asked(), 2);
    w.admit(&a).await;
    // The old loss's ticks reach nothing.
    advance(ICE_RECOVERY_LIMIT).await;
    assert_eq!(w.state().status, CallStatus::Connected);
    assert!(a.received().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_rejoined_call_asks_for_its_videos_with_ready() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let bob = Uuid::now_v7();
    let watched = vec![WatchedVideo {
        user: bob,
        source: MediaSource::Camera,
    }];
    w.send(Command::SetWatched(watched.clone()));
    settle().await;
    a.received();
    a.send(&ServerMessage::Kicked {
        reason: KickReason::ServerStopping,
    });
    settle().await;
    advance(PAUSE).await;
    a.received();
    a.ready(w.session, w.user, Vec::new(), "secret-2");
    settle().await;
    let frames = a.received();
    assert!(
        frames.contains(&ClientMessage::Watch {
            video: watched.clone()
        }),
        "watched before any offer or connection: {frames:?}"
    );
    assert_eq!(w.state().watching, watched);
}

#[tokio::test(start_paused = true)]
async fn a_fatal_error_mid_call_fails_it() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    a.send(&error("the call is gone", true, None, None));
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Failed);
    assert_eq!(state.error.as_deref(), Some("the call is gone"));
    assert_eq!(state.channel, Some(w.channel));
    assert!(w.pcs.latest().is_closed());
    assert_eq!(w.http.offers_asked(), 1);
}

// --- Resuming -------------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn an_offer_restated_after_a_resume_gets_the_same_answer_and_no_negotiation() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    let answered = a.received();
    assert!(matches!(
        answered.as_slice(),
        [ClientMessage::Answer { revision: 2, .. }]
    ));
    pc.ops();
    // The socket drops before the answer reached the server, which offers revision 2 again.
    a.close();
    settle().await;
    a.received();
    a.ready(w.session, w.user, Vec::new(), "secret-2");
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    assert_eq!(a.received(), answered, "the very same answer frame");
    assert!(
        pc.ops().is_empty(),
        "nothing is asked of the peer connection"
    );
    assert_eq!(w.state().status, CallStatus::Connected);
    // The next revision is negotiated as usual.
    a.offer(
        3,
        vec![
            slot("0", 0),
            sending("1", MediaSource::Microphone),
            slot("2", 1),
        ],
    );
    settle().await;
    assert!(matches!(
        a.received().as_slice(),
        [ClientMessage::Answer { revision: 3, .. }]
    ));
    assert!(!pc.ops().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_watch_is_cut_to_the_videos_the_server_gives_sections_for() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    assert_eq!(w.state().max_watched_video, 25);
    let video: Vec<WatchedVideo> = (0..27)
        .map(|_| WatchedVideo {
            user: Uuid::now_v7(),
            source: MediaSource::Camera,
        })
        .collect();
    w.send(Command::SetWatched(video.clone()));
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::Watch {
            video: video[..25].to_vec()
        }]
    );
    assert_eq!(w.state().watching, video[..25]);
    w.send(Command::Leave);
    settle().await;
    assert_eq!(w.state().max_watched_video, 0);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_socket_is_resumed_within_the_grace_with_media_kept() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    a.received();
    a.close();
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Rejoining);
    assert_eq!(
        state.session,
        Some(w.session),
        "the place in the call is kept"
    );
    assert_eq!(a.connections(), 2);
    assert_eq!(
        a.received(),
        vec![ClientMessage::Resume {
            secret: "secret-1".to_string()
        }]
    );
    let bob = Uuid::now_v7();
    a.ready(
        w.session,
        w.user,
        vec![voice_protocol::signal::ParticipantInfo {
            user: bob,
            muted: false,
            deafened: false,
            held: false,
            speaking: false,
            sources: Vec::new(),
        }],
        "secret-2",
    );
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Connected);
    assert_eq!(state.participants[0].user, bob);
    assert!(!pc.is_closed());
    assert_eq!(w.pcs.count(), 1);
    assert_eq!(w.http.offers_asked(), 1);
    // The server restates the sections where no offer is outstanding.
    pc.ops();
    a.sections(vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    assert_eq!(w.state().slots[0].track, Some(pc.received("0")));
    assert!(pc.ops().is_empty(), "restating answers nothing");
    // The secret rotated with the resume.
    a.received();
    a.close();
    settle().await;
    assert_eq!(a.connections(), 3);
    assert_eq!(
        a.received(),
        vec![ClientMessage::Resume {
            secret: "secret-2".to_string()
        }]
    );
    // An offer still outstanding is sent again with `ready`, and answered.
    a.ready(w.session, w.user, Vec::new(), "secret-3");
    a.offer(3, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    assert!(matches!(
        a.received().as_slice(),
        [ClientMessage::Answer { revision: 3, .. }]
    ));
    assert_eq!(w.state().status, CallStatus::Connected);
}

#[tokio::test(start_paused = true)]
async fn a_resume_the_server_refuses_rejoins_at_once() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    a.close();
    settle().await;
    a.received();
    a.send(&error(
        "there is nothing to resume; join the call again",
        true,
        None,
        None,
    ));
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Rejoining);
    assert_eq!(state.session, None);
    assert!(w.pcs.latest().is_closed());
    assert!(a.is_closed());
    advance(PAUSE).await;
    assert_eq!(w.http.offers_asked(), 2);
    assert_eq!(a.connections(), 3);
    w.admit(&a).await;
    assert_eq!(w.state().microphone, Some(mic()));
}

#[tokio::test(start_paused = true)]
async fn a_rejoin_after_a_refused_resume_takes_the_server_as_alive() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    a.close();
    settle().await;
    a.send(&error(
        "there is nothing to resume; join the call again",
        true,
        None,
        None,
    ));
    settle().await;
    a.set(Behaviour::Refuse);
    advance(PAUSE).await;
    assert_eq!(w.http.offers_asked(), 2);
    assert_eq!(
        w.state().status,
        CallStatus::Failed,
        "the server answered, so no second offer is waited for"
    );
}

#[tokio::test(start_paused = true)]
async fn a_socket_that_opens_after_its_candidate_was_given_up_is_closed_unused() {
    let w = World::new(0.5);
    let (a, b) = (w.sockets.server(A), w.sockets.server(B));
    a.set(Behaviour::After(READY_TIMEOUT + Duration::from_secs(1)));
    b.set(Behaviour::After(Duration::from_secs(2)));
    let offer = Offer::to(w.channel, &[A, B]);
    // Both answer their health check at once, so the ranking takes no time.
    w.join(&offer).await;
    advance(READY_TIMEOUT).await;
    assert_eq!(w.http.failures_reported(), vec![offer.candidates[0].0]);
    assert_eq!(a.connections(), 0);
    // The first candidate's socket opens while the second's is still being opened: it is not
    // taken for the second's.
    advance(Duration::from_secs(1)).await;
    assert_eq!(a.connections(), 1);
    assert!(a.is_closed());
    assert!(
        a.received().is_empty(),
        "nothing is said to a server given up on"
    );
    assert_eq!(b.connections(), 0);
    advance(Duration::from_secs(1)).await;
    assert_eq!(b.connections(), 1);
    w.admit(&b).await;
    assert_eq!(w.state().status, CallStatus::Connected);
}

#[tokio::test(start_paused = true)]
async fn a_resume_that_cannot_reach_the_server_is_retried_within_the_grace_then_rejoins_twice() {
    let w = World::new(0.5);
    let (offer, a) = w.connected(A).await;
    a.set(Behaviour::Refuse);
    a.close();
    settle().await;
    assert_eq!(w.state().status, CallStatus::Rejoining);
    let tries = || w.sockets.connected.lock().unwrap().len() - 1;
    assert_eq!(tries(), 1);
    // Tried again every pause while a try could still finish within the grace.
    let mut elapsed = Duration::ZERO;
    while elapsed + PAUSE + PAUSE + READY_TIMEOUT <= RESUME_GRACE {
        advance(PAUSE).await;
        elapsed += PAUSE;
        assert_eq!(
            w.http.offers_asked(),
            1,
            "resuming, not rejoining, at {elapsed:?}"
        );
    }
    assert!(tries() > 2);
    assert_eq!(w.state().session, Some(w.session));
    // Past it, the channel is joined again as after a server that went silent, after a pause.
    advance(PAUSE).await;
    assert_eq!(w.state().session, None);
    assert!(w.pcs.latest().is_closed());
    assert_eq!(w.http.offers_asked(), 1);
    advance(PAUSE).await;
    assert_eq!(w.http.offers_asked(), 2);
    // The offer still names the dead server, which is reported; the rejoin waits for the API
    // server to find it silent and asks once more.
    assert_eq!(w.http.failures_reported(), vec![offer.candidates[0].0]);
    assert_eq!(w.state().status, CallStatus::Rejoining);
    assert_eq!(w.state().microphone, Some(mic()));
    advance(DEAD_SERVER_RETRY - Duration::from_millis(1)).await;
    assert_eq!(w.http.offers_asked(), 2);
    a.set(Behaviour::Scripted);
    advance(Duration::from_millis(1)).await;
    assert_eq!(w.http.offers_asked(), 3);
    w.admit(&a).await;
    assert_eq!(w.state().status, CallStatus::Connected);
}

#[tokio::test(start_paused = true)]
async fn a_resume_that_goes_unanswered_is_retried_and_a_second_failure_after_the_retry_fails() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    a.close();
    settle().await;
    assert_eq!(a.connections(), 2);
    advance(READY_TIMEOUT).await;
    assert!(a.is_closed(), "the unanswered resume socket is let go");
    advance(PAUSE).await;
    assert_eq!(a.connections(), 3);
    // Give up on resuming: the grace passes.
    a.set(Behaviour::Refuse);
    a.close();
    let mut elapsed = Duration::ZERO;
    while elapsed < RESUME_GRACE {
        advance(PAUSE).await;
        elapsed += PAUSE;
    }
    assert_eq!(w.http.offers_asked(), 2);
    advance(DEAD_SERVER_RETRY).await;
    assert_eq!(w.http.offers_asked(), 3);
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Failed);
    assert_eq!(state.error_kind, Some(ErrorKind::Server));
    assert_eq!(state.microphone, None);
}

#[tokio::test(start_paused = true)]
async fn a_socket_closing_before_media_connects_is_the_candidates_failure() {
    let w = World::new(0.5);
    let (a, b) = (w.sockets.server(A), w.sockets.server(B));
    let offer = Offer::to(w.channel, &[A, B]);
    w.join(&offer).await;
    advance(PING_TIMEOUT).await;
    a.ready(w.session, w.user, Vec::new(), "secret");
    settle().await;
    a.close();
    settle().await;
    assert_eq!(w.http.failures_reported(), vec![offer.candidates[0].0]);
    assert_eq!(a.connections(), 1, "no resume while joining");
    w.admit(&b).await;
}

// --- Silence, reused sections, and what the user asked -----------------------------------------

#[tokio::test(start_paused = true)]
async fn muting_someone_for_me_tells_the_server_to_silence_them() {
    let w = World::new(0.5);
    let (bob, carol) = (Uuid::now_v7(), Uuid::now_v7());
    let offer = Offer::to(w.channel, &[A]);
    let a = w.sockets.server(A);
    w.join(&offer).await;
    a.ready(
        w.session,
        w.user,
        vec![voice_protocol::signal::ParticipantInfo {
            user: bob,
            muted: false,
            deafened: false,
            held: false,
            speaking: false,
            sources: Vec::new(),
        }],
        "secret-1",
    );
    settle().await;
    let frames = a.received();
    assert!(
        !frames
            .iter()
            .any(|f| matches!(f, ClientMessage::Silence { .. })),
        "a new place silences no one, which needs no saying: {frames:?}"
    );
    // Quieter is not silent.
    w.send(Command::SetUserVolume {
        user: bob,
        source: MediaSource::Microphone,
        gain: 0.5,
    });
    settle().await;
    assert!(a.received().is_empty());
    w.send(Command::SetUserMuted {
        user: bob,
        source: MediaSource::Microphone,
        muted: true,
    });
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::Silence {
            voices: vec![bob],
            shares: Vec::new()
        }]
    );
    // Someone not in the call is not named; once they join, they are silenced from the start.
    w.send(Command::SetUserVolume {
        user: carol,
        source: MediaSource::ScreenAudio,
        gain: 0.0,
    });
    settle().await;
    assert!(a.received().is_empty());
    a.send(&ServerMessage::ParticipantJoined {
        user: carol,
        muted: false,
        deafened: false,
    });
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::Silence {
            voices: vec![bob],
            shares: vec![carol]
        }]
    );
    // A volume above nothing is no silence; an unchanged set is not sent again.
    w.send(Command::SetUserVolume {
        user: bob,
        source: MediaSource::Microphone,
        gain: 2.0,
    });
    settle().await;
    assert!(
        a.received().is_empty(),
        "muted for me still, whatever the volume"
    );
    w.send(Command::SetUserMuted {
        user: bob,
        source: MediaSource::Microphone,
        muted: false,
    });
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::Silence {
            voices: Vec::new(),
            shares: vec![carol]
        }]
    );
    a.send(&ServerMessage::ParticipantLeft { user: carol });
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::Silence {
            voices: Vec::new(),
            shares: Vec::new()
        }]
    );
}

#[tokio::test(start_paused = true)]
async fn the_silence_is_said_again_to_a_new_place_in_the_call_and_kept_through_a_resume() {
    let w = World::new(0.5);
    let bob = Uuid::now_v7();
    let (_, a) = w.connected(A).await;
    a.send(&ServerMessage::ParticipantJoined {
        user: bob,
        muted: false,
        deafened: false,
    });
    w.send(Command::SetUserMuted {
        user: bob,
        source: MediaSource::Microphone,
        muted: true,
    });
    settle().await;
    a.received();
    // Resumed: the server kept the silence, so it is not said again.
    a.close();
    settle().await;
    a.received();
    a.ready(
        w.session,
        w.user,
        vec![voice_protocol::signal::ParticipantInfo {
            user: bob,
            muted: false,
            deafened: false,
            held: false,
            speaking: false,
            sources: Vec::new(),
        }],
        "secret-2",
    );
    settle().await;
    assert!(a.received().is_empty());
    // Rejoined: a new place in the call starts with no one silenced, so it is said with `ready`.
    a.send(&ServerMessage::Kicked {
        reason: KickReason::ServerStopping,
    });
    settle().await;
    advance(PAUSE).await;
    a.received();
    a.ready(
        w.session,
        w.user,
        vec![voice_protocol::signal::ParticipantInfo {
            user: bob,
            muted: false,
            deafened: false,
            held: false,
            speaking: false,
            sources: Vec::new(),
        }],
        "secret-3",
    );
    settle().await;
    assert!(
        a.received().contains(&ClientMessage::Silence {
            voices: vec![bob],
            shares: Vec::new()
        }),
        "said before any offer"
    );
}

#[tokio::test(start_paused = true)]
async fn a_slot_moved_to_another_section_is_read_from_its_new_mid() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    let (bob, carol) = (Uuid::now_v7(), Uuid::now_v7());
    a.send(&ServerMessage::Slots {
        assignments: vec![SlotAssignment {
            slot: 0,
            user: Some(carol),
        }],
    });
    w.send(Command::SetUserVolume {
        user: carol,
        source: MediaSource::Microphone,
        gain: 2.0,
    });
    settle().await;
    assert_eq!(w.state().slots[0].track, Some(pc.received("0")));
    pc.ops();
    // The server moves slot 0 to mid 3 and gives mid 0 to Bob's camera.
    a.offer(2, vec![video("0", bob, MediaSource::Camera), slot("3", 0)]);
    settle().await;
    let state = w.state();
    assert_eq!(state.slots.len(), 1);
    assert_eq!(state.slots[0].track, Some(pc.received("3")));
    assert_eq!(state.slots[0].user, Some(carol));
    assert_eq!(state.videos[0].user, bob);
    assert_eq!(state.videos[0].track, pc.received("0"));
    assert!(state.share_audio.is_empty());
    assert!(
        pc.ops().contains(&Op::SetVolume("3".to_string(), 2.0)),
        "her volume follows her slot to its new section"
    );
}

#[tokio::test(start_paused = true)]
async fn a_video_section_reused_for_another_person_shows_that_person() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    let (bob, carol) = (Uuid::now_v7(), Uuid::now_v7());
    w.send(Command::SetUserMuted {
        user: carol,
        source: MediaSource::ScreenAudio,
        muted: true,
    });
    a.offer(
        2,
        vec![
            slot("0", 0),
            video("1", bob, MediaSource::Screen),
            share_audio("2", bob),
        ],
    );
    settle().await;
    let state = w.state();
    assert_eq!(state.videos[0].user, bob);
    assert_eq!(state.share_audio[0].user, bob);
    pc.ops();
    // Bob stops sharing and Carol starts: the server reuses both sections for her.
    a.offer(
        3,
        vec![
            slot("0", 0),
            video("1", carol, MediaSource::Screen),
            share_audio("2", carol),
        ],
    );
    settle().await;
    let state = w.state();
    assert_eq!(state.videos.len(), 1);
    assert_eq!(state.videos[0].user, carol);
    assert_eq!(state.videos[0].track, pc.received("1"));
    assert_eq!(state.share_audio.len(), 1);
    assert_eq!(state.share_audio[0].user, carol);
    assert!(
        pc.ops().contains(&Op::SetVolume("2".to_string(), 0.0)),
        "the reused section takes its new person's volume"
    );
}

#[tokio::test(start_paused = true)]
async fn a_hold_shows_at_once_and_is_lifted_only_on_the_servers_word() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    let pc = w.pcs.latest();
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    a.received();
    pc.ops();
    w.send(Command::SetMuted(true));
    settle().await;
    a.received();
    pc.ops();

    // Held beside the user's own mute: shown held at once, the frame carrying the mute and
    // deafen as they stand, and the microphone already off its sender for the mute.
    w.send(Command::SetHeld(true));
    settle().await;
    let state = w.state();
    assert!(state.held && state.muted && !state.deafened);
    assert_eq!(
        a.received(),
        vec![ClientMessage::SetState {
            muted: true,
            deafened: false,
            held: true
        }]
    );
    assert!(pc.ops().is_empty());
    // The server confirms it, the mute kept beside.
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: true,
        deafened: false,
        held: true,
    });
    settle().await;
    assert!(w.state().held && w.state().muted);

    // Releasing waits for the server; the user's own mute stands after it.
    w.send(Command::SetHeld(false));
    settle().await;
    assert!(w.state().held, "the release waits for the server");
    assert_eq!(
        a.received(),
        vec![ClientMessage::SetState {
            muted: true,
            deafened: false,
            held: false
        }]
    );
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: true,
        deafened: false,
        held: false,
    });
    settle().await;
    let state = w.state();
    assert!(!state.held && state.muted && !state.deafened);
    assert!(
        pc.ops().is_empty(),
        "still muted, the microphone stays off its sender"
    );

    // Held while unmuted: the microphone leaves its sender for the hold alone, and comes back
    // when the server lifts it.
    w.send(Command::SetMuted(false));
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: false,
        deafened: false,
        held: false,
    });
    settle().await;
    assert_eq!(
        pc.ops(),
        vec![Op::ReplaceTrack("1".to_string(), Some(mic()))]
    );
    a.received();
    w.send(Command::SetHeld(true));
    settle().await;
    assert_eq!(pc.ops(), vec![Op::ReplaceTrack("1".to_string(), None)]);
    assert_eq!(
        a.received(),
        vec![ClientMessage::SetState {
            muted: false,
            deafened: false,
            held: true
        }]
    );
    // A `participantState` answering the earlier release never shows the user off hold once
    // they have asked for it again.
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: false,
        deafened: false,
        held: false,
    });
    settle().await;
    assert!(w.state().held);
    assert!(pc.ops().is_empty());
    w.send(Command::SetHeld(false));
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: false,
        deafened: false,
        held: false,
    });
    settle().await;
    assert!(!w.state().held);
    assert_eq!(
        pc.ops(),
        vec![Op::ReplaceTrack("1".to_string(), Some(mic()))]
    );
}

#[tokio::test(start_paused = true)]
async fn outside_a_call_the_hold_simply_is_the_state_and_a_join_carries_it() {
    let w = World::new(0.5);
    w.send(Command::SetHeld(true));
    settle().await;
    assert!(w.state().held);
    w.send(Command::SetHeld(false));
    settle().await;
    assert!(!w.state().held, "nothing to wait for outside a call");
    // Held before joining (a phone call under way): the join says so, and the microphone's
    // sender carries nothing from the start.
    w.send(Command::SetHeld(true));
    settle().await;
    let offer = Offer::to(w.channel, &[A]);
    let a = w.sockets.server(A);
    w.join(&offer).await;
    let frames = w.admit(&a).await;
    assert!(frames.contains(&ClientMessage::SetState {
        muted: false,
        deafened: false,
        held: true
    }));
    assert!(w.state().held);
    a.offer(2, vec![slot("0", 0), sending("1", MediaSource::Microphone)]);
    settle().await;
    assert!(
        w.pcs
            .latest()
            .ops()
            .contains(&Op::ReplaceTrack("1".to_string(), None))
    );
}

#[tokio::test(start_paused = true)]
async fn a_refused_release_leaves_the_call_on_hold() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.send(Command::SetHeld(true));
    settle().await;
    w.send(Command::SetHeld(false));
    settle().await;
    a.received();
    a.send(&error(
        "too many setState frames",
        false,
        Some(3),
        Some("setState"),
    ));
    settle().await;
    assert!(w.state().held);
    assert_eq!(
        w.state()
            .state_refused
            .as_ref()
            .map(|r| r.retry_after_seconds),
        Some(Some(3))
    );
    // The next request carries the hold as it stands, not the refused release.
    w.send(Command::SetDeafened(true));
    settle().await;
    assert_eq!(
        a.received(),
        vec![ClientMessage::SetState {
            muted: false,
            deafened: true,
            held: true
        }]
    );
    w.send(Command::Leave);
    settle().await;
    assert!(!w.state().held, "idle, nothing is held");
}

#[tokio::test(start_paused = true)]
async fn the_others_on_hold_follow_ready_and_participant_state() {
    let w = World::new(0.5);
    let (bob, carol) = (Uuid::now_v7(), Uuid::now_v7());
    let offer = Offer::to(w.channel, &[A]);
    let a = w.sockets.server(A);
    w.join(&offer).await;
    a.ready(
        w.session,
        w.user,
        vec![
            voice_protocol::signal::ParticipantInfo {
                user: bob,
                muted: false,
                deafened: false,
                held: true,
                speaking: false,
                sources: Vec::new(),
            },
            voice_protocol::signal::ParticipantInfo {
                user: carol,
                muted: false,
                deafened: false,
                held: false,
                speaking: false,
                sources: Vec::new(),
            },
        ],
        "secret",
    );
    settle().await;
    let held = |state: &voice_client::state::CallState| -> Vec<Uuid> {
        state
            .participants
            .iter()
            .filter(|p| p.held)
            .map(|p| p.user)
            .collect()
    };
    assert_eq!(held(&w.state()), vec![bob]);
    assert!(!w.state().held, "another's hold is not the user's");
    // Carol is put on hold and Bob taken off it; their mute and deafen come with the frame.
    a.send(&ServerMessage::ParticipantState {
        user: carol,
        muted: true,
        deafened: false,
        held: true,
    });
    a.send(&ServerMessage::ParticipantState {
        user: bob,
        muted: false,
        deafened: false,
        held: false,
    });
    settle().await;
    assert_eq!(held(&w.state()), vec![carol]);
    assert!(
        w.state()
            .participants
            .iter()
            .any(|p| p.user == carol && p.muted)
    );
    // A `participantState` without `held` (an older server) reads as not held, and someone
    // joining anew is not on hold.
    a.send_text(format!(
        r#"{{"type":"participantState","user":"{carol}","muted":true,"deafened":false}}"#
    ));
    a.send(&ServerMessage::ParticipantJoined {
        user: bob,
        muted: false,
        deafened: false,
    });
    settle().await;
    assert!(held(&w.state()).is_empty());
    assert!(!w.state().held);
}

#[tokio::test(start_paused = true)]
async fn a_ready_after_a_resume_carries_who_is_on_hold() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.send(Command::SetHeld(true));
    settle().await;
    a.received();
    a.close();
    settle().await;
    assert_eq!(w.state().status, CallStatus::Rejoining);
    assert!(w.state().held, "the hold stands through the resume");
    let bob = Uuid::now_v7();
    a.ready(
        w.session,
        w.user,
        vec![voice_protocol::signal::ParticipantInfo {
            user: bob,
            muted: false,
            deafened: true,
            held: true,
            speaking: false,
            sources: Vec::new(),
        }],
        "secret-2",
    );
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Connected);
    assert!(state.participants[0].held && state.participants[0].deafened);
    assert!(
        state.held,
        "the server keeps the participant's hold across a resume"
    );
    // Nothing is asked again on the resumed socket: the server kept the state.
    assert!(
        !a.received()
            .iter()
            .any(|f| matches!(f, ClientMessage::SetState { .. }))
    );
}
