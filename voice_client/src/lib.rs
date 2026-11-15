//! A client's side of a call without a browser: the call engine the mobile apps run natively,
//! shared by both. It drives the platform's WebRTC through `peer_connection`, an interface the
//! iOS and Android layers implement over Google's WebRTC library, and reaches the network
//! through `transport`, which they implement over URLSession and OkHttp, so TLS trust, proxies,
//! and network changes are the operating system's and no TLS stack runs here. Media itself
//! never passes through the engine: tracks are the platform's, named by opaque ids, and the
//! session descriptions the peer connection is given come from the voice server, the only
//! offerer (`voice_protocol::signal`).
//!
//! - `engine`: the facade the platform layers hold, whose methods are the call's commands and
//!   whose observer reads each snapshot of its `state`. It runs the call on a tokio
//!   current-thread runtime on a thread of its own.
//! - `call`: the state machine, everything the web client's `VoiceCall` decides: joining through
//!   the API server's offer and the candidates nearest first, failures reported, refusals and
//!   timeouts, answering the server's offers, mute and deafen, sources sent and videos watched,
//!   audio slots and per-person volume, resuming a dropped socket and rejoining a lost call,
//!   and each `kicked` reason.
//! - `signalling`: typed frames over a `Socket`, and the signalling address of a voice server.
//! - `api`: the REST calls a call makes.
//! - `bindings`: the engine as Swift and Kotlin reach it, through UniFFI: what crosses as it
//!   is, what is mirrored at the boundary, and the `Engine` object the apps hold.
//!
//! File transfers, rings, sounds, CallKit, and Telecom are not here: the signalling's file
//! frames are left alone for the transfers piece, and `state::CallState` is what the rest read.

uniffi::setup_scaffolding!();

pub mod api;
pub mod bindings;
pub mod call;
pub mod engine;
pub mod peer_connection;
pub mod signalling;
pub mod state;
pub mod transport;
