//! The join token: what a client presents to a voice server to prove the API server let it
//! into a channel. It is the base64url-encoded claims, a dot, and a base64url-encoded
//! HMAC-SHA256 of those bytes under the secret both servers hold.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

/// What a join token asserts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinClaims {
    /// The user the token was issued to.
    pub user: Uuid,
    /// The voice channel they may join.
    pub channel: Uuid,
    /// The voice servers the token is good for. A server not listed refuses the token, so a
    /// token cannot be replayed against a server the API server did not offer.
    pub servers: Vec<Uuid>,
    /// Seconds since the Unix epoch after which the token is refused.
    pub expires_at: i64,
    /// Makes two tokens for the same user and channel distinct.
    pub nonce: Uuid,
    /// Whether they may send their microphone, the channel's Speak permission when the token
    /// was issued.
    pub speak: bool,
    /// Whether they may share a screen or game, picture and sound, the channel's Share screen
    /// permission when the token was issued.
    pub share_screen: bool,
    /// Whether they may offer files to the others in the call, the channel's Transfer files
    /// permission when the token was issued. A token without it grants nothing.
    #[serde(default)]
    pub transfer_files: bool,
    /// Whether they may send a camera, the channel's Use camera permission when the token was
    /// issued. A token without it grants nothing.
    #[serde(default)]
    pub camera: bool,
}

impl JoinClaims {
    /// Whether the token lets its holder produce media from `source`.
    pub fn may_produce(&self, source: crate::signal::MediaSource) -> bool {
        use crate::signal::MediaSource;
        match source {
            MediaSource::Microphone => self.speak,
            MediaSource::Screen | MediaSource::ScreenAudio => self.share_screen,
            MediaSource::Camera => self.camera,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("the token is not two base64url parts joined by a dot")]
    Malformed,
    #[error("the token's signature does not match")]
    BadSignature,
    #[error("the token expired")]
    Expired,
    #[error("the token is not for this server")]
    WrongServer,
}

type HmacSha256 = Hmac<Sha256>;

/// Signs `claims` under `secret`.
pub fn sign(claims: &JoinClaims, secret: &[u8]) -> String {
    let payload = serde_json::to_vec(claims).expect("claims serialize");
    let mut mac = HmacSha256::new_from_slice(secret).expect("any key length is valid for HMAC");
    mac.update(&payload);
    let signature = mac.finalize().into_bytes();
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(signature)
    )
}

/// Verifies `token` under `secret` for `server` at time `now` (seconds since the Unix epoch)
/// and returns its claims. The signature is checked before anything else is read.
pub fn verify(
    token: &str,
    secret: &[u8],
    server: Uuid,
    now: i64,
) -> Result<JoinClaims, TokenError> {
    let (payload, signature) = token.split_once('.').ok_or(TokenError::Malformed)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| TokenError::Malformed)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| TokenError::Malformed)?;
    let mut mac = HmacSha256::new_from_slice(secret).expect("any key length is valid for HMAC");
    mac.update(&payload);
    mac.verify_slice(&signature)
        .map_err(|_| TokenError::BadSignature)?;
    let claims: JoinClaims = serde_json::from_slice(&payload).map_err(|_| TokenError::Malformed)?;
    if claims.expires_at <= now {
        return Err(TokenError::Expired);
    }
    if !claims.servers.contains(&server) {
        return Err(TokenError::WrongServer);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims() -> JoinClaims {
        JoinClaims {
            user: Uuid::now_v7(),
            channel: Uuid::now_v7(),
            servers: vec![Uuid::now_v7(), Uuid::now_v7()],
            expires_at: 1_000,
            nonce: Uuid::now_v7(),
            speak: true,
            share_screen: false,
            transfer_files: false,
            camera: false,
        }
    }

    #[test]
    fn round_trips_and_refuses_what_it_should() {
        let claims = claims();
        let token = sign(&claims, b"secret");
        let server = claims.servers[1];
        assert_eq!(verify(&token, b"secret", server, 999), Ok(claims.clone()));
        assert_eq!(
            verify(&token, b"secret", server, 1_000),
            Err(TokenError::Expired)
        );
        assert_eq!(
            verify(&token, b"other", server, 999),
            Err(TokenError::BadSignature)
        );
        assert_eq!(
            verify(&token, b"secret", Uuid::now_v7(), 999),
            Err(TokenError::WrongServer)
        );
        let mut forged = token.clone();
        forged.replace_range(0..4, "AAAA");
        assert_eq!(
            verify(&forged, b"secret", server, 999),
            Err(TokenError::BadSignature)
        );
        assert_eq!(
            verify("nodot", b"secret", server, 999),
            Err(TokenError::Malformed)
        );
    }

    #[test]
    fn grants_decide_what_may_be_produced() {
        use crate::signal::MediaSource;
        let claims = claims();
        assert!(claims.may_produce(MediaSource::Microphone));
        assert!(!claims.may_produce(MediaSource::Screen));
        assert!(!claims.may_produce(MediaSource::ScreenAudio));
    }
}
