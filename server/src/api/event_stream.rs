use anyhow::Error;
use axum::response::{sse::{Event, KeepAlive}, Sse};
use futures_util::Stream;

pub async fn event_stream() -> Sse<impl Stream<Item = Result<Event, Error>>> {
    let stream = todo!();

    Sse::new(stream).keep_alive(KeepAlive::default())
}