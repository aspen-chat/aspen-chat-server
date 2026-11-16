//! What a slot changing occupant by offer costs a large call of forty people and sixteen slots, in
//! two patterns of speaking: twenty at once, four of whom fall quiet each second for four who were
//! (more voices than slots, changing faster than a conversation does), and three at once, one
//! handing over to someone else every two seconds. Prints, per listener over a minute, the offers
//! answered, their SDP's bytes, and the slot changes told.
//!
//! cargo test --release -p voice_media --test slot_churn -- --ignored --nocapture

mod common;

use common::{Peer, media, send, sending};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;
use voice_media::{CallId, Command, Event};
use voice_protocol::signal::MediaSource;

const PEOPLE: u64 = 40;
const SLOTS: u8 = 16;
const MEASURED: Duration = Duration::from_secs(60);
const LOUD: i8 = -10;
const QUIET: i8 = -100;

#[derive(Default, Debug, Clone, Copy)]
struct Counted {
    offers: u64,
    sdp_bytes: u64,
    slot_changes: u64,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement, run by hand"]
async fn twenty_speaking_four_changing_each_second() {
    measure(20, 4, Duration::from_secs(1)).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement, run by hand"]
async fn three_speaking_one_changing_every_two_seconds() {
    measure(3, 1, Duration::from_secs(2)).await;
}

/// `speaking` loud at once, `swapped` of whom fall quiet for as many who were every `every`.
async fn measure(speaking: usize, swapped: usize, every: Duration) {
    let media = Arc::new(media(SLOTS));
    let call = CallId(Uuid::now_v7());
    let measuring = Arc::new(AtomicBool::new(false));
    let mut levels = Vec::new();
    let mut tasks = Vec::new();
    let (ready_tx, mut ready) = mpsc::unbounded_channel();
    for id in 1..=PEOPLE {
        let (level_tx, mut level_rx) = mpsc::unbounded_channel::<i8>();
        levels.push(level_tx);
        let media = Arc::clone(&media);
        let measuring = Arc::clone(&measuring);
        let ready_tx = ready_tx.clone();
        tasks.push(tokio::spawn(async move {
            let mut peer = Peer::join(&media, call, id);
            peer.until(&media, Duration::from_secs(30), |e| {
                matches!(e, Event::Connected { .. })
            })
            .await
            .expect("connected");
            send(
                &media,
                Command::Send {
                    participant: peer.id,
                    source: MediaSource::Microphone,
                    external: false,
                },
            );
            let microphone = sending(MediaSource::Microphone);
            peer.until(&media, Duration::from_secs(30), |e| {
                matches!(e, Event::Offer { sections, .. } if sections.iter().any(|s| s.content == microphone))
            })
            .await
            .expect("a section for the microphone");
            let mid = peer.mid_of(&microphone).unwrap();
            peer.send_audio(&mid, Some(QUIET));
            ready_tx.send(()).unwrap();
            let mut counted = Counted::default();
            // Never dropped mid-answer: `until` returns once its time is up or an event came, and
            // an offer it read is answered before it returns.
            loop {
                loop {
                    match level_rx.try_recv() {
                        Ok(level) => peer.send_audio(&mid, Some(level)),
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => return counted,
                    }
                }
                let event = peer
                    .until(&media, Duration::from_millis(50), |_| true)
                    .await;
                if !measuring.load(Ordering::Relaxed) {
                    continue;
                }
                match event {
                    Some(Event::Offer { sdp, .. }) => {
                        counted.offers += 1;
                        counted.sdp_bytes += sdp.len() as u64;
                    }
                    Some(Event::Slots { assignments, .. }) => {
                        counted.slot_changes += assignments.len() as u64;
                    }
                    _ => {}
                }
            }
        }));
    }
    for _ in 0..PEOPLE {
        ready.recv().await.unwrap();
    }

    // A fixed sequence, so every run swaps the same people.
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let mut loud: Vec<usize> = (0..speaking).collect();
    let mut quiet: Vec<usize> = (speaking..PEOPLE as usize).collect();
    for &who in &loud {
        levels[who].send(LOUD).unwrap();
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    measuring.store(true, Ordering::Relaxed);
    let started = tokio::time::Instant::now();
    while started.elapsed() < MEASURED {
        for _ in 0..swapped {
            let falls = loud.swap_remove(next(loud.len()));
            let rises = quiet.swap_remove(next(quiet.len()));
            levels[falls].send(QUIET).unwrap();
            levels[rises].send(LOUD).unwrap();
            loud.push(rises);
            quiet.push(falls);
        }
        tokio::time::sleep(every).await;
    }
    measuring.store(false, Ordering::Relaxed);
    drop(levels);
    let mut all = Vec::new();
    for task in tasks {
        all.push(task.await.unwrap());
    }
    let minutes = MEASURED.as_secs_f64() / 60.0;
    let per = |f: fn(&Counted) -> u64| {
        let mut values: Vec<f64> = all.iter().map(|c| f(c) as f64 / minutes).collect();
        values.sort_by(f64::total_cmp);
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        (mean, values[values.len() / 2], values[values.len() - 1])
    };
    for (name, f) in [
        (
            "offers a minute",
            (|c: &Counted| c.offers) as fn(&Counted) -> u64,
        ),
        ("SDP bytes a minute", |c: &Counted| c.sdp_bytes),
        ("slot changes a minute", |c: &Counted| c.slot_changes),
    ] {
        let (mean, median, max) = per(f);
        println!("{name} per listener: mean {mean:.0}, median {median:.0}, max {max:.0}");
    }
    let offers: u64 = all.iter().map(|c| c.offers).sum();
    let bytes: u64 = all.iter().map(|c| c.sdp_bytes).sum();
    println!(
        "mean offer {:.0} bytes; {:.1} offers a second across the call",
        bytes as f64 / offers.max(1) as f64,
        offers as f64 / MEASURED.as_secs_f64()
    );
}
