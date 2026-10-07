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

/// The key mailed codes are kept under in Valkey (`app::email`): a short code's plain digest
/// could be reversed by trying every code, by whoever reads Valkey, while its HMAC under a key
/// only the database holds cannot.
#[derive(Clone)]
pub struct CodeKey(ring::hmac::Key);

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
        Self(ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret))
    }

    /// What `code` is kept as: its HMAC, with surrounding space taken off, as the person may have
    /// typed it.
    pub fn digest(&self, code: &str) -> String {
        BASE64_URL_SAFE_NO_PAD.encode(ring::hmac::sign(&self.0, code.trim().as_bytes()))
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
