//! Short-lived secrets kept in Valkey: second-factor sign-in tickets and passkey ceremonies,
//! each stored under a digest of its token and expiring on its own.

use crate::context::GlobalServerContext;
use fred::interfaces::KeysInterface;
use fred::types::Expiration;
use sha2::{Digest, Sha256};

/// Shared by every short-lived secret kept in Valkey: its key holds a digest of the token, so a
/// dump of the store does not hand out usable tokens. For the same reason a value that acts on a
/// sign-in names it by `login::sign_in_id`, never by its refresh or session token, and one that
/// waits on another secret here names it by its key (`login::TicketKey`).
pub fn token_key(prefix: &str, token: &str) -> String {
    format!(
        "{prefix}:{}",
        data_encoding::HEXLOWER.encode(&Sha256::digest(token.as_bytes()))
    )
}

/// Stores a short-lived value under `token_key(prefix, token)`.
pub async fn put_token<V: serde::Serialize>(
    state: &GlobalServerContext,
    prefix: &str,
    token: &str,
    value: &V,
    ttl_seconds: i64,
) -> crate::Result<()> {
    let _: () = state
        .valkey
        .set(
            token_key(prefix, token),
            serde_json::to_string(value)?,
            Some(Expiration::EX(ttl_seconds)),
            None,
            false,
        )
        .await?;
    Ok(())
}

/// Reads a value stored with `put_token`, removing it when `take` is set so it is used once.
pub async fn get_token<V: serde::de::DeserializeOwned>(
    state: &GlobalServerContext,
    prefix: &str,
    token: &str,
    take: bool,
) -> crate::Result<Option<V>> {
    get_at(state, token_key(prefix, token), take).await
}

/// Reads a value stored with `put_token` by its key (`token_key`), for what keeps the key in
/// place of the token, removing it when `take` is set.
pub async fn get_at<V: serde::de::DeserializeOwned>(
    state: &GlobalServerContext,
    key: String,
    take: bool,
) -> crate::Result<Option<V>> {
    let raw: Option<String> = if take {
        state.valkey.getdel(key).await?
    } else {
        state.valkey.get(key).await?
    };
    raw.map(|raw| serde_json::from_str(&raw).map_err(crate::Error::from))
        .transpose()
}
