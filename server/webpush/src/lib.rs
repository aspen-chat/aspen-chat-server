//! What a deployment sends to a push endpoint: a Web Push message (RFC 8030) encrypted to the
//! phone (RFC 8291, `aes128gcm` content coding, RFC 8188) and signed with the deployment's push
//! key (RFC 8292, VAPID), as `spec/push.md` describes.

use aws_lc_rs::aead::{AES_128_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::agreement::{ECDH_P256, EphemeralPrivateKey, UnparsedPublicKey};
use aws_lc_rs::hkdf::{HKDF_SHA256, KeyType, Salt};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

/// The largest message a push service accepts, encrypted (RFC 8030 and every service after it).
pub const MAX_MESSAGE_BYTES: usize = 4096;
/// The record size written in the header. One record carries the whole message, so any size
/// above it will do; 4096 is what every implementation writes.
const RECORD_SIZE: u32 = 4096;
/// The bytes encryption adds: the header (salt, record size, key id length, a 65-byte key) and
/// the padding delimiter and authentication tag of the one record.
pub const OVERHEAD: usize = 16 + 4 + 1 + 65 + 1 + 16;
/// How long a VAPID token is good for; RFC 8292 allows at most a day.
const TOKEN_LIFETIME_SECONDS: i64 = 12 * 60 * 60;
/// A token is reused for pushes to the same service until less than this much of it is left.
const TOKEN_RENEW_SECONDS: i64 = TOKEN_LIFETIME_SECONDS / 2;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WebPushError {
    #[error("the subscription's public key is not a P-256 point")]
    PublicKey,
    #[error("the subscription's authentication secret is not 16 bytes")]
    AuthSecret,
    #[error("the message is too long to push")]
    TooLong,
    #[error("the push key cannot sign")]
    Key,
    #[error("no randomness")]
    Random,
}

/// Encrypts `plaintext` to the phone whose subscription holds `ua_public` (an uncompressed
/// P-256 point) and `auth_secret`, with a fresh key pair and salt.
pub fn encrypt(
    plaintext: &[u8],
    ua_public: &[u8],
    auth_secret: &[u8],
) -> Result<Vec<u8>, WebPushError> {
    if plaintext.len() + OVERHEAD > MAX_MESSAGE_BYTES {
        return Err(WebPushError::TooLong);
    }
    let rng = SystemRandom::new();
    let mut salt = [0u8; 16];
    rng.fill(&mut salt).map_err(|_| WebPushError::Random)?;
    let private =
        EphemeralPrivateKey::generate(&ECDH_P256, &rng).map_err(|_| WebPushError::Random)?;
    let as_public = private
        .compute_public_key()
        .map_err(|_| WebPushError::Random)?;
    aws_lc_rs::agreement::agree_ephemeral(
        private,
        UnparsedPublicKey::new(&ECDH_P256, ua_public),
        WebPushError::PublicKey,
        |ecdh_secret| {
            seal(
                plaintext,
                ecdh_secret,
                ua_public,
                as_public.as_ref(),
                auth_secret,
                &salt,
            )
        },
    )
}

/// Everything encryption does after the key agreement, which is what RFC 8291's example lets
/// be checked step by step.
fn seal(
    plaintext: &[u8],
    ecdh_secret: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    auth_secret: &[u8],
    salt: &[u8; 16],
) -> Result<Vec<u8>, WebPushError> {
    if auth_secret.len() != 16 {
        return Err(WebPushError::AuthSecret);
    }
    if ua_public.len() != 65 || as_public.len() != 65 {
        return Err(WebPushError::PublicKey);
    }
    let key_info = [b"WebPush: info\0".as_slice(), ua_public, as_public].concat();
    let ikm: [u8; 32] = expand(auth_secret, ecdh_secret, &key_info);
    let cek: [u8; 16] = expand(salt, &ikm, b"Content-Encoding: aes128gcm\0");
    let nonce: [u8; 12] = expand(salt, &ikm, b"Content-Encoding: nonce\0");
    let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &cek).map_err(|_| WebPushError::Key)?);
    // The one record, ending with the delimiter that marks it the last.
    let mut record = [plaintext, &[2u8]].concat();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::empty(),
        &mut record,
    )
    .map_err(|_| WebPushError::Key)?;
    let mut message = Vec::with_capacity(OVERHEAD + plaintext.len());
    message.extend_from_slice(salt);
    message.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    message.push(65);
    message.extend_from_slice(as_public);
    message.extend_from_slice(&record);
    Ok(message)
}

/// HKDF-SHA256 (RFC 5869) of `ikm` with `salt`, expanded under `info` to `N` bytes.
fn expand<const N: usize>(salt: &[u8], ikm: &[u8], info: &[u8]) -> [u8; N] {
    struct Length(usize);
    impl KeyType for Length {
        fn len(&self) -> usize {
            self.0
        }
    }
    let mut out = [0u8; N];
    Salt::new(HKDF_SHA256, salt)
        .extract(ikm)
        .expand(&[info], Length(N))
        .and_then(|okm| okm.fill(&mut out))
        .expect("HKDF-SHA256 expands to at most 255 blocks");
    out
}

/// The deployment's push key, loaded to sign with.
pub struct PushKey {
    pair: EcdsaKeyPair,
    /// The `Authorization` header made for each audience and subject, and when its token
    /// expires. RFC 8292 lets a token be reused until then, so a message waking thousands of
    /// phones through one relay is signed once, not once per phone. Every endpoint's origin is
    /// an audience, and endpoints are chosen by whoever subscribes, so it holds at most
    /// [`MAX_TOKENS`], letting go of the tokens too near expiry to be used first and then of
    /// those nearest it.
    tokens: Mutex<HashMap<(String, Option<String>), Token>>,
}

/// The most `Authorization` headers a key keeps for reuse.
const MAX_TOKENS: usize = 1024;

/// An `Authorization` header made earlier, and when its token expires.
struct Token {
    expires: i64,
    header: String,
}

impl PushKey {
    /// Makes a new key, answering its PKCS #8 document (to keep) and its public key.
    pub fn generate() -> Result<(Vec<u8>, Vec<u8>), WebPushError> {
        let rng = SystemRandom::new();
        let document = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|_| WebPushError::Random)?;
        let key = Self::from_pkcs8(document.as_ref())?;
        Ok((document.as_ref().to_vec(), key.public_key().to_vec()))
    }

    pub fn from_pkcs8(document: &[u8]) -> Result<Self, WebPushError> {
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document)
            .map(|pair| PushKey {
                pair,
                tokens: Mutex::default(),
            })
            .map_err(|_| WebPushError::Key)
    }

    /// The public key, as an uncompressed point: what subscriptions are bound to.
    pub fn public_key(&self) -> &[u8] {
        self.pair.public_key().as_ref()
    }

    /// The `Authorization` header of a push to an endpoint at `audience` (its origin), naming
    /// the deployment by `subject` (`https://…` or `mailto:…`) when it has one: the one made
    /// for them before while at least [`TOKEN_RENEW_SECONDS`] of it is left, else a new one.
    pub fn authorization(
        &self,
        audience: &str,
        subject: Option<&str>,
        now: i64,
    ) -> Result<String, WebPushError> {
        let slot = (audience.to_owned(), subject.map(str::to_owned));
        if let Some(token) = self.tokens.lock().expect("token cache").get(&slot)
            && token.expires - now >= TOKEN_RENEW_SECONDS
        {
            return Ok(token.header.clone());
        }
        let header = self.sign(audience, subject, now)?;
        let mut tokens = self.tokens.lock().expect("token cache");
        if tokens.len() >= MAX_TOKENS && !tokens.contains_key(&slot) {
            tokens.retain(|_, token| token.expires - now >= TOKEN_RENEW_SECONDS);
            while tokens.len() >= MAX_TOKENS {
                let nearest = tokens
                    .iter()
                    .min_by_key(|(_, token)| token.expires)
                    .map(|(slot, _)| slot.clone());
                match nearest {
                    Some(nearest) => tokens.remove(&nearest),
                    None => break,
                };
            }
        }
        tokens.insert(
            slot,
            Token {
                expires: now + TOKEN_LIFETIME_SECONDS,
                header: header.clone(),
            },
        );
        Ok(header)
    }

    fn sign(
        &self,
        audience: &str,
        subject: Option<&str>,
        now: i64,
    ) -> Result<String, WebPushError> {
        #[derive(Serialize)]
        struct Claims<'a> {
            aud: &'a str,
            exp: i64,
            #[serde(skip_serializing_if = "Option::is_none")]
            sub: Option<&'a str>,
        }
        let encode = |value: &[u8]| URL_SAFE_NO_PAD.encode(value);
        let input = format!(
            "{}.{}",
            encode(br#"{"typ":"JWT","alg":"ES256"}"#),
            encode(
                &serde_json::to_vec(&Claims {
                    aud: audience,
                    exp: now + TOKEN_LIFETIME_SECONDS,
                    sub: subject,
                })
                .expect("claims serialize")
            ),
        );
        let signature = self
            .pair
            .sign(&SystemRandom::new(), input.as_bytes())
            .map_err(|_| WebPushError::Key)?;
        Ok(format!(
            "vapid t={input}.{}, k={}",
            encode(signature.as_ref()),
            encode(self.public_key())
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey as SignaturePublicKey};

    fn b64(value: &str) -> Vec<u8> {
        URL_SAFE_NO_PAD.decode(value).unwrap()
    }

    /// RFC 8291, Appendix A: every value after the key agreement.
    #[test]
    fn encryption_matches_rfc_8291() {
        let plaintext = b64("V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24");
        let as_public = b64(
            "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
        );
        let ua_public = b64(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let auth = b64("BTBZMqHH6r4Tts7J_aSIgg");
        let ecdh = b64("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs");

        let key_info = [b"WebPush: info\0".as_slice(), &ua_public, &as_public].concat();
        let ikm: [u8; 32] = expand(&auth, &ecdh, &key_info);
        assert_eq!(
            ikm.to_vec(),
            b64("S4lYMb_L0FxCeq0WhDx813KgSYqU26kOyzWUdsXYyrg")
        );
        let cek: [u8; 16] = expand(&salt, &ikm, b"Content-Encoding: aes128gcm\0");
        assert_eq!(cek.to_vec(), b64("oIhVW04MRdy2XN9CiKLxTg"));
        let nonce: [u8; 12] = expand(&salt, &ikm, b"Content-Encoding: nonce\0");
        assert_eq!(nonce.to_vec(), b64("4h_95klXJ5E_qnoN"));

        let message = seal(&plaintext, &ecdh, &ua_public, &as_public, &auth, &salt).unwrap();
        let header = b64(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
        );
        let ciphertext =
            b64("8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ");
        assert_eq!(message, [header, ciphertext].concat());
        assert_eq!(message.len(), plaintext.len() + OVERHEAD);
    }

    #[test]
    fn a_message_encrypts_to_a_real_key_and_nothing_too_long_does() {
        let rng = SystemRandom::new();
        let phone = EphemeralPrivateKey::generate(&ECDH_P256, &rng).unwrap();
        let public = phone.compute_public_key().unwrap();
        let message = encrypt(b"{}", public.as_ref(), &[7; 16]).unwrap();
        assert_eq!(message.len(), 2 + OVERHEAD);
        assert_eq!(
            encrypt(&[0; MAX_MESSAGE_BYTES], public.as_ref(), &[7; 16]),
            Err(WebPushError::TooLong)
        );
        assert_eq!(
            encrypt(b"{}", &[4; 65], &[7; 16]),
            Err(WebPushError::PublicKey)
        );
        assert_eq!(
            encrypt(b"{}", public.as_ref(), &[7; 15]),
            Err(WebPushError::AuthSecret)
        );
    }

    #[test]
    fn a_vapid_token_is_reused_for_its_audience_until_half_its_life_is_left() {
        let (document, _) = PushKey::generate().unwrap();
        let key = PushKey::from_pkcs8(&document).unwrap();
        let at = |audience: &str, now: i64| key.authorization(audience, None, now).unwrap();
        let first = at("https://push.example", 1_000);
        assert_eq!(
            at("https://push.example", 1_000 + TOKEN_RENEW_SECONDS),
            first
        );
        assert_ne!(at("https://other.example", 1_000), first);
        assert_ne!(
            at("https://push.example", 1_001 + TOKEN_RENEW_SECONDS),
            first
        );
    }

    #[test]
    fn the_vapid_token_cache_is_bounded() {
        let (document, _) = PushKey::generate().unwrap();
        let key = PushKey::from_pkcs8(&document).unwrap();
        for n in 0..MAX_TOKENS + 10 {
            key.authorization(&format!("https://{n}.example"), None, n as i64)
                .unwrap();
        }
        let tokens = key.tokens.lock().unwrap();
        assert_eq!(tokens.len(), MAX_TOKENS);
        // The newest are kept.
        let newest = format!("https://{}.example", MAX_TOKENS + 9);
        assert!(tokens.contains_key(&(newest, None)));
        assert!(!tokens.contains_key(&("https://0.example".to_string(), None)));
    }

    #[test]
    fn a_vapid_token_verifies_with_the_key_it_names() {
        let (document, public) = PushKey::generate().unwrap();
        let key = PushKey::from_pkcs8(&document).unwrap();
        let header = key
            .authorization("https://push.example", Some("https://chat.example"), 1_000)
            .unwrap();
        let rest = header.strip_prefix("vapid t=").unwrap();
        let (token, named) = rest.split_once(", k=").unwrap();
        assert_eq!(b64(named), public);
        let (input, signature) = token.rsplit_once('.').unwrap();
        SignaturePublicKey::new(&ECDSA_P256_SHA256_FIXED, &public)
            .verify(input.as_bytes(), &b64(signature))
            .unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&b64(input.split_once('.').unwrap().1)).unwrap();
        assert_eq!(
            claims,
            serde_json::json!({
                "aud": "https://push.example",
                "exp": 1_000 + TOKEN_LIFETIME_SECONDS,
                "sub": "https://chat.example",
            })
        );
    }
}
