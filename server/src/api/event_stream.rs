use std::sync::Arc;

use crate::api::CommunityId;
use anyhow::Error;
use axum::response::{
    Sse,
    sse::{Event, KeepAlive},
};
use futures_util::{Stream, StreamExt};
use tokio_stream::{StreamMap, wrappers::BroadcastStream};

use super::message_enum::server_event::ServerEvent;

pub async fn event_stream() -> Sse<impl Stream<Item = Result<Event, Error>>> {
    let stream: StreamMap<CommunityId, BroadcastStream<Arc<ServerEvent>>> = StreamMap::new();

    // Subscribe to relevant community mailboxes.
    // use schema::community_user;
    // let mailbox_subscriptions = community_user::table
    //     .select(community_user::community)
    //     .filter(community_user::user.eq(user_id))
    //     .load(&mut conn)?
    //     .into_iter()
    //     .map(|c: Uuid| SubscribeCommand {
    //         community: CommunityId::from(c),
    //         desire_subscribed: true,
    //     })
    //     .collect();
    // let mut sess_context_write = session_context.write().await;
    // sess_context_write.signed_in_user = Some(user_id.into());
    // sess_context_write
    //     .community_mailbox_subscribe_commands
    //     .send(mailbox_subscriptions)
    //     .await;

    Sse::new(stream.map(|e| todo!())).keep_alive(KeepAlive::default())
}
