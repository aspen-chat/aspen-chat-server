use crate::api::message_enum;
use crate::api::message_enum::server_event::{ServerEvent, UserStatusEvent};
use crate::api::user::UserOnlineStatus;
use crate::app::{self, ASPEN_NATS_STREAM_NAME, UserId};
use std::sync::Arc;
use tracing::warn;

async fn publish_status_event(
    nats: &async_nats::jetstream::Context,
    user_id: UserId,
    status: UserOnlineStatus,
) -> app::error::Result<()> {
    let event = ServerEvent::UserStatus(UserStatusEvent::Create(message_enum::UserStatus {
        id: user_id,
        status,
    }));
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
pub fn spawn_expiry_listener(
    valkey: fred::clients::Client,
    nats: Arc<async_nats::jetstream::Context>,
) -> tokio::task::JoinHandle<Result<(), fred::error::Error>> {
    use fred::prelude::{EventInterface, KeysInterface};

    let valkey_for_lock = valkey.clone();
    valkey.on_keyspace_event(move |event| {
        let nats = nats.clone();
        let valkey = valkey_for_lock.clone();
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
