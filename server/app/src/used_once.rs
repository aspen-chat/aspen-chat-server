//! What may be used only once, remembered in PostgreSQL (`used_once`) so that no restart, of this
//! server or of Valkey, makes it usable again. Each claim lasts until what it names could no
//! longer be accepted anyway; `jobs::upkeep::sweep_expired` deletes it after that.

use diesel::sql_types::{BigInt, Bytea, Text};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use sha2::{Digest, Sha256};

/// What a claim is for, stored as its wire name in `used_once.purpose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Purpose {
    /// A solved device challenge, by its random part (`device_credential::redeem`).
    DeviceChallenge,
    /// An identity provider's logout token, by provider and `jti` (`identity_provider::logout`).
    LogoutToken,
    /// A statement from another deployment, by type, sender, and id (`federation::received`).
    FederationStatement,
}

crate::wire_name_traits!(Purpose);

/// The key `parts` are remembered by: a digest of each part with its length, so no two lists of
/// parts share one and a part of any length takes 32 bytes.
fn key_of(parts: &[&[u8]]) -> Vec<u8> {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().to_vec()
}

/// Claims `parts` for `purpose` for the next `seconds` (at least one), on the database's clock:
/// true the first time, false while an earlier claim stands. A claim whose time has passed is
/// taken over, so a row the sweep has not reached yet refuses nothing.
pub async fn claim(
    conn: &mut AsyncPgConnection,
    purpose: Purpose,
    parts: &[&[u8]],
    seconds: i64,
) -> crate::Result<bool> {
    let claimed = diesel::sql_query(
        "INSERT INTO used_once (purpose, key, until) \
         VALUES ($1, $2, now() + make_interval(secs => $3::float8)) \
         ON CONFLICT (purpose, key) DO UPDATE SET until = EXCLUDED.until \
           WHERE used_once.until <= now()",
    )
    .bind::<Text, _>(purpose.to_string())
    .bind::<Bytea, _>(key_of(parts))
    .bind::<BigInt, _>(seconds.max(1))
    .execute(conn)
    .await?;
    Ok(claimed == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel_async::AsyncConnection;

    #[test]
    fn parts_are_told_apart_by_their_lengths() {
        assert_ne!(key_of(&[b"ab", b"c"]), key_of(&[b"a", b"bc"]));
        assert_ne!(key_of(&[b"abc"]), key_of(&[b"abc", b""]));
        assert_eq!(key_of(&[b"a", b"bc"]), key_of(&[b"a", b"bc"]));
    }

    /// Runs against the database `DATABASE_URL` names, inside a transaction that is never
    /// committed; without one it checks nothing.
    #[tokio::test]
    async fn a_claim_refuses_its_repeats_until_it_ends() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        let token = uuid::Uuid::new_v4();
        let parts: &[&[u8]] = &[token.as_bytes()];
        assert!(
            claim(&mut conn, Purpose::LogoutToken, parts, 300)
                .await
                .unwrap()
        );
        assert!(
            !claim(&mut conn, Purpose::LogoutToken, parts, 300)
                .await
                .unwrap()
        );
        // The same parts for another purpose are another claim.
        assert!(
            claim(&mut conn, Purpose::DeviceChallenge, parts, 300)
                .await
                .unwrap()
        );
        // Past its end, a claim no longer refuses, though its row is still there.
        diesel::sql_query(
            "UPDATE used_once SET until = now() - interval '1 second' WHERE purpose = 'logoutToken'",
        )
        .execute(&mut conn)
        .await
        .unwrap();
        assert!(
            claim(&mut conn, Purpose::LogoutToken, parts, 300)
                .await
                .unwrap()
        );
        assert!(
            !claim(&mut conn, Purpose::LogoutToken, parts, 300)
                .await
                .unwrap()
        );
        // A claim asked for no time at all still lasts a second.
        let other = uuid::Uuid::new_v4();
        assert!(
            claim(
                &mut conn,
                Purpose::FederationStatement,
                &[other.as_bytes()],
                -5
            )
            .await
            .unwrap()
        );
        assert!(
            !claim(
                &mut conn,
                Purpose::FederationStatement,
                &[other.as_bytes()],
                -5
            )
            .await
            .unwrap()
        );
    }
}
