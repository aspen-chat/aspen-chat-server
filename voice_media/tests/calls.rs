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
