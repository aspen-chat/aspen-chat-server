//! The call's sounds as the observer is told to play them, from the engine's own transitions
//! against the scripted fakes: one scenario for each cue the engine gives (the transfer cues'
//! from the server's link frames and a file received whole through the fake data channel), and
//! the changes that give none (a socket dropped and taken back within the grace, the user's own
//! state echoed back, a snapshot read again).

mod support;

use std::time::Duration;
use support::*;
use uuid::Uuid;
use voice_client::call::{Command, READY_TIMEOUT, RESUME_GRACE};
use voice_client::peer_connection::{ConnectionState, TrackId};
use voice_client::sounds::SoundCue;
use voice_client::state::{CallStatus, EndedReason, SessionEndReason, TransferStatus};
use voice_protocol::signal::{
    ClientMessage, KickReason, MediaSource, ParticipantInfo, ServerMessage, SourceInfo,
    TransferLink, TransferMode, TransferRole,
};

const A: &str = "https://a.example.org";
const B: &str = "https://b.example.org";

/// The pause a `World::new(0.5)` takes before a rejoin or a resume retry.
const PAUSE: Duration = Duration::from_millis(500);

fn person(user: Uuid, sources: &[MediaSource]) -> ParticipantInfo {
    ParticipantInfo {
        user,
        muted: false,
        deafened: false,
        held: false,
        speaking: false,
        sources: sources
            .iter()
            .map(|&source| SourceInfo {
                source,
                paused: false,
            })
            .collect(),
    }
}

#[tokio::test(start_paused = true)]
async fn connecting_is_a_joining_and_who_is_there_already_is_taken_as_found() {
    let w = World::new(0.5);
    let bob = Uuid::now_v7();
    let offer = Offer::to(w.channel, &[A]);
    let a = w.sockets.server(A);
    w.join(&offer).await;
    assert_eq!(w.sounds(), vec![], "joining itself is silent");
    a.ready(
        w.session,
        w.user,
        vec![person(bob, &[MediaSource::Screen])],
        "secret-1",
    );
    settle().await;
    a.offer(1, vec![slot("0", 0)]);
    settle().await;
    assert_eq!(
        w.sounds(),
        vec![],
        "nothing is said before the media connects"
    );
    w.pcs.latest().report(ConnectionState::Connected);
    settle().await;
    assert_eq!(w.state().status, CallStatus::Connected);
    assert_eq!(w.sounds(), vec![SoundCue::CallJoined]);
    // Bob's share was there as the call connected.
    a.send(&ServerMessage::Sources {
        user: bob,
        sources: vec![
            SourceInfo {
                source: MediaSource::Screen,
                paused: false,
            },
            SourceInfo {
                source: MediaSource::Camera,
                paused: false,
            },
        ],
    });
    settle().await;
    assert_eq!(w.sounds(), vec![]);
}

#[tokio::test(start_paused = true)]
async fn others_joining_leaving_and_sharing_are_heard_unless_deafened() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    assert_eq!(w.sounds(), vec![SoundCue::CallJoined]);
    // Someone joining as the call connects is heard as part of the connection itself.
    let (bob, carol) = (Uuid::now_v7(), Uuid::now_v7());
    a.send(&ServerMessage::ParticipantJoined {
        user: Uuid::now_v7(),
        muted: false,
        deafened: false,
    });
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    advance(Duration::from_secs(1)).await;
    a.send(&ServerMessage::ParticipantJoined {
        user: bob,
        muted: false,
        deafened: false,
    });
    settle().await;
    assert_eq!(w.sounds(), vec![SoundCue::CallJoined]);
    // Within the gap, a second join is heard as part of the first.
    a.send(&ServerMessage::ParticipantJoined {
        user: carol,
        muted: false,
        deafened: false,
    });
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    advance(Duration::from_secs(1)).await;
    // Bob joining again from another device is no one new.
    a.send(&ServerMessage::ParticipantJoined {
        user: bob,
        muted: true,
        deafened: false,
    });
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    a.send(&ServerMessage::Sources {
        user: carol,
        sources: vec![SourceInfo {
            source: MediaSource::Screen,
            paused: false,
        }],
    });
    settle().await;
    assert_eq!(w.sounds(), vec![SoundCue::StreamStarted]);
    a.send(&ServerMessage::ParticipantLeft { user: carol });
    settle().await;
    assert_eq!(w.sounds(), vec![SoundCue::CallLeft]);
    advance(Duration::from_secs(1)).await;
    // Deafened: the others' changes are taken in silently, and stay silent once undeafened.
    w.send(Command::SetDeafened(true));
    settle().await;
    a.send(&ServerMessage::ParticipantJoined {
        user: carol,
        muted: false,
        deafened: false,
    });
    a.send(&ServerMessage::ParticipantLeft { user: bob });
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    a.received();
    w.send(Command::SetDeafened(false));
    settle().await;
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: false,
        deafened: false,
        held: false,
    });
    settle().await;
    assert!(!w.state().deafened);
    assert_eq!(w.sounds(), vec![]);
    advance(Duration::from_secs(1)).await;
    a.send(&ServerMessage::ParticipantLeft { user: carol });
    settle().await;
    assert_eq!(w.sounds(), vec![SoundCue::CallLeft]);
}

#[tokio::test(start_paused = true)]
async fn the_users_own_state_echoed_back_is_no_sound() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.sounds();
    advance(Duration::from_secs(1)).await;
    // The server lists the user among the participants, speaks of their state, and tells of
    // their own sources: none of it is anyone else coming or going.
    a.send(&ServerMessage::ParticipantJoined {
        user: w.user,
        muted: false,
        deafened: false,
    });
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: true,
        deafened: false,
        held: false,
    });
    a.send(&ServerMessage::Sources {
        user: w.user,
        sources: vec![SourceInfo {
            source: MediaSource::Screen,
            paused: false,
        }],
    });
    a.send(&ServerMessage::Speaking {
        user: w.user,
        speaking: true,
    });
    settle().await;
    assert!(w.state().muted);
    assert_eq!(w.sounds(), vec![]);
    w.send(Command::SetMuted(false));
    settle().await;
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: false,
        deafened: false,
        held: false,
    });
    settle().await;
    assert!(!w.state().muted);
    assert_eq!(w.sounds(), vec![]);
}

#[tokio::test(start_paused = true)]
async fn leaving_is_left_and_so_is_another_device_taking_the_call_over() {
    let w = World::new(0.5);
    let (_, _) = w.connected(A).await;
    w.sounds();
    w.send(Command::Leave);
    settle().await;
    assert_eq!(w.state().status, CallStatus::Idle);
    assert_eq!(w.sounds(), vec![SoundCue::CallLeft]);
    w.send(Command::Leave);
    settle().await;
    assert_eq!(w.sounds(), vec![], "leaving nothing says nothing");
    for reason in [KickReason::Replaced, KickReason::SignedOut] {
        let w = World::new(0.5);
        let (_, a) = w.connected(A).await;
        w.sounds();
        a.send(&ServerMessage::Kicked { reason });
        settle().await;
        assert_eq!(w.state().ended_reason, None);
        assert_eq!(w.sounds(), vec![SoundCue::CallLeft], "{reason:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn an_end_the_call_bar_explains_is_a_disconnection() {
    for (reason, ended) in [
        (KickReason::Kicked, EndedReason::Kicked),
        (KickReason::AccessLost, EndedReason::AccessLost),
    ] {
        let w = World::new(0.5);
        let (_, a) = w.connected(A).await;
        w.sounds();
        a.send(&ServerMessage::Kicked { reason });
        settle().await;
        assert_eq!(w.state().ended_reason, Some(ended));
        assert_eq!(w.sounds(), vec![SoundCue::Disconnected], "{reason:?}");
        w.send(Command::AcknowledgeEnd);
        settle().await;
        assert_eq!(w.sounds(), vec![]);
    }
    let w = World::new(0.5);
    let (_, _) = w.connected(A).await;
    w.sounds();
    w.send(Command::SessionEnded {
        session: w.session,
        channel: w.channel,
        reason: SessionEndReason::Idle,
    });
    settle().await;
    assert_eq!(w.state().ended_reason, Some(EndedReason::Idle));
    assert_eq!(w.sounds(), vec![SoundCue::Disconnected]);
    // A fatal error mid-call fails it: lost under the user too.
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.sounds();
    a.send(&ServerMessage::Error {
        detail: "the server is going away".to_string(),
        fatal: true,
        retry_after_seconds: None,
        refused: None,
    });
    settle().await;
    assert_eq!(w.state().status, CallStatus::Failed);
    assert_eq!(w.sounds(), vec![SoundCue::Disconnected]);
}

#[tokio::test(start_paused = true)]
async fn a_dropped_socket_taken_back_within_the_grace_is_no_sound() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.sounds();
    let bob = Uuid::now_v7();
    a.send(&ServerMessage::ParticipantJoined {
        user: bob,
        muted: false,
        deafened: false,
    });
    settle().await;
    w.sounds();
    // The call stays connected while the socket is taken back, as the web client's does, so
    // the drop is no loss.
    tokio::time::advance(Duration::from_secs(1)).await;
    a.close();
    settle().await;
    assert_eq!(w.state().status, CallStatus::Connected);
    assert_eq!(w.sounds(), vec![]);
    // Nor is the resume's `ready` a return; it restates who is there, Bob gone meanwhile and
    // Carol come, which is heard as any leaving and joining of a connected call, as the web
    // client hears them from its event stream while its socket is down.
    let carol = Uuid::now_v7();
    a.ready(w.session, w.user, vec![person(carol, &[])], "secret-2");
    settle().await;
    assert_eq!(w.state().status, CallStatus::Connected);
    assert_eq!(
        w.sounds(),
        vec![SoundCue::CallJoined, SoundCue::CallLeft],
        "Carol came and Bob went while the socket was down"
    );
    // Taken back again with no one come or gone: silent.
    tokio::time::advance(Duration::from_secs(1)).await;
    a.received();
    a.close();
    settle().await;
    a.ready(w.session, w.user, vec![person(carol, &[])], "secret-3");
    settle().await;
    assert_eq!(w.state().status, CallStatus::Connected);
    assert_eq!(w.sounds(), vec![]);
    // The sections restated, and the user's own state answered, change nothing heard.
    a.sections(vec![slot("0", 0)]);
    a.send(&ServerMessage::ParticipantState {
        user: w.user,
        muted: false,
        deafened: false,
        held: false,
    });
    settle().await;
    assert_eq!(w.sounds(), vec![]);
}

#[tokio::test(start_paused = true)]
async fn a_resume_the_server_refuses_is_a_disconnection_and_its_rejoin_a_reconnection() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.sounds();
    a.close();
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    a.send(&ServerMessage::Error {
        detail: "there is nothing to resume; join the call again".to_string(),
        fatal: true,
        retry_after_seconds: None,
        refused: None,
    });
    settle().await;
    assert_eq!(w.state().status, CallStatus::Rejoining);
    assert_eq!(w.sounds(), vec![SoundCue::Disconnected]);
    advance(PAUSE).await;
    assert_eq!(w.http.offers_asked(), 2);
    w.admit(&a).await;
    assert_eq!(w.state().status, CallStatus::Connected);
    assert_eq!(w.sounds(), vec![SoundCue::Reconnected]);
    // Refused again within half a minute: said once.
    a.close();
    settle().await;
    a.send(&ServerMessage::Error {
        detail: "there is nothing to resume; join the call again".to_string(),
        fatal: true,
        retry_after_seconds: None,
        refused: None,
    });
    settle().await;
    assert_eq!(w.state().status, CallStatus::Rejoining);
    assert_eq!(w.sounds(), vec![]);
    advance(PAUSE).await;
    w.admit(&a).await;
    assert_eq!(w.state().status, CallStatus::Connected);
    assert_eq!(w.sounds(), vec![]);
}

#[tokio::test(start_paused = true)]
async fn a_resume_that_fails_for_the_whole_grace_is_a_disconnection_at_its_end() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.sounds();
    a.set(Behaviour::Refuse);
    a.close();
    settle().await;
    // Tried again and again within the grace, silently.
    let mut elapsed = Duration::ZERO;
    while elapsed + PAUSE + PAUSE + READY_TIMEOUT <= RESUME_GRACE {
        advance(PAUSE).await;
        elapsed += PAUSE;
        assert_eq!(w.state().status, CallStatus::Connected, "at {elapsed:?}");
        assert_eq!(w.sounds(), vec![], "at {elapsed:?}");
    }
    // Past it, the call is lost.
    advance(PAUSE).await;
    assert_eq!(w.state().status, CallStatus::Rejoining);
    assert_eq!(w.sounds(), vec![SoundCue::Disconnected]);
}

#[tokio::test(start_paused = true)]
async fn a_lost_server_is_a_disconnection_and_its_rejoin_a_reconnection() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.sounds();
    a.send(&ServerMessage::Kicked {
        reason: KickReason::ServerStopping,
    });
    settle().await;
    assert_eq!(w.state().status, CallStatus::Rejoining);
    assert_eq!(w.sounds(), vec![SoundCue::Disconnected]);
    advance(PAUSE).await;
    assert_eq!(w.http.offers_asked(), 2);
    w.admit(&a).await;
    assert_eq!(w.sounds(), vec![SoundCue::Reconnected]);
}

#[tokio::test(start_paused = true)]
async fn a_rejoin_that_fails_adds_nothing_to_the_disconnection() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.sounds();
    w.http
        .answer_join(problem(403, "forbidden", "you may not join"));
    a.send(&ServerMessage::Kicked {
        reason: KickReason::ServerStopping,
    });
    settle().await;
    assert_eq!(w.sounds(), vec![SoundCue::Disconnected]);
    advance(PAUSE).await;
    assert_eq!(w.state().status, CallStatus::Failed);
    assert_eq!(w.sounds(), vec![]);
}

#[tokio::test(start_paused = true)]
async fn moving_to_another_call_is_only_a_joining() {
    let w = World::new(0.5);
    let (_, _) = w.connected(A).await;
    w.sounds();
    advance(Duration::from_secs(1)).await;
    let second = Uuid::now_v7();
    let b = w.sockets.server(B);
    w.http.answer_join(Offer::to(second, &[B]).response());
    w.send(Command::Join { channel: second });
    settle().await;
    assert_eq!(w.state().status, CallStatus::Joining);
    assert_eq!(w.sounds(), vec![]);
    w.admit(&b).await;
    assert_eq!(w.sounds(), vec![SoundCue::CallJoined]);
}

#[tokio::test(start_paused = true)]
async fn the_users_own_share_starting_is_heard() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.sounds();
    w.send(Command::StartSource {
        source: MediaSource::Screen,
        track: TrackId("screen-1".to_string()),
    });
    w.send(Command::StartSource {
        source: MediaSource::ScreenAudio,
        track: TrackId("screen-audio-1".to_string()),
    });
    settle().await;
    assert_eq!(w.sounds(), vec![SoundCue::StreamStarted]);
    a.send(&ServerMessage::Sending {
        source: MediaSource::Screen,
    });
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    w.send(Command::StopSource(MediaSource::Screen));
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    // A camera is no stream.
    w.send(Command::StartSource {
        source: MediaSource::Camera,
        track: TrackId("camera-1".to_string()),
    });
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    assert!(a.received().contains(&ClientMessage::Send {
        source: MediaSource::Camera,
        external: false,
    }));
}

#[tokio::test(start_paused = true)]
async fn a_join_that_fails_says_nothing() {
    let w = World::new(0.5);
    w.http
        .answer_join(problem(403, "forbidden", "you may not join"));
    w.send(Command::SetMicrophone(Some(World::microphone())));
    w.send(Command::Join { channel: w.channel });
    settle().await;
    assert_eq!(w.state().status, CallStatus::Failed);
    assert_eq!(w.sounds(), vec![]);
}

/// `callSounds.test.ts`: "plays transferAccepted as transfers begin, for everyone's but not
/// while deafened unless its own" and "plays transferComplete as the user's transfers arrive".
#[tokio::test(start_paused = true)]
async fn a_transfer_beginning_is_an_acceptance_and_one_arrived_whole_a_completion() {
    let w = World::new(0.5);
    let (_, a) = w.connected(A).await;
    w.sounds();
    let (bob, carol) = (Uuid::now_v7(), Uuid::now_v7());
    let link = |sender, receiver, active| ServerMessage::TransferLinkChanged {
        link: TransferLink { sender, receiver },
        active,
    };
    // A transfer between two others begins: heard, as everyone in the call hears an acceptance.
    a.send(&link(bob, carol, true));
    settle().await;
    assert_eq!(w.sounds(), vec![SoundCue::TransferAccepted]);
    // Another within the gap is heard as part of the first; one ending says nothing.
    a.send(&link(carol, bob, true));
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    advance(Duration::from_secs(1)).await;
    a.send(&link(bob, carol, false));
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    // Deafened, others' transfers are silent, and one the user is a side of is heard.
    w.send(Command::SetDeafened(true));
    settle().await;
    w.sounds();
    a.send(&link(bob, carol, true));
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    advance(Duration::from_secs(1)).await;
    a.send(&link(bob, w.user, true));
    settle().await;
    assert_eq!(w.sounds(), vec![SoundCue::TransferAccepted]);
    // The file Bob sends arrives whole, through the channel he opens: a completion, heard though
    // the user is deafened, since it is their own.
    advance(Duration::from_secs(1)).await;
    let offer = Uuid::now_v7();
    let bytes: Vec<u8> = (0..10u8).collect();
    w.files.add_destination("chosen-1");
    a.send(&offered(offer, bob, "a.bin", 10, true, 60_000));
    settle().await;
    w.send(Command::AcceptOffer {
        offer,
        mode: TransferMode::DirectPreferred,
        destination: "chosen-1".to_string(),
    });
    settle().await;
    a.send(&starting(
        offer,
        bob,
        TransferRole::Receiver,
        TransferMode::DirectPreferred,
        "a.bin",
        10,
    ));
    settle().await;
    let pc = w.pcs.latest();
    a.send(&signal(offer, bob, description("offer", "their offer")));
    settle().await;
    pc.report(ConnectionState::Connected);
    let channel = channel_arrives(&pc);
    settle().await;
    assert_eq!(
        w.sounds(),
        vec![],
        "a transfer moving is no sound of its own"
    );
    deliver_all(&channel, &bytes);
    settle().await;
    assert_eq!(
        w.state().files.transfers[0].status,
        TransferStatus::Completed
    );
    assert_eq!(w.sounds(), vec![SoundCue::TransferComplete]);
    // Kept until dismissed, it is not heard again, nor is its link's end or its dismissal,
    // past the gap.
    advance(Duration::from_secs(1)).await;
    a.send(&link(bob, w.user, false));
    settle().await;
    assert_eq!(w.sounds(), vec![]);
    w.send(Command::DismissTransfer { offer, peer: bob });
    settle().await;
    assert_eq!(w.sounds(), vec![]);
}
