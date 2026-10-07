//! Compact JSON Web Signatures (RFC 7515) with EdDSA over Ed25519 (RFC 8037): what one
//! deployment signs for another. Each kind of statement has its own `typ`, checked when it is
//! verified, so a statement of one kind is never accepted as another.

use aspen_wire::FederationKeyId;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::signature::{ED25519, Ed25519KeyPair, UnparsedPublicKey};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The longest statement read, in bytes; a profile snapshot is the largest part.
pub const MAX_LENGTH: usize = 16 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    alg: String,
    typ: String,
    kid: FederationKeyId,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum JwsError {
    #[error("not a compact JWS")]
    Malformed,
    #[error("not an EdDSA signature")]
    Algorithm,
    #[error("a statement of another kind")]
    Type,
    #[error("the signature does not verify")]
    Signature,
    #[error("the claims do not parse")]
    Claims,
}

/// Signs `claims` as a statement of kind `typ` with `key`, named `kid`.
pub fn sign<T: Serialize>(
    typ: &str,
    kid: FederationKeyId,
    key: &Ed25519KeyPair,
    claims: &T,
) -> String {
    let header = Header {
        alg: "EdDSA".into(),
        typ: typ.into(),
        kid,
    };
    let input = format!("{}.{}", encode(&header), encode(claims));
    let signature = key.sign(input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}

fn encode<T: Serialize + ?Sized>(value: &T) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).expect("statements serialize"))
}

/// A statement read but not yet verified. The key that verifies it comes from what the
/// verifier pinned, never from the statement; the `kid` its header names is for people
/// reading it, and at most a hint at which key to try first.
#[derive(Debug)]
pub struct Unverified<'a> {
    typ: String,
    kid: FederationKeyId,
    signing_input: &'a str,
    payload: Vec<u8>,
    signature: Vec<u8>,
}

/// Reads a statement's parts; nothing in it is to be believed until [`Unverified::verify`].
pub fn parse(token: &str) -> Result<Unverified<'_>, JwsError> {
    if token.len() > MAX_LENGTH {
        return Err(JwsError::Malformed);
    }
    let (signing_input, signature) = token.rsplit_once('.').ok_or(JwsError::Malformed)?;
    let (header, payload) = signing_input.split_once('.').ok_or(JwsError::Malformed)?;
    let decode = |part: &str| {
        URL_SAFE_NO_PAD
            .decode(part)
            .map_err(|_| JwsError::Malformed)
    };
    let header: Header =
        serde_json::from_slice(&decode(header)?).map_err(|_| JwsError::Malformed)?;
    if header.alg != "EdDSA" {
        return Err(JwsError::Algorithm);
    }
    Ok(Unverified {
        typ: header.typ,
        kid: header.kid,
        signing_input,
        payload: decode(payload)?,
        signature: decode(signature)?,
    })
}

impl Unverified<'_> {
    /// The claims as the statement makes them, before anything verifies them: only for
    /// finding the key to verify with, never to be believed.
    pub fn peek<T: DeserializeOwned>(&self) -> Result<T, JwsError> {
        serde_json::from_slice(&self.payload).map_err(|_| JwsError::Claims)
    }

    /// The key its header says signed it: only a hint at which key to try first, never to
    /// be believed.
    pub fn kid(&self) -> FederationKeyId {
        self.kid
    }

    /// The claims, once the signature verifies with `public_key` and the statement is of kind
    /// `typ`.
    pub fn verify<T: DeserializeOwned>(&self, typ: &str, public_key: &[u8]) -> Result<T, JwsError> {
        if self.typ != typ {
            return Err(JwsError::Type);
        }
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(self.signing_input.as_bytes(), &self.signature)
            .map_err(|_| JwsError::Signature)?;
        serde_json::from_slice(&self.payload).map_err(|_| JwsError::Claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::KeyPair;

    fn key() -> Ed25519KeyPair {
        let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(document.as_ref()).unwrap()
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Claims {
        said: String,
    }

    #[test]
    fn a_statement_verifies_only_with_its_key_and_kind() {
        let (signer, other) = (key(), key());
        let kid = FederationKeyId::new();
        let claims = Claims {
            said: "hello".into(),
        };
        let token = sign("test", kid, &signer, &claims);
        let read = parse(&token).unwrap();
        let public = signer.public_key().as_ref();
        assert_eq!(read.verify::<Claims>("test", public).unwrap(), claims);
        assert_eq!(read.verify::<Claims>("other", public), Err(JwsError::Type));
        assert_eq!(
            read.verify::<Claims>("test", other.public_key().as_ref()),
            Err(JwsError::Signature)
        );
    }

    #[test]
    fn a_changed_statement_does_not_verify() {
        let signer = key();
        let token = sign(
            "test",
            FederationKeyId::new(),
            &signer,
            &Claims { said: "a".into() },
        );
        let (header, rest) = token.split_once('.').unwrap();
        let (_, signature) = rest.split_once('.').unwrap();
        let forged = format!(
            "{header}.{}.{signature}",
            URL_SAFE_NO_PAD.encode(br#"{"said":"b"}"#)
        );
        assert_eq!(
            parse(&forged)
                .unwrap()
                .verify::<Claims>("test", signer.public_key().as_ref()),
            Err(JwsError::Signature)
        );
        assert_eq!(parse("a.b").unwrap_err(), JwsError::Malformed);
    }
}
