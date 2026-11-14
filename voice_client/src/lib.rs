//! A client's side of a call without a browser: the call engine the mobile apps run natively,
//! shared by both. It drives the platform's WebRTC through `peer_connection`, an interface the
//! iOS and Android layers implement over Google's WebRTC library; media itself never passes
//! through here, and the session descriptions the peer connection is given come from the voice
//! server.

pub mod peer_connection;
