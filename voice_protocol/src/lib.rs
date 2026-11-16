//! The contract between the API server and the voice servers.
//!
//! A client never authenticates with a voice server directly. It asks the API server to join a
//! voice channel and receives a short-lived [`token`] naming the user, the channel, and the
//! candidate servers; the voice server verifies that token with the API servers' public key, or,
//! when it is given `token_secret`, also with that shared secret. Voice servers then keep the API
//! server informed over NATS with the [`control`] messages, and the API server turns those into
//! the events every client follows. The client and a voice server talk [`signal`] frames over a
//! WebSocket.
//!
//! The mobile apps' call engine (`voice_client`) needs [`signal`] alone, so [`control`] (NATS,
//! and the limits' bucket names) and [`token`] (Ed25519 and HMAC) are each behind a feature of
//! the same name, both on by default; a phone's build turns the defaults off and links neither.

#[cfg(feature = "control")]
pub mod control;
pub mod signal;
#[cfg(feature = "token")]
pub mod token;
