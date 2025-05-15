use std::sync::Arc;

use anyhow::Error;
use axum::response::{
    Sse,
    sse::{Event, KeepAlive},
};
use futures_util::{Stream, StreamExt};
use tokio_stream::{StreamMap, wrappers::BroadcastStream};
use crate::api::CommunityId;

use super::message_enum::server_event::ServerEvent;

pub async fn event_stream() -> Sse<impl Stream<Item = Result<Event, Error>>> {
    let stream: StreamMap<CommunityId, BroadcastStream<Arc<ServerEvent>>> = StreamMap::new();

    Sse::new(stream.map(|e| todo!())).keep_alive(KeepAlive::default())
}
