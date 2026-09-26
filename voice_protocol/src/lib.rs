//! The contract between the API server and the voice servers.
//!
//! A client never authenticates with a voice server directly. It asks the API server to join a
//! voice channel and receives a short-lived [`token`] naming the user, the channel, and the
//! candidate servers; the voice server verifies that token with the secret it shares with the
//! API server. Voice servers then keep the API server informed over NATS with the
//! [`control`] messages, and the API server turns those into the events every client follows.
//! The client and a voice server talk [`signal`] frames over a WebSocket.

pub mod control;
pub mod signal;
pub mod token;
