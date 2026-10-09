//! Secrets the deployment's servers share and nothing outside them knows, kept in
//! `server_secret`, each made by the first server to start and read by every one as it starts.

use crate::CHACHA_RNG;
use aspen_schema::server_secret;
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use rand::RngExt;

/// The name of the secret that keys [`CodeKey`].
const CODES: &str = "codes";
/// The name of the secret that is [`JoinTokenKey`], an Ed25519 key's PKCS#8 document.
const JOIN_TOKENS: &str = "joinTokens";

/// Makes the secret named `name` with `make` when the deployment has none, and reads it. Servers
/// starting together make one: the name is the key of its row.
async fn load_or_make(
    conn: &mut AsyncPgConnection,
    name: &str,
    make: impl FnOnce() -> crate::Result<Vec<u8>>,
) -> crate::Result<Vec<u8>> {
    let existing: Option<Vec<u8>> = server_secret::table
        .select(server_secret::secret)
        .filter(server_secret::name.eq(name))
        .first(conn)
        .await
        .optional()?;
    if let Some(secret) = existing {
        return Ok(secret);
    }
    diesel::insert_into(server_secret::table)
        .values((
            server_secret::name.eq(name),
            server_secret::secret.eq(make()?),
        ))
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    Ok(server_secret::table
        .select(server_secret::secret)
        .filter(server_secret::name.eq(name))
        .first(conn)
        .await?)
}

fn key_error(message: &str) -> crate::Error {
    crate::Error::Config(config::ConfigError::Message(message.to_string()))
}

/// The key the API servers sign join tokens with (`voice_protocol::token::sign`). The voice
/// servers hold only its public half, which they ask for over NATS
/// (`app::voice::spawn_token_key_answerer`), so a voice server taken over can check tokens but
/// not make them.
#[derive(Clone)]
pub struct JoinTokenKey(std::sync::Arc<aws_lc_rs::signature::Ed25519KeyPair>);

impl std::fmt::Debug for JoinTokenKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JoinTokenKey(..)")
    }
}

impl JoinTokenKey {
    /// Makes the deployment's join token key when it has none, and reads it.
    pub async fn load(conn: &mut AsyncPgConnection) -> crate::Result<Self> {
        let invalid = |_| key_error("the join token key is not an Ed25519 key");
        let document = load_or_make(conn, JOIN_TOKENS, || {
            aws_lc_rs::signature::Ed25519KeyPair::generate_pkcs8(
                &aws_lc_rs::rand::SystemRandom::new(),
            )
            .map(|document| document.as_ref().to_vec())
            .map_err(|_| key_error("could not make a join token key"))
        })
        .await?;
        let pair = aws_lc_rs::signature::Ed25519KeyPair::from_pkcs8(&document).map_err(invalid)?;
        Ok(Self(std::sync::Arc::new(pair)))
    }

    pub fn pair(&self) -> &aws_lc_rs::signature::Ed25519KeyPair {
        &self.0
    }

    /// Its public half, as voice servers are given it.
    pub fn public(&self) -> voice_protocol::control::TokenKey {
        use aws_lc_rs::signature::KeyPair;
        let public = self.0.public_key().as_ref();
        voice_protocol::control::TokenKey {
            key_id: voice_protocol::token::key_id(public),
            public_key: BASE64_URL_SAFE_NO_PAD.encode(public),
        }
    }
}

/// The key mailed codes are kept under in Valkey (`app::email`): a short code's plain digest
/// could be reversed by trying every code, by whoever reads Valkey, while its HMAC under a key
/// only the database holds cannot.
#[derive(Clone)]
pub struct CodeKey(aws_lc_rs::hmac::Key);

impl std::fmt::Debug for CodeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodeKey(..)")
    }
}

impl CodeKey {
    /// Makes the deployment's code key when it has none, and reads it. Servers starting together
    /// make one: the name is the key of its row.
    pub async fn load(conn: &mut AsyncPgConnection) -> crate::Result<Self> {
        let made = CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>());
        diesel::insert_into(server_secret::table)
            .values((
                server_secret::name.eq(CODES),
                server_secret::secret.eq(made.as_slice()),
            ))
            .on_conflict_do_nothing()
            .execute(conn)
            .await?;
        let secret: Vec<u8> = server_secret::table
            .select(server_secret::secret)
            .filter(server_secret::name.eq(CODES))
            .first(conn)
            .await?;
        Ok(Self::from_secret(&secret))
    }

    fn from_secret(secret: &[u8]) -> Self {
        Self(aws_lc_rs::hmac::Key::new(
            aws_lc_rs::hmac::HMAC_SHA256,
            secret,
        ))
    }

    /// What `code` is kept as: its HMAC, with surrounding space taken off, as the person may have
    /// typed it.
    pub fn digest(&self, code: &str) -> String {
        BASE64_URL_SAFE_NO_PAD.encode(aws_lc_rs::hmac::sign(&self.0, code.trim().as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A code's digest depends on the key, and not on space around what was typed.
    #[test]
    fn a_digest_is_keyed() {
        let one = CodeKey::from_secret(b"one");
        let other = CodeKey::from_secret(b"other");
        assert_eq!(one.digest("123456"), one.digest(" 123456\n"));
        assert_ne!(one.digest("123456"), one.digest("123457"));
        assert_ne!(one.digest("123456"), other.digest("123456"));
    }
}
