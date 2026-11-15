//! Calls through a real media thread on loopback, with str0m clients answering its offers.

mod common;

use common::{Peer, media, send, sending};
use std::time::Duration;
use uuid::Uuid;
use voice_media::{CallId, Command, Event};
use voice_protocol::signal::{MediaSource, SectionContent};

const WAIT: Duration = Duration::from_secs(10);

fn call() -> CallId {
    CallId(Uuid::now_v7())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_microphone_reaches_the_others_on_an_audio_slot() {
    let media = media(16);
    let call = call();
    let mut ann = Peer::join(&media, call, 1);
    let mut bob = Peer::join(&media, call, 2);
    // Everyone's connection opens with sixteen audio slots and nothing to send.
    ann.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .expect("Ann connects");
    bob.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .expect("Bob connects");
    assert_eq!(
        bob.sections
            .iter()
            .filter(|s| matches!(s.content, SectionContent::AudioSlot { .. }))
            .count(),
        16
    );

    send(
        &media,
        Command::Send {
            participant: ann.id,
            source: MediaSource::Microphone,
            external: false,
        },
    );
    ann.until(&media, WAIT, |e| {
        matches!(e, Event::Offer { revision: 2, .. })
    })
    .await
    .expect("Ann is offered a section to send her microphone on");
    let mid = ann
        .mid_of(&sending(MediaSource::Microphone))
        .expect("the microphone's section");
    ann.settle(&media).await;
    ann.send_audio(&mid, Some(-20));
    ann.until(&media, WAIT, |e| {
        matches!(
            e,
            Event::Live {
                source: MediaSource::Microphone,
                ..
            }
        )
    })
    .await
    .expect("Ann's microphone goes live");
    ann.until(&media, WAIT, |e| {
        matches!(e, Event::Speaking { speaking: true, .. })
    })
    .await
    .expect("Ann, loud, is speaking");

    // Bob is told which slot carries Ann, and hears her there, named as the contributing source.
    bob.until(&media, WAIT, |e| matches!(e, Event::Slots { .. }))
        .await
        .expect("Bob's slots are filled");
    let slot = bob
        .slots
        .iter()
        .find(|(_, user)| **user == Some(ann.user))
        .map(|(slot, _)| *slot)
        .expect("a slot carrying Ann");
    let slot_mid = bob
        .mid_of(&SectionContent::AudioSlot { slot })
        .expect("the slot's section");
    bob.settle(&media).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let ann_ssrc = ann.ssrc(&mid).await.expect("Ann's SSRC");
    let heard: Vec<_> = bob
        .drain()
        .into_iter()
        .filter(|p| p.mid == slot_mid)
        .collect();
    assert!(heard.len() > 10, "Bob heard {} packets", heard.len());
    assert!(heard.iter().all(|p| p.csrc == vec![ann_ssrc]));
    assert!(heard.iter().all(|p| p.audio_level == Some(-20)));
    // Numbered from one, without gaps.
    for pair in heard.windows(2) {
        assert_eq!(pair[1].seq, pair[0].seq + 1);
    }

    // Muted, she stops at once, speaking and sound.
    send(
        &media,
        Command::SetSourcePaused {
            participant: ann.id,
            source: MediaSource::Microphone,
            paused: true,
        },
    );
    ann.until(&media, WAIT, |e| {
        matches!(
            e,
            Event::Speaking {
                speaking: false,
                ..
            }
        )
    })
    .await
    .expect("Ann stops speaking when muted");
    tokio::time::sleep(Duration::from_millis(200)).await;
    bob.drain();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(bob.drain().iter().all(|p| p.mid != slot_mid));
}

#[tokio::test(flavor = "multi_thread")]
async fn video_reaches_a_watcher_and_starts_on_a_keyframe() {
    let media = media(16);
    let call = call();
    let mut ann = Peer::join(&media, call, 1);
    let mut bob = Peer::join(&media, call, 2);
    ann.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();
    bob.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();

    send(
        &media,
        Command::Send {
            participant: ann.id,
            source: MediaSource::Camera,
            external: false,
        },
    );
    ann.until(&media, WAIT, |e| {
        matches!(e, Event::Offer { revision: 2, .. })
    })
    .await
    .unwrap();
    let mid = ann.mid_of(&sending(MediaSource::Camera)).unwrap();
    ann.settle(&media).await;
    ann.send_video(&mid);

    // Nothing comes to Bob until he watches.
    bob.settle(&media).await;
    assert!(
        bob.mid_of(&SectionContent::Video {
            user: ann.user,
            source: MediaSource::Camera
        })
        .is_none()
    );
    send(
        &media,
        Command::Watch {
            participant: bob.id,
            video: vec![(ann.user, MediaSource::Camera)],
        },
    );
    bob.until(&media, WAIT, |e| {
        matches!(e, Event::Offer { revision: 2, .. })
    })
    .await
    .expect("Bob is offered Ann's camera");
    let video_mid = bob
        .mid_of(&SectionContent::Video {
            user: ann.user,
            source: MediaSource::Camera,
        })
        .unwrap();
    bob.settle(&media).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let seen: Vec<_> = bob
        .drain()
        .into_iter()
        .filter(|p| p.mid == video_mid)
        .collect();
    assert!(seen.len() > 20, "Bob saw {} packets", seen.len());
    // The first packet starts a keyframe: the sequence parameter set.
    assert_eq!(seen[0].first_byte & 0x1f, 7);

    // Unwatched, it stops.
    send(
        &media,
        Command::Watch {
            participant: bob.id,
            video: vec![],
        },
    );
    bob.until(&media, WAIT, |e| {
        matches!(e, Event::Offer { revision: 3, .. })
    })
    .await
    .unwrap();
    bob.settle(&media).await;
    bob.drain();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(bob.drain().iter().all(|p| p.mid != video_mid));
}

/// Starts a participant's microphone and returns the section it sends on.
async fn open_microphone(media: &voice_media::Media, peer: &mut Peer) -> String {
    send(
        media,
        Command::Send {
            participant: peer.id,
            source: MediaSource::Microphone,
            external: false,
        },
    );
    peer.until(media, WAIT, |e| {
        matches!(e, Event::Offer { revision: 2, .. })
    })
    .await
    .expect("an offer of a section to send the microphone on");
    let mid = peer.mid_of(&sending(MediaSource::Microphone)).unwrap();
    peer.settle(media).await;
    mid
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slot_moves_to_who_speaks_and_its_numbering_runs_on() {
    // One slot, two microphones: Bob's first, then Cat's once she speaks and he does not.
    let media = media(1);
    let call = call();
    let mut ann = Peer::join(&media, call, 1);
    let mut bob = Peer::join(&media, call, 2);
    let mut cat = Peer::join(&media, call, 3);
    for peer in [&mut ann, &mut bob, &mut cat] {
        peer.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
            .await
            .unwrap();
    }
    let bob_mid = open_microphone(&media, &mut bob).await;
    bob.send_audio(&bob_mid, Some(-100));
    ann.until(&media, WAIT, |_| false).await;
    assert_eq!(
        ann.slots.get(&0),
        Some(&Some(bob.user)),
        "Bob, first, holds the slot"
    );
    let cat_mid = open_microphone(&media, &mut cat).await;
    ann.settle(&media).await;
    assert_eq!(
        ann.slots.get(&0),
        Some(&Some(bob.user)),
        "Bob keeps it while neither speaks"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    cat.send_audio(&cat_mid, Some(-10));
    ann.until(&media, WAIT, |e| {
        matches!(e, Event::Slots { assignments, .. } if assignments.iter().any(|a| a.slot == 0))
    })
    .await
    .expect("the slot moves");
    assert_eq!(
        ann.slots.get(&0),
        Some(&Some(cat.user)),
        "the slot carries Cat now"
    );
    tokio::time::sleep(Duration::from_millis(400)).await;

    let slot_mid = ann.mid_of(&SectionContent::AudioSlot { slot: 0 }).unwrap();
    let heard: Vec<_> = ann
        .drain()
        .into_iter()
        .filter(|p| p.mid == slot_mid)
        .collect();
    let bob_ssrc = bob.ssrc(&bob_mid).await.unwrap();
    let cat_ssrc = cat.ssrc(&cat_mid).await.unwrap();
    let switch = heard
        .iter()
        .position(|p| p.csrc == vec![cat_ssrc])
        .expect("Cat heard on the slot");
    assert!(switch > 0 && heard[..switch].iter().all(|p| p.csrc == vec![bob_ssrc]));
    assert!(heard[switch..].iter().all(|p| p.csrc == vec![cat_ssrc]));
    // One stream to the client: its SSRC, sequence numbers, and timestamps run on across the
    // change of speaker.
    assert!(heard.iter().all(|p| p.ssrc == heard[0].ssrc));
    for pair in heard.windows(2) {
        assert_eq!(
            pair[1].seq,
            pair[0].seq + 1,
            "no gap or repeat in the numbering"
        );
    }
    let (before, after) = (&heard[switch - 1], &heard[switch]);
    let gap = after.timestamp.wrapping_sub(before.timestamp);
    let elapsed = after.at.duration_since(before.at).as_micros() as u32 * 48 / 1000;
    assert!(
        gap > 0 && gap < elapsed + 48_000 / 10,
        "the timestamp moves on by the time between"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ice_restart_is_offered_and_media_goes_on() {
    let media = media(16);
    let call = call();
    let mut ann = Peer::join(&media, call, 1);
    let mut bob = Peer::join(&media, call, 2);
    ann.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();
    bob.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();
    let mid = open_microphone(&media, &mut ann).await;
    ann.send_audio(&mid, Some(-30));
    bob.until(&media, WAIT, |e| matches!(e, Event::Slots { .. }))
        .await
        .unwrap();
    bob.settle(&media).await;

    send(
        &media,
        Command::RestartIce {
            participant: bob.id,
            connection: voice_protocol::signal::ConnectionKind::Call,
        },
    );
    let offer = bob
        .until(&media, WAIT, |e| matches!(e, Event::Offer { .. }))
        .await
        .expect("an ICE restart is offered");
    let ufrag = |sdp: &str| {
        sdp.lines()
            .find_map(|l| l.strip_prefix("a=ice-ufrag:"))
            .map(str::to_owned)
    };
    let first = bob
        .log
        .iter()
        .find_map(|e| match e {
            Event::Offer { sdp, .. } => ufrag(sdp),
            _ => None,
        })
        .unwrap();
    let Event::Offer { sdp, .. } = offer else {
        unreachable!()
    };
    assert_ne!(
        ufrag(&sdp).unwrap(),
        first,
        "the restart offers new ICE credentials"
    );
    bob.settle(&media).await;
    bob.drain();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        bob.drain().len() > 10,
        "Bob still hears Ann after the restart"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_capture_connection_sends_a_share_its_sender_previews() {
    let media = media(16);
    let call = call();
    let mut ann = Peer::join(&media, call, 1);
    let mut bob = Peer::join(&media, call, 2);
    ann.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();
    bob.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();

    send(
        &media,
        Command::Send {
            participant: ann.id,
            source: MediaSource::Screen,
            external: true,
        },
    );
    ann.until(&media, WAIT, |e| {
        matches!(
            e,
            Event::Offer {
                connection: voice_protocol::signal::ConnectionKind::Capture,
                ..
            }
        )
    })
    .await
    .expect("a capture connection is offered");
    let mid = ann.capture_mid_of(&sending(MediaSource::Screen)).unwrap();
    // The capture connection offers H.264 alone, which the helper encodes.
    assert!(ann.log.iter().any(|e| matches!(e,
        Event::Offer { connection: voice_protocol::signal::ConnectionKind::Capture, sdp, .. }
            if sdp.contains("H264/90000") && !sdp.contains("VP8/90000"))));
    ann.until(&media, WAIT, |e| {
        matches!(
            e,
            Event::Connected {
                connection: voice_protocol::signal::ConnectionKind::Capture,
                ..
            }
        )
    })
    .await
    .expect("the capture connection connects");
    ann.capture_video(&mid);

    let share = (ann.user, MediaSource::Screen);
    send(
        &media,
        Command::Watch {
            participant: bob.id,
            video: vec![share],
        },
    );
    send(
        &media,
        Command::Watch {
            participant: ann.id,
            video: vec![share],
        },
    );
    bob.until(&media, WAIT, |e| {
        matches!(e, Event::Offer { revision: 2, .. })
    })
    .await
    .unwrap();
    ann.until(&media, WAIT, |e| {
        matches!(
            e,
            Event::Offer {
                connection: voice_protocol::signal::ConnectionKind::Call,
                revision: 2,
                ..
            }
        )
    })
    .await
    .unwrap();
    // Watching a screen brings a section for its sound too.
    assert!(
        bob.mid_of(&SectionContent::ShareAudio { user: ann.user })
            .is_some()
    );
    bob.settle(&media).await;
    ann.settle(&media).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let content = SectionContent::Video {
        user: ann.user,
        source: MediaSource::Screen,
    };
    for peer in [&mut bob, &mut ann] {
        let video_mid = peer.mid_of(&content).unwrap();
        let seen: Vec<_> = peer
            .drain()
            .into_iter()
            .filter(|p| p.mid == video_mid)
            .collect();
        assert!(seen.len() > 20, "{} packets of the share", seen.len());
        assert_eq!(seen[0].first_byte & 0x1f, 7, "it starts on a keyframe");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn one_who_leaves_is_taken_off_everyone_elses_slots_and_video() {
    let media = media(16);
    let call = call();
    let mut ann = Peer::join(&media, call, 1);
    let mut bob = Peer::join(&media, call, 2);
    ann.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();
    bob.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();
    open_microphone(&media, &mut ann).await;
    send(
        &media,
        Command::Send {
            participant: ann.id,
            source: MediaSource::Camera,
            external: false,
        },
    );
    ann.settle(&media).await;
    send(
        &media,
        Command::Watch {
            participant: bob.id,
            video: vec![(ann.user, MediaSource::Camera)],
        },
    );
    bob.settle(&media).await;
    assert!(bob.slots.values().any(|u| *u == Some(ann.user)));
    let content = SectionContent::Video {
        user: ann.user,
        source: MediaSource::Camera,
    };
    assert!(bob.mid_of(&content).is_some());

    send(
        &media,
        Command::Leave {
            participant: ann.id,
        },
    );
    bob.until(&media, WAIT, |e| matches!(e, Event::Slots { .. }))
        .await
        .expect("Bob's slots change");
    assert!(bob.slots.values().all(|u| *u != Some(ann.user)));
    bob.settle(&media).await;
    assert!(
        bob.mid_of(&content).is_none(),
        "Ann's camera is no longer offered to Bob"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deafened_hears_nothing_until_undeafened() {
    let media = media(16);
    let call = call();
    let mut ann = Peer::join(&media, call, 1);
    let mut bob = Peer::join(&media, call, 2);
    ann.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();
    bob.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();
    let mid = open_microphone(&media, &mut ann).await;
    ann.send_audio(&mid, Some(-30));
    bob.until(&media, WAIT, |e| matches!(e, Event::Slots { .. }))
        .await
        .unwrap();
    bob.settle(&media).await;
    send(
        &media,
        Command::SetDeafened {
            participant: bob.id,
            deafened: true,
        },
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    bob.drain();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        bob.drain().is_empty(),
        "a deafened participant hears nothing"
    );
    send(
        &media,
        Command::SetDeafened {
            participant: bob.id,
            deafened: false,
        },
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(bob.drain().len() > 10, "undeafened, Bob hears Ann again");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_offer_left_unanswered_fails_its_connection() {
    let media = media(16);
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    send(
        &media,
        Command::Join {
            call: call(),
            participant: voice_media::ParticipantId(1),
            user: Uuid::now_v7(),
            events,
        },
    );
    assert!(matches!(received.recv().await, Some(Event::Offer { .. })));
    let failed = tokio::time::timeout(Duration::from_secs(12), received.recv())
        .await
        .expect("a failure within the answer timeout");
    assert!(matches!(
        failed,
        Some(Event::Failed {
            reason: voice_media::Failure::AnswerTimedOut,
            ..
        })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_without_udp_connects_over_tcp() {
    let media = media(16);
    let call = call();
    let mut ann = Peer::join_over_tcp(&media, call, 1);
    let mut bob = Peer::join(&media, call, 2);
    ann.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .expect("Ann connects over TCP");
    bob.until(&media, WAIT, |e| matches!(e, Event::Connected { .. }))
        .await
        .unwrap();
    let ann_mid = open_microphone(&media, &mut ann).await;
    let bob_mid = open_microphone(&media, &mut bob).await;
    ann.send_audio(&ann_mid, Some(-30));
    bob.send_audio(&bob_mid, Some(-30));
    ann.settle(&media).await;
    bob.settle(&media).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(bob.drain().len() > 10, "Bob hears Ann, who sends over TCP");
    assert!(ann.drain().len() > 10, "Ann hears Bob over TCP");
}
