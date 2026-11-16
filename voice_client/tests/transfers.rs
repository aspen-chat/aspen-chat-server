//! Scenarios of the call's file transfers against a scripted voice server, with the other side
//! of each transfer played by the test through the fake peer connection and its data channels,
//! on a paused clock. Each scenario of the web client's `transfers.test.ts` has its counterpart
//! here, and the engine's own paths (files that cannot be opened, that changed, backpressure,
//! the verdict's grace, expiry) their own.

mod support;

use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use support::*;
use uuid::Uuid;
use voice_client::call::Command;
use voice_client::peer_connection::{ConnectionState, DescriptionKind, SessionDescription};
use voice_client::state::{
    CallStatus, EndedBy, FileCommand, TransferRoute, TransferState, TransferStatus,
};
use voice_client::transfers::{
    BUFFER_HIGH_BYTES, BUFFER_LOW_BYTES, CHUNK_BYTES, MAX_OFFER_SECONDS, MIN_OFFER_SECONDS,
    PROGRESS_INTERVAL, ROUTE_INTERVAL, VERDICT_GRACE,
};
use voice_protocol::signal::{
    ClientMessage, FileOffer, Grants, OfferEnd, ServerMessage, TransferEnd, TransferLink,
    TransferMode, TransferPolicy, TransferRole,
};

const A: &str = "https://a.example.org";
const REF: &str = "picked-1";
const DEST: &str = "chosen-1";

/// Bytes that are not all alike, so a chunk out of place would show.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn candidate(candidate: &str) -> Value {
    json!({ "kind": "candidate", "candidate": { "candidate": candidate, "sdpMid": "0", "sdpMLineIndex": 0 } })
}

fn ended(offer: Uuid, peer: Uuid, reason: TransferEnd) -> ServerMessage {
    ServerMessage::TransferEnded {
        offer,
        peer,
        reason,
    }
}

fn withdrawn(offer: Uuid, reason: OfferEnd) -> ServerMessage {
    ServerMessage::FileWithdrawn { offer, reason }
}

/// The transfer signals the client sent, as `(peer, signal)`.
fn signals_sent(frames: &[ClientMessage]) -> Vec<(Uuid, Value)> {
    frames
        .iter()
        .filter_map(|frame| match frame {
            ClientMessage::TransferSignal { peer, signal, .. } => Some((*peer, signal.clone())),
            _ => None,
        })
        .collect()
}

fn transfer_of(w: &World) -> TransferState {
    w.state()
        .files
        .transfers
        .first()
        .cloned()
        .expect("a transfer")
}

/// A call connected at `A`, with the file `REF` holding `bytes` to offer and the place `DEST`
/// chosen for receiving.
async fn in_call(bytes: Vec<u8>) -> (World, Arc<FakeServer>, Arc<FakeFile>, Arc<FakeSink>) {
    let w = World::new(0.5);
    let (_, server) = w.connected(A).await;
    let file = w.files.add(REF, bytes);
    let sink = w.files.add_destination(DEST);
    (w, server, file, sink)
}

/// Offers `REF` as `name` and has the server tell of it; the frames the client sent for it.
async fn offer_file(
    w: &World,
    server: &FakeServer,
    offer: Uuid,
    name: &str,
    valid: u32,
) -> Vec<ClientMessage> {
    w.send(Command::OfferFile {
        offer,
        reference: REF.to_string(),
        name: name.to_string(),
        allow_direct: true,
        valid_for_seconds: valid,
    });
    settle().await;
    let frames = server.received();
    let size = frames
        .iter()
        .find_map(|frame| match frame {
            ClientMessage::OfferFile { size, .. } => Some(*size),
            _ => None,
        })
        .expect("offerFile was sent");
    server.send(&offered(
        offer,
        w.user,
        name,
        size,
        true,
        u64::from(valid) * 1000,
    ));
    settle().await;
    frames
}

/// Someone else's offer told by the server, accepted into `DEST`, and the transfer started: the
/// receiver's connection, with the other side's offer applied and answered.
async fn receiving(
    w: &World,
    server: &FakeServer,
    offer: Uuid,
    from: Uuid,
    name: &str,
    size: u64,
    mode: TransferMode,
) -> Arc<FakePeerConnection> {
    server.send(&offered(offer, from, name, size, true, 60_000));
    settle().await;
    w.send(Command::AcceptOffer {
        offer,
        mode,
        destination: DEST.to_string(),
    });
    settle().await;
    assert!(
        server
            .received()
            .contains(&ClientMessage::AcceptFile { offer, mode }),
        "acceptFile is sent once the place is open"
    );
    server.send(&starting(
        offer,
        from,
        TransferRole::Receiver,
        mode,
        name,
        size,
    ));
    settle().await;
    let pc = w.pcs.latest();
    server.send(&signal(offer, from, description("offer", "their offer")));
    settle().await;
    pc
}

#[test]
fn the_limits_are_as_documented() {
    assert_eq!(MIN_OFFER_SECONDS, 10);
    assert_eq!(MAX_OFFER_SECONDS, 60 * 60);
    assert_eq!(CHUNK_BYTES, 64 * 1024);
    assert_eq!(BUFFER_HIGH_BYTES, 4 * 1024 * 1024);
    assert_eq!(BUFFER_LOW_BYTES, 1024 * 1024);
    assert_eq!(PROGRESS_INTERVAL, Duration::from_millis(200));
    assert_eq!(VERDICT_GRACE, Duration::from_secs(5));
    assert_eq!(ROUTE_INTERVAL, Duration::from_secs(3));
}

// --- The web client's scenarios ---------------------------------------------------------------

/// `transfers.test.ts`: "sends an offered file to whoever accepts it", the sender's side.
#[tokio::test(start_paused = true)]
async fn an_offered_file_is_sent_to_whoever_accepts_it() {
    let bytes = pattern(200_000);
    let (w, server, file, _) = in_call(bytes.clone()).await;
    let offer = Uuid::now_v7();
    let bob = Uuid::now_v7();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let frames = offer_file(&w, &server, offer, "notes.bin", 60).await;
    assert!(frames.contains(&ClientMessage::OfferFile {
        offer,
        name: "notes.bin".to_string(),
        size: 200_000,
        allow_direct: true,
        valid_for_seconds: 60,
    }));
    // The file was opened for its size and closed again; nothing was read.
    assert_eq!(file.opened.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        w.files
            .latest_source()
            .closed
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    let state = w.state();
    assert!(state.grants.transfer_files);
    let standing = &state.files.offers[0];
    assert_eq!(
        (standing.id, standing.from, standing.own),
        (offer, w.user, true)
    );
    assert_eq!(
        (standing.name.as_str(), standing.size, standing.allow_direct),
        ("notes.bin", 200_000, true)
    );
    assert!(
        standing.expires_at_ms >= before + 60_000,
        "{} < {}",
        standing.expires_at_ms,
        before + 60_000
    );

    // Bob accepts: the sender opens a connection with the server's ICE servers, a channel, and
    // offers.
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "notes.bin",
        200_000,
    ));
    settle().await;
    let pc = w.pcs.latest();
    assert_eq!(
        pc.configuration.ice_transport_policy.as_deref(),
        Some("all")
    );
    assert_eq!(pc.configuration.ice_servers.len(), 2);
    assert_eq!(
        pc.configuration.ice_servers[1].username.as_deref(),
        Some("u")
    );
    assert_eq!(
        pc.configuration.ice_servers[1].credential.as_deref(),
        Some("c")
    );
    let ops = pc.ops();
    assert_eq!(ops[0], Op::CreateDataChannel("file".to_string()));
    assert_eq!(ops[1], Op::CreateOffer);
    assert_eq!(ops[2], Op::SetLocal("offer from pc 2".to_string()));
    let channel = pc.data_channel();
    assert_eq!(
        channel.threshold.load(std::sync::atomic::Ordering::SeqCst),
        BUFFER_LOW_BYTES
    );
    assert_eq!(
        signals_sent(&server.received()),
        vec![(bob, description("offer", "offer from pc 2"))]
    );
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.role, transfer.status, transfer.bytes),
        (TransferRole::Sender, TransferStatus::Connecting, 0)
    );
    assert_eq!(
        (transfer.offer, transfer.peer, transfer.size),
        (offer, bob, 200_000)
    );
    assert_eq!(transfer.route, None);

    // Bob's answer and candidates; the sender's own candidates go to Bob.
    server.send(&signal(offer, bob, description("answer", "bob's answer")));
    server.send(&signal(offer, bob, candidate("candidate:bob")));
    settle().await;
    assert_eq!(
        pc.remote_description(),
        Some(SessionDescription {
            kind: DescriptionKind::Answer,
            sdp: "bob's answer".to_string()
        })
    );
    assert!(
        pc.ops()
            .iter()
            .any(|op| matches!(op, Op::AddCandidate(c) if c.candidate == "candidate:bob"))
    );
    pc.gathered("candidate:me");
    settle().await;
    assert_eq!(
        signals_sent(&server.received()),
        vec![(bob, candidate("candidate:me"))]
    );

    // Connected, and the channel opens: every byte goes down it, in order, in chunks.
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    assert_eq!(channel.sent_bytes(), bytes);
    assert!(
        channel
            .sent
            .lock()
            .unwrap()
            .iter()
            .all(|chunk| chunk.len() as u64 <= CHUNK_BYTES)
    );
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.status, transfer.bytes),
        (TransferStatus::Moving, 200_000)
    );
    // The file was opened again for the transfer, read, and closed.
    assert_eq!(file.opened.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(
        w.files
            .latest_source()
            .closed
            .load(std::sync::atomic::Ordering::SeqCst)
    );

    // Only Bob's word that it all arrived ends the transfer, as completed by the peer.
    channel.deliver_text("nearly");
    settle().await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Moving);
    channel.deliver_text("done");
    settle().await;
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.status, transfer.ended_by),
        (TransferStatus::Completed, Some(EndedBy::Peer))
    );
    assert!(channel.is_closed());
    assert!(pc.is_closed());
    assert!(
        server.received().is_empty(),
        "the receiver's endTransfer tells the server"
    );
    // The offer still stands, so the file is kept for the next acceptance.
    assert!(w.files.released().is_empty());
    server.send(&withdrawn(offer, OfferEnd::Expired));
    settle().await;
    assert!(w.state().files.offers.is_empty());
    assert_eq!(w.files.released(), vec![REF.to_string()]);
}

/// `transfers.test.ts`: "sends an offered file to whoever accepts it", the receiver's side,
/// together with "writes to the file the receiver chose as it arrives, and keeps nothing in
/// memory".
#[tokio::test(start_paused = true)]
async fn an_accepted_file_is_written_where_the_receiver_chose_as_it_arrives() {
    let bytes = pattern(150_000);
    let (w, server, _, sink) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    let pc = receiving(
        &w,
        &server,
        offer,
        alice,
        "to-disk.bin",
        150_000,
        TransferMode::DirectPreferred,
    )
    .await;
    assert_eq!(
        pc.configuration.ice_transport_policy.as_deref(),
        Some("all")
    );
    assert!(
        pc.channel.lock().unwrap().is_none(),
        "the receiver opens no channel"
    );
    let ops = pc.ops();
    assert_eq!(ops[0], Op::SetRemote("their offer".to_string()));
    assert_eq!(ops[1], Op::CreateAnswer);
    assert!(matches!(&ops[2], Op::SetLocal(sdp) if sdp.starts_with("answer")));
    let sent = signals_sent(&server.received());
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, alice);
    assert_eq!(sent[0].1["kind"], "description");
    assert_eq!(sent[0].1["description"]["type"], "answer");
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.role, transfer.status),
        (TransferRole::Receiver, TransferStatus::Connecting)
    );
    assert!(!w.state().files.offers[0].own);

    pc.report(ConnectionState::Connected);
    let channel = channel_arrives(&pc);
    settle().await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Moving);
    deliver_all(&channel, &bytes);
    settle().await;
    assert_eq!(sink.written_bytes(), bytes);
    assert!(sink.is_finished());
    assert!(!sink.is_discarded());
    assert_eq!(*channel.texts.lock().unwrap(), vec!["done".to_string()]);
    assert!(server.received().contains(&ClientMessage::EndTransfer {
        offer,
        peer: alice,
        reason: TransferEnd::Completed,
    }));
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.status, transfer.ended_by, transfer.bytes),
        (
            TransferStatus::Completed,
            Some(EndedBy::ThisDevice),
            150_000
        )
    );
    assert!(channel.is_closed());
    assert!(pc.is_closed());
}

/// `transfers.test.ts`: "relays only when the receiver chose relayOnly", with the route read
/// from the connection's statistics and read again as ICE moves it.
#[tokio::test(start_paused = true)]
async fn relay_only_gathers_relay_candidates_alone_and_the_route_follows_the_selected_pair() {
    let (w, server, _, _) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    let pc = receiving(
        &w,
        &server,
        offer,
        alice,
        "a.txt",
        1,
        TransferMode::RelayOnly,
    )
    .await;
    assert_eq!(
        pc.configuration.ice_transport_policy.as_deref(),
        Some("relay")
    );
    assert_eq!(transfer_of(&w).mode, TransferMode::RelayOnly);
    pc.set_stats(
        r#"[
            {"id":"T","type":"transport","selectedCandidatePairId":"P"},
            {"id":"P","type":"candidate-pair","localCandidateId":"L","remoteCandidateId":"R"},
            {"id":"L","type":"local-candidate","candidateType":"relay"},
            {"id":"R","type":"remote-candidate","candidateType":"relay"}
        ]"#,
    );
    pc.report(ConnectionState::Connected);
    settle().await;
    assert_eq!(transfer_of(&w).route, Some(TransferRoute::Relayed));
    // ICE moves the pair: the next reading, an interval on, shows it.
    pc.set_stats(
        r#"[
            {"id":"T","type":"transport","selectedCandidatePairId":"P"},
            {"id":"P","type":"candidate-pair","localCandidateId":"L","remoteCandidateId":"R"},
            {"id":"L","type":"local-candidate","candidateType":"host"},
            {"id":"R","type":"remote-candidate","candidateType":"srflx"}
        ]"#,
    );
    advance(ROUTE_INTERVAL - Duration::from_millis(1)).await;
    assert_eq!(transfer_of(&w).route, Some(TransferRoute::Relayed));
    advance(Duration::from_millis(1)).await;
    assert_eq!(transfer_of(&w).route, Some(TransferRoute::Direct));
    // A reading that finds no pair leaves the last route standing.
    pc.set_stats("[]");
    advance(ROUTE_INTERVAL).await;
    assert_eq!(transfer_of(&w).route, Some(TransferRoute::Direct));
}

/// `transfers.test.ts`: "closes both sides at once when either cancels", from this side.
#[tokio::test(start_paused = true)]
async fn cancelling_tells_the_other_side_and_closes_the_transfer_at_once() {
    let (w, server, _, _) = in_call(pattern(50_000_000)).await;
    let offer = Uuid::now_v7();
    let bob = Uuid::now_v7();
    offer_file(&w, &server, offer, "big.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "big.bin",
        50_000_000,
    ));
    settle().await;
    let pc = w.pcs.latest();
    let channel = pc.data_channel();
    // A stalled link: the channel's buffer is full, so the pump waits.
    channel
        .buffered
        .store(BUFFER_HIGH_BYTES + 1, std::sync::atomic::Ordering::SeqCst);
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    server.received();
    assert_eq!(transfer_of(&w).status, TransferStatus::Moving);
    w.send(Command::CancelTransfer { offer, peer: bob });
    settle().await;
    assert!(server.received().contains(&ClientMessage::EndTransfer {
        offer,
        peer: bob,
        reason: TransferEnd::Cancelled,
    }));
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.status, transfer.ended_by),
        (TransferStatus::Cancelled, Some(EndedBy::ThisDevice))
    );
    assert!(channel.is_closed());
    assert!(pc.is_closed());
    // The pump, woken, finds the transfer stopped and neither reads nor sends anything more.
    channel.drained(0);
    settle().await;
    assert!(channel.sent.lock().unwrap().is_empty());
    assert_eq!(w.files.latest_source().position(), 0);
    // A cancel of a transfer that is over does nothing.
    w.send(Command::CancelTransfer { offer, peer: bob });
    settle().await;
    assert!(server.received().is_empty());
}

/// `transfers.test.ts`: "closes both sides at once when either cancels", from the other side,
/// together with "discards what was written of a file whose transfer is cancelled".
#[tokio::test(start_paused = true)]
async fn the_other_sides_cancel_closes_the_transfer_and_discards_what_was_written() {
    let (w, server, _, sink) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    let pc = receiving(
        &w,
        &server,
        offer,
        alice,
        "big.bin",
        50_000_000,
        TransferMode::DirectPreferred,
    )
    .await;
    pc.report(ConnectionState::Connected);
    let channel = channel_arrives(&pc);
    channel.deliver(pattern(CHUNK_BYTES as usize));
    settle().await;
    assert_eq!(sink.written_bytes().len() as u64, CHUNK_BYTES);
    server.send(&ended(offer, alice, TransferEnd::Cancelled));
    settle().await;
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.status, transfer.ended_by),
        (TransferStatus::Cancelled, Some(EndedBy::Peer))
    );
    assert!(sink.is_discarded());
    assert!(!sink.is_finished());
    assert!(channel.is_closed());
    assert!(pc.is_closed());
    // What arrives late is dropped.
    channel.deliver(pattern(10));
    settle().await;
    assert_eq!(sink.written_bytes().len() as u64, CHUNK_BYTES);
}

/// `transfers.test.ts`: "fails a transfer that brings more than was offered".
#[tokio::test(start_paused = true)]
async fn a_transfer_bringing_more_than_was_offered_fails() {
    let (w, server, _, sink) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    let pc = receiving(
        &w,
        &server,
        offer,
        alice,
        "ten.txt",
        4,
        TransferMode::DirectPreferred,
    )
    .await;
    pc.report(ConnectionState::Connected);
    let channel = channel_arrives(&pc);
    // One byte more than offered is more.
    channel.deliver(b"01234".to_vec());
    settle().await;
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.status, transfer.ended_by),
        (TransferStatus::Failed, Some(EndedBy::ThisDevice))
    );
    assert!(
        transfer
            .failure
            .as_deref()
            .unwrap()
            .contains("more arrived")
    );
    assert!(server.received().contains(&ClientMessage::EndTransfer {
        offer,
        peer: alice,
        reason: TransferEnd::Failed,
    }));
    assert!(sink.is_discarded());
    assert!(!sink.is_finished());
}

/// `transfers.test.ts`: "keeps a transfer going after its offer is withdrawn".
#[tokio::test(start_paused = true)]
async fn a_transfer_goes_on_after_its_offer_is_withdrawn() {
    let bytes = pattern(100_000);
    let (w, server, _, _) = in_call(bytes.clone()).await;
    let offer = Uuid::now_v7();
    let bob = Uuid::now_v7();
    offer_file(&w, &server, offer, "keep.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "keep.bin",
        100_000,
    ));
    server.send(&withdrawn(offer, OfferEnd::Expired));
    settle().await;
    assert!(w.state().files.offers.is_empty());
    assert!(
        w.files.released().is_empty(),
        "the file is kept while it is being sent"
    );
    let pc = w.pcs.latest();
    let channel = pc.data_channel();
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    assert_eq!(channel.sent_bytes(), bytes);
    channel.deliver_text("done");
    settle().await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Completed);
    assert_eq!(w.files.released(), vec![REF.to_string()]);
}

// --- The engine's own ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn the_offers_validity_is_clamped_to_what_the_server_allows() {
    let (w, server, _, _) = in_call(pattern(1)).await;
    let offer = Uuid::now_v7();
    let frames = offer_file(&w, &server, offer, "short.txt", 5).await;
    assert!(frames.iter().any(|f| matches!(
        f,
        ClientMessage::OfferFile {
            valid_for_seconds: 10,
            ..
        }
    )));
    let other = Uuid::now_v7();
    let frames = offer_file(&w, &server, other, "long.txt", 7200).await;
    assert!(frames.iter().any(|f| matches!(
        f,
        ClientMessage::OfferFile {
            valid_for_seconds: 3600,
            ..
        }
    )));
}

#[tokio::test(start_paused = true)]
async fn a_file_that_cannot_be_opened_is_the_offers_failure_and_is_let_go() {
    let (w, server, _, _) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    w.send(Command::OfferFile {
        offer,
        reference: "nowhere".to_string(),
        name: "gone.txt".to_string(),
        allow_direct: true,
        valid_for_seconds: 60,
    });
    settle().await;
    assert!(server.received().is_empty(), "nothing is offered");
    let failure = w.state().files.failure.expect("a failure");
    assert_eq!(
        (failure.command, failure.offer, failure.name.as_str()),
        (FileCommand::Offer, offer, "gone.txt")
    );
    assert!(failure.detail.contains("no file at nowhere"));
    assert_eq!(w.files.released(), vec!["nowhere".to_string()]);
    // The next file command clears it; so does asking.
    w.send(Command::ClearFileFailure);
    settle().await;
    assert_eq!(w.state().files.failure, None);
}

#[tokio::test(start_paused = true)]
async fn a_place_that_cannot_be_opened_is_the_acceptances_failure_and_nothing_is_accepted() {
    let (w, server, _, _) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    server.send(&offered(offer, alice, "theirs.bin", 10, true, 60_000));
    settle().await;
    w.send(Command::AcceptOffer {
        offer,
        mode: TransferMode::DirectPreferred,
        destination: "nowhere".to_string(),
    });
    settle().await;
    assert!(server.received().is_empty());
    let failure = w.state().files.failure.expect("a failure");
    assert_eq!(
        (failure.command, failure.offer, failure.name.as_str()),
        (FileCommand::Accept, offer, "theirs.bin")
    );
    assert!(failure.detail.contains("no place at nowhere"));
    // Accepting again, with a place, works and clears the failure.
    w.files.add_destination(DEST);
    w.send(Command::AcceptOffer {
        offer,
        mode: TransferMode::DirectPreferred,
        destination: DEST.to_string(),
    });
    settle().await;
    assert_eq!(w.state().files.failure, None);
    assert!(server.received().contains(&ClientMessage::AcceptFile {
        offer,
        mode: TransferMode::DirectPreferred
    }));
}

#[tokio::test(start_paused = true)]
async fn the_servers_refusal_of_an_offer_or_acceptance_is_the_files_failure() {
    let (w, server, _, sink) = in_call(pattern(3)).await;
    let offer = Uuid::now_v7();
    w.send(Command::OfferFile {
        offer,
        reference: REF.to_string(),
        name: "mine.txt".to_string(),
        allow_direct: false,
        valid_for_seconds: 60,
    });
    settle().await;
    server.received();
    server.send(&ServerMessage::Error {
        detail: "this server does not relay transfers, so an offer must allow direct ones"
            .to_string(),
        fatal: false,
        retry_after_seconds: None,
        refused: Some("offerFile".to_string()),
    });
    settle().await;
    let failure = w.state().files.failure.expect("a failure");
    assert_eq!(
        (failure.command, failure.offer, failure.name.as_str()),
        (FileCommand::Offer, offer, "mine.txt")
    );
    assert!(failure.detail.contains("must allow direct"));
    assert_eq!(w.files.released(), vec![REF.to_string()]);

    let theirs = Uuid::now_v7();
    let alice = Uuid::now_v7();
    server.send(&offered(theirs, alice, "theirs.bin", 10, true, 60_000));
    settle().await;
    w.send(Command::AcceptOffer {
        offer: theirs,
        mode: TransferMode::RelayOnly,
        destination: DEST.to_string(),
    });
    settle().await;
    server.received();
    server.send(&ServerMessage::Error {
        detail: "this server does not relay transfers".to_string(),
        fatal: false,
        retry_after_seconds: None,
        refused: Some("acceptFile".to_string()),
    });
    settle().await;
    let failure = w.state().files.failure.expect("a failure");
    assert_eq!(
        (failure.command, failure.offer, failure.name.as_str()),
        (FileCommand::Accept, theirs, "theirs.bin")
    );
    assert!(sink.is_discarded(), "the place opened for it is let go");
}

#[tokio::test(start_paused = true)]
async fn an_offer_runs_out_on_this_devices_clock_and_a_later_one_under_its_id_is_left_alone() {
    let (w, server, _, sink) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    server.send(&offered(offer, alice, "soon.txt", 10, true, 1_000));
    settle().await;
    w.send(Command::AcceptOffer {
        offer,
        mode: TransferMode::DirectPreferred,
        destination: DEST.to_string(),
    });
    settle().await;
    server.received();
    advance(Duration::from_millis(500)).await;
    // Offered again under the same id, for longer: the first telling's expiry is let go.
    server.send(&offered(offer, alice, "soon.txt", 10, true, 5_000));
    settle().await;
    advance(Duration::from_millis(600)).await;
    assert_eq!(w.state().files.offers.len(), 1, "the second telling stands");
    assert!(!sink.is_discarded(), "the place waits for the transfer");
    advance(Duration::from_millis(4_400)).await;
    assert!(
        w.state().files.offers.is_empty(),
        "the second telling ran out"
    );
    assert!(
        sink.is_discarded(),
        "a place for an offer that ran out is let go"
    );
}

#[tokio::test(start_paused = true)]
async fn an_offer_withdrawn_before_its_transfer_starts_lets_the_place_go() {
    let (w, server, _, sink) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    server.send(&offered(offer, alice, "theirs.bin", 10, true, 60_000));
    settle().await;
    w.send(Command::AcceptOffer {
        offer,
        mode: TransferMode::DirectPreferred,
        destination: DEST.to_string(),
    });
    settle().await;
    server.send(&withdrawn(offer, OfferEnd::Withdrawn));
    settle().await;
    assert!(sink.is_discarded());
    assert!(w.state().files.offers.is_empty());
    // A transfer the server starts for it anyway finds no place to write and fails it.
    server.send(&starting(
        offer,
        alice,
        TransferRole::Receiver,
        TransferMode::DirectPreferred,
        "theirs.bin",
        10,
    ));
    settle().await;
    assert!(server.received().contains(&ClientMessage::EndTransfer {
        offer,
        peer: alice,
        reason: TransferEnd::Failed,
    }));
    assert!(w.state().files.transfers.is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_offer_gone_while_its_place_is_opened_is_not_accepted() {
    let (w, server, _, sink) = in_call(Vec::new()).await;
    w.files
        .slow_sinks
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    server.send(&offered(offer, alice, "theirs.bin", 10, true, 60_000));
    settle().await;
    w.send(Command::AcceptOffer {
        offer,
        mode: TransferMode::DirectPreferred,
        destination: DEST.to_string(),
    });
    settle().await;
    server.send(&withdrawn(offer, OfferEnd::Withdrawn));
    advance(Duration::from_secs(1)).await;
    assert!(server.received().is_empty(), "nothing is accepted");
    let failure = w.state().files.failure.expect("a failure");
    assert_eq!(
        (failure.command, failure.name.as_str()),
        (FileCommand::Accept, "theirs.bin")
    );
    assert!(failure.detail.contains("no longer stands"));
    assert!(sink.is_discarded());
}

#[tokio::test(start_paused = true)]
async fn a_file_that_changed_since_it_was_offered_fails_its_transfer() {
    // Shorter than offered: it held 150 bytes when offered and 100 when sent.
    let (w, server, file, _) = in_call(pattern(100)).await;
    let offer = Uuid::now_v7();
    let bob = Uuid::now_v7();
    *file.claimed_size.lock().unwrap() = Some(150);
    offer_file(&w, &server, offer, "shrinking.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "shrinking.bin",
        150,
    ));
    settle().await;
    let pc = w.pcs.latest();
    let channel = pc.data_channel();
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    let transfer = transfer_of(&w);
    assert_eq!(transfer.status, TransferStatus::Failed);
    assert!(transfer.failure.as_deref().unwrap().contains("shorter"));
    assert!(server.received().contains(&ClientMessage::EndTransfer {
        offer,
        peer: bob,
        reason: TransferEnd::Failed,
    }));

    // Longer than offered: the whole offered size is sent, then the file goes on.
    let (w, server, file, _) = in_call(pattern(100)).await;
    let offer = Uuid::now_v7();
    *file.claimed_size.lock().unwrap() = Some(60);
    offer_file(&w, &server, offer, "growing.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "growing.bin",
        60,
    ));
    settle().await;
    let pc = w.pcs.latest();
    let channel = pc.data_channel();
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    assert_eq!(channel.sent_bytes().len(), 60);
    let transfer = transfer_of(&w);
    assert_eq!(transfer.status, TransferStatus::Failed);
    assert!(transfer.failure.as_deref().unwrap().contains("longer"));

    // One that cannot be read.
    let (w, server, file, _) = in_call(pattern(100)).await;
    let offer = Uuid::now_v7();
    offer_file(&w, &server, offer, "unreadable.bin", 60).await;
    file.fail_read
        .store(true, std::sync::atomic::Ordering::SeqCst);
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "unreadable.bin",
        100,
    ));
    settle().await;
    let pc = w.pcs.latest();
    pc.report(ConnectionState::Connected);
    pc.data_channel().open();
    settle().await;
    let transfer = transfer_of(&w);
    assert_eq!(transfer.status, TransferStatus::Failed);
    assert!(
        transfer
            .failure
            .as_deref()
            .unwrap()
            .contains("the disk refused")
    );
}

#[tokio::test(start_paused = true)]
async fn the_pump_pauses_above_the_high_mark_and_resumes_at_the_low_one() {
    let bytes = pattern(3 * CHUNK_BYTES as usize);
    let (w, server, _, _) = in_call(bytes.clone()).await;
    let offer = Uuid::now_v7();
    let bob = Uuid::now_v7();
    offer_file(&w, &server, offer, "paced.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "paced.bin",
        bytes.len() as u64,
    ));
    settle().await;
    let pc = w.pcs.latest();
    let channel = pc.data_channel();
    channel
        .buffered
        .store(BUFFER_HIGH_BYTES + 1, std::sync::atomic::Ordering::SeqCst);
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    assert!(
        channel.sent.lock().unwrap().is_empty(),
        "nothing is sent over a full buffer"
    );
    // At the mark itself the pump goes on.
    channel.drained(BUFFER_HIGH_BYTES);
    settle().await;
    assert_eq!(channel.sent_bytes(), bytes);
    assert_eq!(transfer_of(&w).bytes, bytes.len() as u64);
}

#[tokio::test(start_paused = true)]
async fn progress_moves_at_most_every_interval_and_with_every_other_change() {
    let bytes = pattern(3 * CHUNK_BYTES as usize);
    let (w, server, _, sink) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    let pc = receiving(
        &w,
        &server,
        offer,
        alice,
        "slow.bin",
        bytes.len() as u64,
        TransferMode::DirectPreferred,
    )
    .await;
    pc.report(ConnectionState::Connected);
    let channel = channel_arrives(&pc);
    settle().await;
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK_BYTES as usize).collect();
    channel.deliver(chunks[0].to_vec());
    settle().await;
    assert_eq!(
        transfer_of(&w).bytes,
        0,
        "within the interval of the start, nothing moves"
    );
    advance(PROGRESS_INTERVAL).await;
    channel.deliver(chunks[1].to_vec());
    settle().await;
    assert_eq!(transfer_of(&w).bytes, 2 * CHUNK_BYTES);
    channel.deliver(chunks[2].to_vec());
    settle().await;
    // The end is a change of its own, so the last bytes show with it.
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.status, transfer.bytes),
        (TransferStatus::Completed, bytes.len() as u64)
    );
    assert_eq!(sink.written_bytes(), bytes);
}

#[tokio::test(start_paused = true)]
async fn a_channel_closing_early_fails_the_transfer_and_one_closing_after_the_last_byte_waits_for_the_verdict()
 {
    // Early: nothing sent yet when the channel closes.
    let bytes = pattern(10);
    let (w, server, _, _) = in_call(bytes.clone()).await;
    let offer = Uuid::now_v7();
    let bob = Uuid::now_v7();
    offer_file(&w, &server, offer, "a.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "a.bin",
        10,
    ));
    settle().await;
    let pc = w.pcs.latest();
    let channel = pc.data_channel();
    channel
        .buffered
        .store(BUFFER_HIGH_BYTES + 1, std::sync::atomic::Ordering::SeqCst);
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    server.received();
    channel.closed_by_peer();
    settle().await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Failed);
    assert!(server.received().contains(&ClientMessage::EndTransfer {
        offer,
        peer: bob,
        reason: TransferEnd::Failed,
    }));

    // After the last byte: the verdict may still be on its way; without it, failed.
    let (w, server, _, _) = in_call(bytes.clone()).await;
    let offer = Uuid::now_v7();
    offer_file(&w, &server, offer, "b.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "b.bin",
        10,
    ));
    settle().await;
    let pc = w.pcs.latest();
    let channel = pc.data_channel();
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    assert_eq!(channel.sent_bytes(), bytes);
    channel.closed_by_peer();
    settle().await;
    advance(VERDICT_GRACE - Duration::from_millis(1)).await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Moving);
    advance(Duration::from_millis(1)).await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Failed);

    // With it: completed, and the grace's end changes nothing.
    let (w, server, _, _) = in_call(bytes.clone()).await;
    let offer = Uuid::now_v7();
    offer_file(&w, &server, offer, "c.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "c.bin",
        10,
    ));
    settle().await;
    let pc = w.pcs.latest();
    let channel = pc.data_channel();
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    channel.closed_by_peer();
    settle().await;
    advance(Duration::from_secs(1)).await;
    channel.deliver_text("done");
    settle().await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Completed);
    advance(VERDICT_GRACE).await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Completed);

    // A receiver's channel closing before the last byte fails it.
    let (w, server, _, sink) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    let pc = receiving(
        &w,
        &server,
        offer,
        alice,
        "d.bin",
        10,
        TransferMode::DirectPreferred,
    )
    .await;
    pc.report(ConnectionState::Connected);
    let channel = channel_arrives(&pc);
    channel.deliver(pattern(4));
    channel.closed_by_peer();
    settle().await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Failed);
    assert!(sink.is_discarded());
}

#[tokio::test(start_paused = true)]
async fn a_connection_that_fails_or_cannot_be_set_up_fails_the_transfer() {
    let (w, server, _, _) = in_call(pattern(10)).await;
    let offer = Uuid::now_v7();
    let bob = Uuid::now_v7();
    offer_file(&w, &server, offer, "a.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "a.bin",
        10,
    ));
    settle().await;
    let pc = w.pcs.latest();
    pc.report(ConnectionState::Failed);
    settle().await;
    let transfer = transfer_of(&w);
    assert_eq!(transfer.status, TransferStatus::Failed);
    assert!(transfer.failure.as_deref().unwrap().contains("failed"));

    // An answer the platform refuses.
    let (w, server, _, _) = in_call(pattern(10)).await;
    let offer = Uuid::now_v7();
    offer_file(&w, &server, offer, "b.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "b.bin",
        10,
    ));
    settle().await;
    let pc = w.pcs.latest();
    pc.fail_next_remote();
    server.send(&signal(offer, bob, description("answer", "bad")));
    settle().await;
    let transfer = transfer_of(&w);
    assert_eq!(transfer.status, TransferStatus::Failed);
    assert!(transfer.failure.as_deref().unwrap().contains("bad sdp"));

    // A signal this device does not read.
    let (w, server, _, _) = in_call(pattern(10)).await;
    let offer = Uuid::now_v7();
    offer_file(&w, &server, offer, "c.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "c.bin",
        10,
    ));
    settle().await;
    server.send(&signal(offer, bob, json!({ "kind": "telepathy" })));
    settle().await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Failed);

    // A data channel the platform cannot open.
    let (w, server, _, _) = in_call(pattern(10)).await;
    let offer = Uuid::now_v7();
    offer_file(&w, &server, offer, "d.bin", 60).await;
    w.pcs
        .fail_create
        .store(true, std::sync::atomic::Ordering::SeqCst);
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "d.bin",
        10,
    ));
    settle().await;
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.status, transfer.ended_by),
        (TransferStatus::Failed, Some(EndedBy::ThisDevice))
    );
    assert!(transfer.failure.as_deref().unwrap().contains("no WebRTC"));
    assert!(server.received().contains(&ClientMessage::EndTransfer {
        offer,
        peer: bob,
        reason: TransferEnd::Failed,
    }));
}

#[tokio::test(start_paused = true)]
async fn candidates_wait_for_the_description_and_a_browsers_end_mark_is_never_handed_on() {
    let (w, server, _, _) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    server.send(&offered(offer, alice, "x.bin", 10, true, 60_000));
    settle().await;
    w.send(Command::AcceptOffer {
        offer,
        mode: TransferMode::DirectPreferred,
        destination: DEST.to_string(),
    });
    settle().await;
    server.send(&starting(
        offer,
        alice,
        TransferRole::Receiver,
        TransferMode::DirectPreferred,
        "x.bin",
        10,
    ));
    settle().await;
    let pc = w.pcs.latest();
    server.send(&signal(offer, alice, candidate("candidate:early")));
    server.send(&signal(offer, alice, candidate("")));
    settle().await;
    assert!(pc.ops().is_empty(), "candidates wait for the description");
    server.send(&signal(offer, alice, description("offer", "their offer")));
    server.send(&signal(offer, alice, candidate("candidate:late")));
    server.send(&signal(offer, alice, candidate("")));
    settle().await;
    let ops = pc.ops();
    assert_eq!(ops[0], Op::SetRemote("their offer".to_string()));
    assert!(matches!(&ops[1], Op::AddCandidate(c) if c.candidate == "candidate:early"));
    assert_eq!(ops[2], Op::CreateAnswer);
    assert!(matches!(&ops[4], Op::AddCandidate(c) if c.candidate == "candidate:late"));
    assert_eq!(ops.len(), 5, "the end marks are not handed on: {ops:?}");
    // Nor is this side's own end mark sent.
    server.received();
    pc.gathered("");
    settle().await;
    assert!(signals_sent(&server.received()).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_file_of_no_bytes_is_sent_and_received_whole() {
    let (w, server, _, _) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let bob = Uuid::now_v7();
    offer_file(&w, &server, offer, "empty", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "empty",
        0,
    ));
    settle().await;
    let pc = w.pcs.latest();
    let channel = pc.data_channel();
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    assert!(channel.sent.lock().unwrap().is_empty());
    assert_eq!(transfer_of(&w).status, TransferStatus::Moving);
    channel.deliver_text("done");
    settle().await;
    assert_eq!(transfer_of(&w).status, TransferStatus::Completed);

    let (w, server, _, sink) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let alice = Uuid::now_v7();
    let pc = receiving(
        &w,
        &server,
        offer,
        alice,
        "empty",
        0,
        TransferMode::DirectPreferred,
    )
    .await;
    pc.report(ConnectionState::Connected);
    let channel = channel_arrives(&pc);
    settle().await;
    assert!(sink.is_finished());
    assert_eq!(*channel.texts.lock().unwrap(), vec!["done".to_string()]);
    let transfer = transfer_of(&w);
    assert_eq!(
        (transfer.status, transfer.bytes),
        (TransferStatus::Completed, 0)
    );
}

#[tokio::test(start_paused = true)]
async fn a_transfer_for_a_file_this_device_no_longer_holds_is_failed_at_once() {
    let (w, server, _, _) = in_call(Vec::new()).await;
    let offer = Uuid::now_v7();
    let bob = Uuid::now_v7();
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "lost.bin",
        10,
    ));
    settle().await;
    assert_eq!(
        server.received(),
        vec![ClientMessage::EndTransfer {
            offer,
            peer: bob,
            reason: TransferEnd::Failed,
        }]
    );
    assert!(w.state().files.transfers.is_empty());
    assert_eq!(w.pcs.count(), 1, "no connection is made for it");
}

#[tokio::test(start_paused = true)]
async fn the_servers_endings_links_and_dismissals_follow_the_frames() {
    let (w, server, _, _) = in_call(pattern(10)).await;
    let offer = Uuid::now_v7();
    let (bob, carol) = (Uuid::now_v7(), Uuid::now_v7());
    offer_file(&w, &server, offer, "a.bin", 60).await;
    server.send(&starting(
        offer,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "a.bin",
        10,
    ));
    server.send(&starting(
        offer,
        carol,
        TransferRole::Sender,
        TransferMode::RelayOnly,
        "a.bin",
        10,
    ));
    server.send(&ServerMessage::TransferLinkChanged {
        link: TransferLink {
            sender: w.user,
            receiver: bob,
        },
        active: true,
    });
    server.send(&ServerMessage::TransferLinkChanged {
        link: TransferLink {
            sender: w.user,
            receiver: carol,
        },
        active: true,
    });
    settle().await;
    let files = w.state().files;
    assert_eq!(files.transfers.len(), 2);
    assert_eq!(files.links.len(), 2);
    server.send(&ended(offer, bob, TransferEnd::Left));
    server.send(&ended(offer, carol, TransferEnd::NotPermitted));
    server.send(&ServerMessage::TransferLinkChanged {
        link: TransferLink {
            sender: w.user,
            receiver: bob,
        },
        active: false,
    });
    settle().await;
    let files = w.state().files;
    assert_eq!(
        files.links,
        vec![TransferLink {
            sender: w.user,
            receiver: carol
        }]
    );
    assert_eq!(
        files
            .transfers
            .iter()
            .map(|t| (t.peer, t.status, t.ended_by))
            .collect::<Vec<_>>(),
        vec![
            (bob, TransferStatus::Left, Some(EndedBy::Peer)),
            (carol, TransferStatus::NotPermitted, Some(EndedBy::Peer)),
        ]
    );
    w.send(Command::DismissTransfer { offer, peer: bob });
    settle().await;
    let files = w.state().files;
    assert_eq!(files.transfers.len(), 1);
    assert_eq!(files.transfers[0].peer, carol);
    // Live transfers are listed before the finished.
    let dave = Uuid::now_v7();
    server.send(&starting(
        offer,
        dave,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "a.bin",
        10,
    ));
    settle().await;
    let files = w.state().files;
    assert_eq!(
        files.transfers.iter().map(|t| t.peer).collect::<Vec<_>>(),
        vec![dave, carol]
    );
}

#[tokio::test(start_paused = true)]
async fn leaving_closes_every_transfer_lets_every_file_go_and_a_grant_lost_shows() {
    let bytes = pattern(10);
    let (w, server, _, sink) = in_call(bytes.clone()).await;
    let mine = Uuid::now_v7();
    let bob = Uuid::now_v7();
    offer_file(&w, &server, mine, "mine.bin", 60).await;
    server.send(&starting(
        mine,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "mine.bin",
        10,
    ));
    settle().await;
    let sending = w.pcs.latest();
    let theirs = Uuid::now_v7();
    let alice = Uuid::now_v7();
    let pc = receiving(
        &w,
        &server,
        theirs,
        alice,
        "theirs.bin",
        10,
        TransferMode::DirectPreferred,
    )
    .await;
    server.send(&ServerMessage::GrantsChanged {
        grants: Grants {
            speak: true,
            share_screen: true,
            camera: true,
            transfer_files: false,
        },
    });
    settle().await;
    assert!(!w.state().grants.transfer_files);
    assert_eq!(w.state().files.transfers.len(), 2);
    w.send(Command::Leave);
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Idle);
    assert_eq!(state.files, Default::default());
    assert!(sending.is_closed());
    assert!(pc.is_closed());
    assert!(sink.is_discarded());
    assert_eq!(w.files.released(), vec![REF.to_string()]);
}

#[tokio::test(start_paused = true)]
async fn a_ready_carries_the_calls_offers_links_and_policy_and_a_resume_keeps_the_files_still_offered()
 {
    let bytes = pattern(10);
    let w = World::new(0.5);
    let offer = Offer::to(w.channel, &[A]);
    let server = w.sockets.server(A);
    w.join(&offer).await;
    let alice = Uuid::now_v7();
    let theirs = Uuid::now_v7();
    server.received();
    server.send(&ServerMessage::Ready {
        session: w.session,
        user: w.user,
        participants: Vec::new(),
        audio_slots: 16,
        max_watched_video: 25,
        resume: "secret-1".to_string(),
        offers: vec![FileOffer {
            id: theirs,
            from: alice,
            name: "theirs.bin".to_string(),
            size: 7,
            allow_direct: false,
            expires_in_ms: 30_000,
        }],
        links: vec![TransferLink {
            sender: alice,
            receiver: Uuid::now_v7(),
        }],
        transfers: TransferPolicy {
            relay_mbps: Some(50),
        },
    });
    settle().await;
    server.offer(1, vec![slot("0", 0)]);
    settle().await;
    w.pcs.latest().report(ConnectionState::Connected);
    settle().await;
    let files = w.state().files;
    assert_eq!(files.relay_mbps, Some(50));
    assert_eq!(files.links.len(), 1);
    assert_eq!(
        (
            files.offers[0].id,
            files.offers[0].own,
            files.offers[0].allow_direct
        ),
        (theirs, false, false)
    );

    // This device offers a file, then its socket drops and is resumed: the offer stands at the
    // server, so the file is kept for it; another offer the resumed place no longer lists is let
    // go, and a transfer under way is closed.
    w.files.add(REF, bytes.clone());
    w.files.add("picked-2", bytes.clone());
    let mine = Uuid::now_v7();
    offer_file(&w, &server, mine, "mine.bin", 60).await;
    let gone = Uuid::now_v7();
    w.send(Command::OfferFile {
        offer: gone,
        reference: "picked-2".to_string(),
        name: "gone.bin".to_string(),
        allow_direct: true,
        valid_for_seconds: 60,
    });
    settle().await;
    server.send(&offered(gone, w.user, "gone.bin", 10, true, 60_000));
    let bob = Uuid::now_v7();
    server.send(&starting(
        mine,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "mine.bin",
        10,
    ));
    settle().await;
    let transfer_pc = w.pcs.latest();
    server.received();
    server.close();
    advance(Duration::from_millis(500)).await;
    // The call stays connected while the socket is taken back within the grace; the resume's
    // `ready` restates the files.
    assert_eq!(w.state().status, CallStatus::Connected);
    assert!(
        server
            .received()
            .iter()
            .any(|frame| matches!(frame, ClientMessage::Resume { .. })),
        "the resume is presented on the new socket"
    );
    server.send(&ServerMessage::Ready {
        session: w.session,
        user: w.user,
        participants: Vec::new(),
        audio_slots: 16,
        max_watched_video: 25,
        resume: "secret-2".to_string(),
        offers: vec![FileOffer {
            id: mine,
            from: w.user,
            name: "mine.bin".to_string(),
            size: 10,
            allow_direct: true,
            expires_in_ms: 50_000,
        }],
        links: Vec::new(),
        transfers: TransferPolicy { relay_mbps: None },
    });
    settle().await;
    let state = w.state();
    assert_eq!(state.status, CallStatus::Connected);
    assert!(transfer_pc.is_closed(), "a transfer under way is closed");
    assert!(state.files.transfers.is_empty());
    assert_eq!(state.files.offers.len(), 1);
    assert_eq!(state.files.relay_mbps, None);
    assert_eq!(w.files.released(), vec!["picked-2".to_string()]);
    // An acceptance of the standing offer is still served.
    server.send(&starting(
        mine,
        bob,
        TransferRole::Sender,
        TransferMode::DirectPreferred,
        "mine.bin",
        10,
    ));
    settle().await;
    let pc = w.pcs.latest();
    let channel = pc.data_channel();
    pc.report(ConnectionState::Connected);
    channel.open();
    settle().await;
    assert_eq!(channel.sent_bytes(), bytes);
}

#[tokio::test(start_paused = true)]
async fn file_commands_outside_a_call_are_the_files_failure() {
    let w = World::new(0.5);
    let offer = Uuid::now_v7();
    w.files.add(REF, pattern(3));
    w.send(Command::OfferFile {
        offer,
        reference: REF.to_string(),
        name: "mine.txt".to_string(),
        allow_direct: true,
        valid_for_seconds: 60,
    });
    settle().await;
    let failure = w.state().files.failure.expect("a failure");
    assert_eq!(
        (failure.command, failure.detail.as_str()),
        (FileCommand::Offer, "not in a call")
    );
    assert_eq!(w.files.released(), vec![REF.to_string()]);
    w.send(Command::AcceptOffer {
        offer,
        mode: TransferMode::DirectPreferred,
        destination: DEST.to_string(),
    });
    settle().await;
    let failure = w.state().files.failure.expect("a failure");
    assert_eq!(
        (failure.command, failure.detail.as_str()),
        (FileCommand::Accept, "not in a call")
    );
}
