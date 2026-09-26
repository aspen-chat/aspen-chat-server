use crate::api::message_enum::server_event::ServerEvent;
use crate::api::user::UserOnlineStatus;
use crate::app::{self, ASPEN_NATS_STREAM_NAME, UserId};
use std::sync::Arc;
use tracing::warn;

async fn publish_status_event(
    nats: &async_nats::jetstream::Context,
    user_id: UserId,
    status: UserOnlineStatus,
) -> app::error::Result<()> {
    let event = ServerEvent::UserStatus {
        id: user_id,
        status,
    };
    nats.publish(
        ASPEN_NATS_STREAM_NAME,
        serde_json::to_string(&event)?.into_bytes().into(),
    )
    .await?
    .await?;
    Ok(())
}

pub async fn publish_online(
    nats: &async_nats::jetstream::Context,
    user_id: UserId,
) -> app::error::Result<()> {
    publish_status_event(nats, user_id, UserOnlineStatus::Online).await
}

pub async fn publish_offline(
    nats: &async_nats::jetstream::Context,
    user_id: UserId,
) -> app::error::Result<()> {
    publish_status_event(nats, user_id, UserOnlineStatus::Offline).await
}

const ONLINE_KEY_PREFIX: &str = "user:";
const ONLINE_KEY_SUFFIX: &str = ":online";

/// The Valkey key whose presence means the user is online: `user:{uuid}:online`. The uuid is
/// written bare, not through `UserId`'s `Display`, so that `parse_user_id_from_key` can read
/// it back when the key expires.
pub fn online_key(user_id: UserId) -> String {
    format!("{ONLINE_KEY_PREFIX}{}{ONLINE_KEY_SUFFIX}", user_id.0)
}

/// Parse a user ID from a Valkey key of the form `user:{uuid}:online`.
fn parse_user_id_from_key(key: &str) -> Option<UserId> {
    let rest = key.strip_prefix(ONLINE_KEY_PREFIX)?;
    let uuid_str = rest.strip_suffix(ONLINE_KEY_SUFFIX)?;
    uuid::Uuid::parse_str(uuid_str).ok().map(UserId::from)
}

/// Subscribes to Valkey keyspace expiry notifications and publishes offline events.
///
/// Uses a distributed lock (`SET NX EX 5`) to ensure only one server instance
/// publishes the offline event when multiple API servers are behind a load balancer.
///
/// `subscriber` is the connection that holds the `psubscribe`; `commands` is an ordinary
/// connection. They must be different clients: a connection in subscribe mode accepts nothing
/// but subscription commands, so the lock's `SET` would be refused on `subscriber`.
pub fn spawn_expiry_listener(
    subscriber: fred::clients::Client,
    commands: fred::clients::Client,
    nats: Arc<async_nats::jetstream::Context>,
) -> tokio::task::JoinHandle<Result<(), fred::error::Error>> {
    use fred::prelude::{EventInterface, KeysInterface};

    subscriber.on_keyspace_event(move |event| {
        let nats = nats.clone();
        let valkey = commands.clone();
        async move {
            if event.operation != "expired" {
                return Ok(());
            }
            let Some(key_str) = event.key.as_str() else {
                return Ok(());
            };
            let Some(user_id) = parse_user_id_from_key(key_str) else {
                return Ok(());
            };

            // Attempt to acquire a short-lived distributed lock.
            // Only the server that wins the lock publishes the offline event.
            let lock_key = format!("user:{}:offline_lock", user_id);
            let acquired: bool = match valkey
                .set::<fred::types::Value, _, _>(
                    &lock_key,
                    "1",
                    Some(fred::types::Expiration::EX(5)),
                    Some(fred::types::SetOptions::NX),
                    false,
                )
                .await
            {
                Ok(v) => !v.is_null(),
                Err(e) => {
                    warn!(error = %e, "failed to acquire offline lock in Valkey");
                    return Ok(());
                }
            };

            if acquired && let Err(e) = publish_offline(&nats, user_id).await {
                warn!(error = %e, %user_id, "failed to publish user offline event");
            }

            Ok(())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key the presence writer produces must be the key the expiry listener recognises,
    /// or users never go offline.
    #[test]
    fn online_key_round_trips_through_the_parser() {
        let id = UserId::new();
        assert_eq!(parse_user_id_from_key(&online_key(id)), Some(id));
        assert_eq!(parse_user_id_from_key(&format!("user:{id}:online")), None);
        assert_eq!(
            parse_user_id_from_key(&format!("user:{}:offline_lock", id.0)),
            None
        );
    }
}
