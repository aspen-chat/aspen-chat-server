use std::collections::HashMap;
use std::sync::Arc;

use anyhow::bail;
use anyhow::{Result, anyhow};
use argon2::PasswordVerifier;
use diesel::{r2d2::{ConnectionManager, Pool, PooledConnection}, ExpressionMethods, PgConnection, QueryDsl, RunQueryDsl};
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tracing::error;
use tracing::info;
use uuid::Uuid;

use crate::{aspen_protocol::{client_event::{ClientEvent, Error, Login, LoginFailed, LoginSuccess, RegisterUser, ServerResponse}, server_event::ServerEvent, CommunityId, UserId}, database::schema};

pub struct SessionContext {
    pub signed_in_user: Option<UserId>,
    pub community_mail_boxes: HashMap<CommunityId, broadcast::Receiver<ServerEvent>>,
    pub connection_pool: Pool<ConnectionManager<PgConnection>>,
}

impl SessionContext {
    pub fn new(connection_pool: Pool<ConnectionManager<PgConnection>>) -> Self {
        Self {
            signed_in_user: None,
            community_mail_boxes: HashMap::new(),
            connection_pool,
        }
    }
}

pub async fn handle_request(
    session_context: Arc<RwLock<SessionContext>>,
    (mut send, mut recv): (quinn::SendStream, quinn::RecvStream),
) -> Result<()> {
    let req = recv
        .read_to_end(64 * 1024)
        .await
        .map_err(|e| anyhow!("failed reading request: {}", e))?;
    // Execute the request
    make_response(session_context, &req, &mut send)
        .await
        .unwrap_or_else(|e| {
            error!("request failed: {}", e);
        });
    send.finish().unwrap();
    info!("complete");
    Ok(())
}

pub async fn make_response(
    session_context: Arc<RwLock<SessionContext>>,
    input: &[u8],
    send_stream: &mut quinn::SendStream,
) -> Result<()> {
    let client_event = ciborium::from_reader::<ClientEvent, _>(input)
        .map_err(|e| anyhow!("client event read error {e}"))?;
    let conn = session_context.read().await.connection_pool.get()?;
    let resp = match message_handling_logic(session_context, client_event, conn).await {
        Ok(value) => value,
        Err(e) => {
            let uuid = Uuid::now_v7().to_string();
            error!(id = uuid, "request error {e}");
            ServerResponse::Error(Error {
                cause: Some(format!(
                    "Error occurred. Error logged for server admin to review. Error ID: {uuid}"
                )),
            })
        }
    };
    let mut response = Vec::new();
    ciborium::into_writer(&resp, &mut response)?;
    send_stream.write_all(&response).await?;
    Ok(())
}

async fn message_handling_logic(
    session_context: Arc<RwLock<SessionContext>>,
    client_event: ClientEvent,
    mut conn: PooledConnection<ConnectionManager<PgConnection>>,
) -> Result<ServerResponse> {
    let resp: ServerResponse = match client_event {
        ClientEvent::RegisterUser(register_user) => {
            let RegisterUser {
                username,
                password,
                invite_code,
            } = register_user;
            todo!()
        }
        ClientEvent::Login(login) => {
            use schema::user::dsl::*;
            let Login { username, password } = login;
            let user_entry: Result<(Uuid, String), _> = user
                .select((id, password_hash))
                .filter(name.eq(username))
                .first(&mut conn);
            match user_entry {
                Ok((user_id, entry_password_hash)) => {
                    let argon2 = argon2::Argon2::default();
                    let entry_hash =
                        match argon2::PasswordHash::try_from(entry_password_hash.as_str()) {
                            Ok(v) => v,
                            Err(e) => {
                                bail!("user entry password hash malformed in database {e}")
                            }
                        };
                    if argon2
                        .verify_password(password.as_bytes(), &entry_hash)
                        .is_ok()
                    {
                        // Subscribe to relevant community mailboxes.
                        let community_mail_boxes = todo!();
                        let sess_context_write = session_context.write().await;
                        sess_context_write.signed_in_user = Some(user_id.into());
                        sess_context_write.community_mail_boxes = community_mail_boxes;
                        ServerResponse::LoginSuccess(LoginSuccess {
                            user_id: user_id.into(),
                        })
                    } else {
                        ServerResponse::LoginFailed(LoginFailed {})
                    }
                }
                Err(e) => {
                    if let diesel::result::Error::NotFound = e {
                        ServerResponse::LoginFailed(LoginFailed {})
                    } else {
                        ServerResponse::Error(Error {
                            cause: Some("the server encountered an error".to_string()),
                        })
                    }
                }
            }
        }
        ClientEvent::ChangePassword(change_password) => todo!(),
        ClientEvent::SendMessage(message) => todo!(),
        ClientEvent::SendReact(react) => todo!(),
        ClientEvent::CreateChannel(create_channel) => todo!(),
        ClientEvent::CreateCategory(create_category) => todo!(),
        ClientEvent::CreateCommunity(create_community) => todo!(),
        ClientEvent::JoinCommunity(join_community) => todo!(),
        ClientEvent::LeaveCommunity(leave_community) => todo!(),
    };
    Ok(resp)
}
