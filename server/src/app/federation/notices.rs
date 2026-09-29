//! Notices: what one deployment tells another about that deployment's users, server to server.
//! A notice is a signed statement (`aspen-notice+jwt`) addressed to one user's home, POSTed to
//! `/api/v1/federation/notices` there, naming the user by their id at home. What it says is its
//! `kind`; a kind the receiver does not know it accepts and ignores (`spec/federation.md`).
//!
//! The one kind so far, `dmJoined`: a deployment that hosts a DM tells each of its participants
//! from elsewhere that they are in it, when it is started with them or they are added to it, and
//! their home tells their devices (`foreignDmJoined`), which sign in there if they are not and
//! read it. A home passes on notices only from deployments its user still uses, so one they left
//! stays left.

use crate::api::GlobalServerContext;
use crate::api::message_enum::server_event::ServerEvent;
use crate::app::federation::keys::signing_key;
use crate::app::federation::received::{Received, Statement, receive};
use crate::app::federation::{Direction, Domain, Subject, admits, jws, own_domain};
use crate::app::{self, ChannelId, EventScope, UserId, publish_event};
use crate::database::schema::{user, user_foreign_deployment};
use chrono::{Duration, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The `typ` of a notice.
const NOTICE_TYPE: &str = "aspen-notice+jwt";
/// How long a notice is good for once signed.
const NOTICE_LIFETIME: Duration = Duration::minutes(2);
/// Where a home receives notices, under the API.
pub const NOTICES_PATH: &str = "/federation/notices";
/// How long to wait before each further attempt to deliver a notice its receiver did not take.
const RETRY_DELAYS: [std::time::Duration; 3] = [
    std::time::Duration::from_secs(2),
    std::time::Duration::from_secs(15),
    std::time::Duration::from_secs(60),
];

/// A notice about one user, to their home.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// The deployment telling.
    pub iss: Domain,
    /// The user's home.
    pub aud: Domain,
    /// The user's id at home.
    pub sub: Uuid,
    pub iat: i64,
    pub exp: i64,
    pub jti: Uuid,
    #[serde(flatten)]
    pub about: About,
}

/// What a notice says.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum About {
    /// The user is in a DM on the deployment telling: started with them, or they were added.
    DmJoined {
        /// The DM, by the telling deployment's id.
        channel: ChannelId,
        /// Who started it or added them.
        by: Person,
    },
    /// A kind this deployment does not know, sent by a newer one; accepted and ignored.
    #[serde(other)]
    Unknown,
}

/// Someone a notice names, as the telling deployment shows them.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    pub display_name: Option<String>,
}

impl Statement for Notice {
    const TYPE: &'static str = NOTICE_TYPE;
    fn issuer(&self) -> &Domain {
        &self.iss
    }
    fn audience(&self) -> &Domain {
        &self.aud
    }
    fn issued_at(&self) -> i64 {
        self.iat
    }
    fn expires_at(&self) -> i64 {
        self.exp
    }
    fn id(&self) -> Uuid {
        self.jti
    }
}

/// Tells the homes of the users of `joined` from elsewhere that `by` put them in the DM
/// `channel` here. Delivered in the background after the change is committed, each notice tried
/// again a few times while its receiver does not take it; one never taken is only logged, since
/// the user's devices signed in here learn of the DM from this deployment's own events anyway.
pub fn announce_dm(
    state: &GlobalServerContext,
    channel: ChannelId,
    joined: Vec<UserId>,
    by: UserId,
) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = announce_dm_now(&state, channel, &joined, by).await {
            tracing::warn!(%error, "could not announce a DM to other deployments");
        }
    });
}

async fn announce_dm_now(
    state: &GlobalServerContext,
    channel: ChannelId,
    joined: &[UserId],
    by: UserId,
) -> app::Result<()> {
    let Some(here) = own_domain(&state.config.federation) else {
        return Ok(());
    };
    let mut conn = state.connection_pool.get().await?;
    let foreign: Vec<(Option<Domain>, Option<Uuid>)> = user::table
        .select((user::home_domain, user::home_id))
        .filter(user::id.eq_any(joined))
        .filter(user::id.ne(by))
        .filter(user::home_domain.is_not_null())
        .load(&mut conn)
        .await?;
    if foreign.is_empty() {
        return Ok(());
    }
    let (name, display_name): (String, Option<String>) = user::table
        .select((user::name, user::display_name))
        .find(by)
        .first(&mut conn)
        .await?;
    let Some(key) = signing_key(&mut conn).await? else {
        return Ok(());
    };
    drop(conn);
    for (home, home_id) in foreign {
        let (Some(home), Some(home_id)) = (home, home_id) else {
            continue;
        };
        let now = Utc::now();
        let notice = jws::sign(
            NOTICE_TYPE,
            key.id,
            &key.pair,
            &Notice {
                iss: here.clone(),
                aud: home.clone(),
                sub: home_id,
                iat: now.timestamp(),
                exp: (now + NOTICE_LIFETIME).timestamp(),
                jti: Uuid::new_v4(),
                about: About::DmJoined {
                    channel,
                    by: Person {
                        name: name.clone(),
                        display_name: display_name.clone(),
                    },
                },
            },
        );
        let state = state.clone();
        tokio::spawn(async move {
            deliver(&state, &home, notice).await;
        });
    }
    Ok(())
}

/// POSTs a notice to its receiver, trying again while it does not take it; a notice is signed
/// for a few minutes, so the last attempt falls within that.
async fn deliver(state: &GlobalServerContext, to: &Domain, notice: String) {
    let url = format!("https://{to}{}{NOTICES_PATH}", crate::api::API_PREFIX);
    let body = serde_json::json!({ "notice": notice });
    for delay in std::iter::once(std::time::Duration::ZERO).chain(RETRY_DELAYS) {
        tokio::time::sleep(delay).await;
        match state.federation_client.post(&url).json(&body).send().await {
            Ok(response) if response.status().is_success() => return,
            // A refusal will not change on its own.
            Ok(response) if response.status().is_client_error() => {
                tracing::info!(%to, status = %response.status(), "a deployment refused a notice");
                return;
            }
            Ok(response) => {
                tracing::info!(%to, status = %response.status(), "a deployment could not take a notice");
            }
            Err(error) => tracing::info!(%to, %error, "could not reach a deployment with a notice"),
        }
    }
    tracing::warn!(%to, "gave up delivering a notice");
}

/// Takes a notice from another deployment about one of this deployment's users, and passes it
/// on to their devices when they still use that deployment.
pub async fn receive_notice(state: &GlobalServerContext, token: &str) -> app::Result<()> {
    let Received {
        claims,
        from,
        lists,
    } = receive::<Notice>(state, token, Direction::Emigration).await?;
    let About::DmJoined { channel, by } = claims.about else {
        return Ok(());
    };
    let mut conn = state.connection_pool.get().await?;
    let found: Option<(UserId, bool)> = user::table
        .select((user::id, user::bot))
        .filter(user::id.eq(claims.sub))
        .filter(user::home_domain.is_null())
        .filter(user::deleted_at.is_null())
        .first(&mut conn)
        .await
        .optional()?;
    let Some((user_id, bot)) = found else {
        return Ok(());
    };
    let subject = if bot { Subject::Bots } else { Subject::Users };
    let uses = diesel::select(diesel::dsl::exists(
        user_foreign_deployment::table
            .filter(user_foreign_deployment::user.eq(user_id))
            .filter(user_foreign_deployment::domain.eq(from.as_str())),
    ))
    .get_result::<bool>(&mut conn)
    .await?;
    if !uses
        || !admits(
            &state.config.federation,
            subject,
            Direction::Emigration,
            &lists,
        )
    {
        return Ok(());
    }
    publish_event(
        state,
        &mut conn,
        EventScope::User(user_id),
        &ServerEvent::ForeignDmJoined {
            domain: from.to_string(),
            channel,
            by_name: by.name,
            by_display_name: by.display_name,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_of_every_known_kind_and_of_kinds_to_come_read() {
        let known: Notice = serde_json::from_str(include_str!(
            "../../../../spec/fixtures/federation/notice-dm-joined.json"
        ))
        .unwrap();
        assert!(matches!(known.about, About::DmJoined { .. }));
        let future: Notice = serde_json::from_str(include_str!(
            "../../../../spec/fixtures/federation/notice-from-newer.json"
        ))
        .unwrap();
        assert!(matches!(future.about, About::Unknown));
    }
}
