//! Typed frames over a voice server's signalling socket (`voice_protocol::signal`), and the
//! address of that socket. The server pings the socket itself and the platform answers, so no
//! keepalive is sent from here; a frame this build cannot read (a type a newer server added, or
//! a value added to one it knows) is skipped, since the protocol changes only by addition.

use crate::transport::{Received, Socket, TransportError};
use std::sync::Arc;
use voice_protocol::signal::{ClientMessage, ServerMessage};

/// What a signalling socket gave next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    Frame(ServerMessage),
    /// Text that is no frame this build knows; `kind` is its `type`, when it has one.
    Unknown {
        kind: Option<String>,
    },
    Closed,
}

/// One socket to a voice server, speaking frames.
pub struct Signalling {
    socket: Arc<dyn Socket>,
}

impl Signalling {
    pub fn new(socket: Arc<dyn Socket>) -> Self {
        Self { socket }
    }

    pub async fn send(&self, frame: &ClientMessage) -> Result<(), TransportError> {
        self.socket
            .send_text(serde_json::to_string(frame).expect("a frame serializes"))
            .await
    }

    pub async fn receive(&self) -> Incoming {
        match self.socket.receive().await {
            Received::Text(text) => read_frame(&text),
            Received::Closed => Incoming::Closed,
        }
    }

    pub fn close(&self) {
        self.socket.close();
    }
}

/// Reads one text frame. Text that is not a JSON object at all is `Unknown` with no kind, as
/// is an object without a string `type`.
pub fn read_frame(text: &str) -> Incoming {
    match serde_json::from_str::<ServerMessage>(text) {
        Ok(frame) => Incoming::Frame(frame),
        Err(_) => Incoming::Unknown {
            kind: serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text)
                .ok()
                .and_then(|object| object.get("type")?.as_str().map(str::to_owned)),
        },
    }
}

/// An address the engine refuses to connect to: it is not reached over HTTPS, and is not this
/// machine's own with plain text allowed there.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0} is not reached over HTTPS")]
pub struct InsecureAddress(pub String);

/// Whether `host` names this machine: `localhost`, a name under it, or a loopback address.
fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "[::1]"
        || (host.starts_with("127.")
            && host.split('.').count() == 4
            && host.split('.').all(|part| part.parse::<u8>().is_ok()))
}

/// The scheme, host (with its brackets for IPv6), and the rest of `url` after its authority.
fn parse(url: &str) -> Option<(&str, &str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if authority.starts_with('[') {
        let close = authority.find(']')?;
        &authority[..=close]
    } else {
        authority.split_once(':').map_or(authority, |(h, _)| h)
    };
    if host.is_empty() {
        return None;
    }
    Some((scheme, host, path))
}

/// The signalling socket of the voice server at `server_url`: its `/ws` over the WebSocket
/// scheme matching its own. Refused unless reached over HTTPS, or over plain HTTP at this
/// machine's own address while `loopback_plain_text` allows it, as a development build does.
pub fn signalling_url(
    server_url: &str,
    loopback_plain_text: bool,
) -> Result<String, InsecureAddress> {
    let insecure = || InsecureAddress(server_url.to_string());
    let (scheme, host, _) = parse(server_url).ok_or_else(insecure)?;
    let secure = match scheme {
        "https" | "wss" => true,
        "http" | "ws" => false,
        _ => return Err(insecure()),
    };
    if !secure && !(loopback_plain_text && is_loopback_host(host)) {
        return Err(insecure());
    }
    let rest = &server_url[scheme.len() + "://".len()..];
    let (authority_and_path, _) = rest.split_once(['?', '#']).unwrap_or((rest, ""));
    Ok(format!(
        "{}://{}/ws",
        if secure { "wss" } else { "ws" },
        authority_and_path.trim_end_matches('/')
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_protocol::signal::KickReason;

    #[test]
    fn frames_are_read_and_the_unknown_told_apart() {
        assert_eq!(
            read_frame(r#"{"type":"kicked","reason":"replaced"}"#),
            Incoming::Frame(ServerMessage::Kicked {
                reason: KickReason::Replaced
            })
        );
        assert_eq!(
            read_frame(r#"{"type":"handRaised","user":"x"}"#),
            Incoming::Unknown {
                kind: Some("handRaised".to_string())
            }
        );
        assert_eq!(
            read_frame(r#"{"type":"kicked","reason":"outOfCoffee"}"#),
            Incoming::Unknown {
                kind: Some("kicked".to_string())
            }
        );
        for text in ["", "kicked", r#"[{"type":"kicked"}]"#, r#"{"type":7}"#] {
            assert_eq!(read_frame(text), Incoming::Unknown { kind: None }, "{text}");
        }
    }

    #[test]
    fn signalling_urls_follow_the_scheme_and_refuse_plain_text_away_from_home() {
        assert_eq!(
            signalling_url("https://voice.example.org", false).as_deref(),
            Ok("wss://voice.example.org/ws")
        );
        assert_eq!(
            signalling_url("https://voice.example.org:8443/voice/", false).as_deref(),
            Ok("wss://voice.example.org:8443/voice/ws")
        );
        assert_eq!(
            signalling_url("http://localhost:8080", true).as_deref(),
            Ok("ws://localhost:8080/ws")
        );
        assert_eq!(
            signalling_url("http://alpha.localhost:8080/", true).as_deref(),
            Ok("ws://alpha.localhost:8080/ws")
        );
        assert_eq!(
            signalling_url("http://127.0.0.1:8080", true).as_deref(),
            Ok("ws://127.0.0.1:8080/ws")
        );
        assert_eq!(
            signalling_url("http://[::1]:8080", true).as_deref(),
            Ok("ws://[::1]:8080/ws")
        );
        for refused in [
            ("http://localhost:8080", false),
            ("http://voice.example.org", true),
            ("http://127.0.0.1.example.org", true),
            ("http://localhost.example.org", true),
            ("http://evil.localhost.example.org", true),
            ("http://127.0.0.256", true),
            ("ftp://voice.example.org", false),
            ("voice.example.org", false),
            ("https://", false),
        ] {
            assert_eq!(
                signalling_url(refused.0, refused.1),
                Err(InsecureAddress(refused.0.to_string())),
                "{refused:?}"
            );
        }
    }
}
