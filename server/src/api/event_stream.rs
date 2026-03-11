use axum::extract::WebSocketUpgrade;
use axum::extract::ws::WebSocket;
use axum::response::Response;

use super::message_enum::server_event::ServerEvent;

pub async fn event_stream(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(handle_socket_conn)
}

async fn handle_socket_conn(mut socket: WebSocket) {
    while let Some(msg) = socket.recv().await {
        let msg = if let Ok(msg) = msg {
            msg
        } else {
            // client disconnected
            return;
        };

        if socket.send(msg).await.is_err() {
            // client disconnected
            return;
        }
    }
}
