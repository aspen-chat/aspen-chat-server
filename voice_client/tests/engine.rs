//! The engine's facade: a thread of its own, commands queued from the test's thread, and
//! snapshots read back.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};
use support::*;
use uuid::Uuid;
use voice_client::call::{Options, Platform};
use voice_client::engine::Engine;
use voice_client::state::{CallStatus, ErrorKind};

#[test]
fn the_engine_runs_the_call_on_its_own_thread_and_numbers_its_snapshots() {
    let http = Arc::new(FakeHttp::default());
    http.answer_join(problem(403, "forbidden", "You may not join this call."));
    let sockets = Arc::new(FakeSockets::default());
    let pcs = Arc::new(FakePeerConnections::default());
    let snapshots = Arc::new(Snapshots::default());
    let engine = Engine::start(
        Platform {
            http: http.clone(),
            sockets,
            peer_connections: pcs,
            files: Arc::new(FakeFiles::default()),
        },
        Options {
            api_url: API_URL.to_string(),
            credentials: Arc::new(Token),
            loopback_plain_text: true,
            random: Box::new(Options::system_random),
        },
        snapshots.clone(),
    );
    assert_eq!(engine.state().status, CallStatus::Idle);
    let channel = Uuid::now_v7();
    // Nothing to leave or acknowledge: no snapshot comes of these.
    engine.leave();
    engine.acknowledge_end();
    engine.set_microphone(Some(World::microphone()));
    engine.join(channel);
    let started = Instant::now();
    while engine.state().status != CallStatus::Failed {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the call never failed: {:?}",
            engine.state()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let state = engine.state();
    assert_eq!(state.error_kind, Some(ErrorKind::Offer));
    assert_eq!(state.error.as_deref(), Some("You may not join this call."));
    assert_eq!(state.channel, Some(channel));
    let seen = snapshots.0.lock().unwrap().clone();
    assert!(
        seen.len() >= 2,
        "the microphone, then the join, then the failure"
    );
    assert!(
        seen.windows(2)
            .all(|pair| pair[1].revision == pair[0].revision + 1),
        "revisions count up by one"
    );
    assert!(
        seen.windows(2).all(|pair| {
            let mut before = pair[0].clone();
            before.revision = pair[1].revision;
            before != pair[1]
        }),
        "every snapshot differs from the one before"
    );
    assert_eq!(seen.last(), Some(&state));
    drop(engine);
    assert_eq!(http.offers_asked(), 1);
}

#[test]
fn the_system_random_is_spread_over_the_unit_interval() {
    let draws: Vec<f64> = (0..64).map(|_| Options::system_random()).collect();
    assert!(draws.iter().all(|d| (0.0..1.0).contains(d)));
    assert!(draws.iter().any(|d| *d != draws[0]));
}
