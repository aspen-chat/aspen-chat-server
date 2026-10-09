//! The join token: what a client presents to a voice server to prove the API server let it
//! into a channel. The API servers sign it with an Ed25519 key of their own (`sign`), whose
//! public half the voice servers ask them for over NATS (`control::TOKEN_KEY_SUBJECT`), so a
//! voice server can check a token but never make one. A voice server given `token_secret` also
//! takes the shared-secret form (`verify_shared`).

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
    /// The sign-in the token was issued to (the API server's `login::sign_in_id`), so the
    /// participant it admits leaves the call when that sign-in ends
    /// (`VoiceCommand::EndSignIns`). A bot's token, which belongs to no sign-in, has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sign_in: Option<String>,
    /// Whether a moderator's mute of them stands where the call is, so the participant the
    /// token admits is muted from the start (`VoiceCommand::Mute` changes it after).
    #[serde(default)]
    pub server_muted: bool,
}

impl JoinClaims {
    /// What the token lets its holder do in the call, until the API server says otherwise
    /// (`VoiceCommand::Grant`).
    pub fn grants(&self) -> Grants {
        Grants {
            speak: self.speak,
            share_screen: self.share_screen,
            camera: self.camera,
            transfer_files: self.transfer_files,
        }
    }
}

/// What a participant may do in a call besides listen and watch: their channel permissions
/// Speak, Share screen, Use camera, and Transfer files. A join token carries them as they stood
/// when it was issued, and the API server sends them again whenever they change.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct Grants {
    pub speak: bool,
    pub share_screen: bool,
    pub camera: bool,
    pub transfer_files: bool,
}

impl Grants {
    /// Whether they may produce media from `source`.
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
    #[error("the token is not a key id, claims, and signature joined by dots")]
    Malformed,
    #[error("the token's signature does not match")]
    BadSignature,
    #[error("the token is signed with a key this server does not know")]
    UnknownKey,
    #[error("the token expired")]
    Expired,
    #[error("the token is not for this server")]
    WrongServer,
    #[error("the token was already used; ask the API server for another")]
    Used,
}

/// Names a signing key by its public half: the first twelve bytes of its SHA-256, base64url.
/// A token names the key it is signed with by this, so a voice server finds which key to verify
/// it with (`control::TokenKey`).
pub fn key_id(public_key: &[u8]) -> String {
    use sha2::Digest;
    URL_SAFE_NO_PAD.encode(&Sha256::digest(public_key)[..12])
}

/// Signs `claims` with the API servers' join token key: the key's id, the base64url claims, and
/// the base64url Ed25519 signature of the first two parts as they stand, joined by dots.
pub fn sign(claims: &JoinClaims, key: &aws_lc_rs::signature::Ed25519KeyPair) -> String {
    use aws_lc_rs::signature::KeyPair;
    let payload = serde_json::to_vec(claims).expect("claims serialize");
    let signed = format!(
        "{}.{}",
        key_id(key.public_key().as_ref()),
        URL_SAFE_NO_PAD.encode(payload)
    );
    let signature = key.sign(signed.as_bytes());
    format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}

/// The id of the key `token` says it is signed with, or `None` when it is not a signed token
/// (a shared-secret one has two parts).
pub fn key_of(token: &str) -> Option<&str> {
    let mut parts = token.split('.');
    let (Some(key), Some(_), Some(_), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    Some(key)
}

/// Verifies a signed `token` with `public_key`, the Ed25519 key its id names, for `server` at
/// time `now` (seconds since the Unix epoch), and returns its claims. The signature is checked
/// before the claims are read.
pub fn verify(
    token: &str,
    public_key: &[u8],
    server: Uuid,
    now: i64,
) -> Result<JoinClaims, TokenError> {
    let key = key_of(token).ok_or(TokenError::Malformed)?;
    if key != key_id(public_key) {
        return Err(TokenError::UnknownKey);
    }
    let (signed, signature) = token.rsplit_once('.').ok_or(TokenError::Malformed)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| TokenError::Malformed)?;
    aws_lc_rs::signature::UnparsedPublicKey::new(&aws_lc_rs::signature::ED25519, public_key)
        .verify(signed.as_bytes(), &signature)
        .map_err(|_| TokenError::BadSignature)?;
    let (_, payload) = signed.split_once('.').ok_or(TokenError::Malformed)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| TokenError::Malformed)?;
    let claims: JoinClaims = serde_json::from_slice(&payload).map_err(|_| TokenError::Malformed)?;
    admits(claims, server, now)
}

type HmacSha256 = Hmac<Sha256>;

/// Verifies a token of the shared-secret form (the base64url claims, a dot, and their
/// HMAC-SHA256 under `secret`), which a voice server accepts only while it is given
/// `token_secret`, so that it takes the tokens of API servers that do not yet sign with a key
/// of their own. The signature is checked before anything else is read.
pub fn verify_shared(
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
    admits(claims, server, now)
}

/// The claims, if they admit their holder to `server` at `now`.
fn admits(claims: JoinClaims, server: Uuid, now: i64) -> Result<JoinClaims, TokenError> {
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
            sign_in: Some("0123456789abcdef0123456789abcdef".to_string()),
            server_muted: false,
        }
    }

    fn key() -> aws_lc_rs::signature::Ed25519KeyPair {
        let document = aws_lc_rs::signature::Ed25519KeyPair::generate_pkcs8(
            &aws_lc_rs::rand::SystemRandom::new(),
        )
        .unwrap();
        aws_lc_rs::signature::Ed25519KeyPair::from_pkcs8(document.as_ref()).unwrap()
    }

    #[test]
    fn signed_tokens_round_trip_and_refuse_what_they_should() {
        use aws_lc_rs::signature::KeyPair;
        let (key, other) = (key(), key());
        let public = key.public_key().as_ref().to_vec();
        let claims = claims();
        let token = sign(&claims, &key);
        let server = claims.servers[1];
        assert_eq!(key_of(&token), Some(key_id(&public).as_str()));
        assert_eq!(verify(&token, &public, server, 999), Ok(claims.clone()));
        assert_eq!(
            verify(&token, &public, server, 1_000),
            Err(TokenError::Expired)
        );
        assert_eq!(
            verify(&token, other.public_key().as_ref(), server, 999),
            Err(TokenError::UnknownKey)
        );
        assert_eq!(
            verify(&token, &public, Uuid::now_v7(), 999),
            Err(TokenError::WrongServer)
        );
        // Claims changed under the signature, or another key's signature under this key's id.
        let (signed, _) = token.rsplit_once('.').unwrap();
        let (kid, payload) = signed.split_once('.').unwrap();
        let mut changed = claims.clone();
        changed.speak = false;
        let forged = format!(
            "{kid}.{}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&changed).unwrap()),
            token.rsplit_once('.').unwrap().1
        );
        assert_eq!(
            verify(&forged, &public, server, 999),
            Err(TokenError::BadSignature)
        );
        let impostor = sign(&claims, &other);
        let (_, rest) = impostor.split_once('.').unwrap();
        assert_eq!(
            verify(&format!("{kid}.{rest}"), &public, server, 999),
            Err(TokenError::BadSignature)
        );
        assert_ne!(payload, "");
        assert_eq!(key_of("one.two"), None);
        assert_eq!(
            verify("one.two", &public, server, 999),
            Err(TokenError::Malformed)
        );
    }

    #[test]
    fn shared_secret_tokens_are_checked_under_the_secret() {
        let claims = claims();
        let payload = serde_json::to_vec(&claims).unwrap();
        let mut mac = HmacSha256::new_from_slice(b"secret").unwrap();
        mac.update(&payload);
        let token = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&payload),
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        );
        let server = claims.servers[0];
        assert_eq!(
            verify_shared(&token, b"secret", server, 999),
            Ok(claims.clone())
        );
        assert_eq!(
            verify_shared(&token, b"other", server, 999),
            Err(TokenError::BadSignature)
        );
        assert_eq!(
            verify_shared("nodot", b"secret", server, 999),
            Err(TokenError::Malformed)
        );
    }

    #[test]
    fn grants_decide_what_may_be_produced() {
        use crate::signal::MediaSource;
        let claims = claims();
        assert!(claims.grants().may_produce(MediaSource::Microphone));
        assert!(!claims.grants().may_produce(MediaSource::Screen));
        assert!(!claims.grants().may_produce(MediaSource::ScreenAudio));
    }
}
