use crate::{api::message_enum::command::UserSubCommand, database::schema};
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use chrono::Utc;
use diesel_async::{
    AsyncPgConnection, RunQueryDsl,
    pooled_connection::{
        AsyncDieselConnectionManager,
        deadpool::Pool,
    },
};
use futures_util::{StreamExt, TryFutureExt};
use tracing::error;

mod event_stream;
mod login;
mod message_enum;

use crate::api::message_enum::server_event::ServerEvent;
use dashmap::DashMap;
use diesel::{BoolExpressionMethods, ExpressionMethods as _, QueryDsl, result::DatabaseErrorKind};
use login::{
    ChangePassword, ChangePasswordResponse, Login, LoginResponse, Logout, LogoutResponse,
    OtherServerAuth, OtherServerAuthResponse, TokenRefresh, TokenRefreshResponse, hash_password,
};
use message_enum::command::{
    CategoryCommand, CategoryCommandResponse, CategorySubCommand, ChannelCommand,
    ChannelCommandResponse, ChannelSubCommand, CommunityCommand, CommunityCommandResponse,
    CommunitySubCommand, IconCommand, IconCommandResponse, IconSubCommand, MessageCommand,
    MessageCommandResponse, MessageSubCommand, ReactCommand, ReactCommandResponse, ReactSubCommand,
    UserCommand, UserCommandResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

pub(crate) fn make_router() -> axum::Router {
    axum::Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/token_refresh", post(token_refresh))
        .route("/change_password", post(change_password))
        .route("/other_server_login", post(other_server_login))
        .route("/user", post(user))
        .route("/message", post(message))
        .route("/react", post(react))
        .route("/channel", post(channel))
        .route("/category", post(category))
        .route("/community", post(community))
        .route("/icon", post(icon))
        .route("/event_stream", get(event_stream::event_stream))
        .with_state(GlobalServerContext::new())
}

async fn login(
    State(state): State<GlobalServerContext>,
    Json(login): Json<Login>,
) -> (StatusCode, Json<LoginResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn.and_then(|conn| login::try_login(conn, &login)).await {
        Ok(resp) => resp,
        Err(e) => {
            error!("error during login {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                LoginResponse::ServerError.into(),
            );
        }
    };
    let status_code = match &resp {
        LoginResponse::Ok { .. } => StatusCode::OK,
        LoginResponse::InvalidCredentials => StatusCode::UNAUTHORIZED,
        LoginResponse::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

async fn logout(
    State(state): State<GlobalServerContext>,
    Json(logout): Json<Logout>,
) -> (StatusCode, Json<LogoutResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn.and_then(|conn| login::try_logout(conn, &logout))
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("error during logout {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                LogoutResponse::ServerError.into(),
            );
        }
    };
    let status_code = match &resp {
        LogoutResponse::Ok => StatusCode::OK,
        LogoutResponse::InvalidToken => StatusCode::UNAUTHORIZED,
        LogoutResponse::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

async fn token_refresh(
    State(state): State<GlobalServerContext>,
    Json(token_refresh): Json<TokenRefresh>,
) -> (StatusCode, Json<TokenRefreshResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn.and_then(|conn| login::try_token_refresh(conn, &token_refresh))
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("error during token refresh {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                TokenRefreshResponse::ServerError.into(),
            );
        }
    };
    let status_code = match &resp {
        login::TokenRefreshResponse::Ok { .. } => StatusCode::OK,
        login::TokenRefreshResponse::InvalidToken => StatusCode::UNAUTHORIZED,
        login::TokenRefreshResponse::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

async fn change_password(
    State(state): State<GlobalServerContext>,
    Json(change_password): Json<ChangePassword>,
) -> (StatusCode, Json<ChangePasswordResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn.and_then(|conn| login::try_change_password(conn, &change_password))
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("error during change password {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ChangePasswordResponse::ServerError.into(),
            );
        }
    };
    let status_code = match &resp {
        login::ChangePasswordResponse::Ok { .. } => StatusCode::OK,
        login::ChangePasswordResponse::OldPasswordIncorrect => StatusCode::UNAUTHORIZED,
        login::ChangePasswordResponse::NewPasswordDoesntMeetRequirements { .. } => {
            StatusCode::BAD_REQUEST
        }
        login::ChangePasswordResponse::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

async fn other_server_login(
    State(state): State<GlobalServerContext>,
    Json(other_server_auth): Json<OtherServerAuth>,
) -> (StatusCode, Json<OtherServerAuthResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn.and_then(|conn| login::try_other_server_auth(conn, &other_server_auth))
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("error during other_server_auth_token {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                OtherServerAuthResponse::Error.into(),
            );
        }
    };
    let status_code = match &resp {
        OtherServerAuthResponse::Ok { .. } => StatusCode::OK,
        OtherServerAuthResponse::InvalidToken => StatusCode::BAD_REQUEST,
        OtherServerAuthResponse::Error => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

async fn user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserCommand>,
) -> (StatusCode, Json<UserCommandResponse>) {
    let mut conn = match state.connection_pool.get().await {
        Ok(conn) => conn,
        Err(e) => {
            error!("unable to get a database connection from pool {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                UserCommandResponse::Error { cause: None }.into(),
            );
        }
    };
    match command.subcommand {
        UserSubCommand::Create {
            name,
            icon,
            password,
        } => {
            let password_hash_result = hash_password(&password);
            let password_hash = match password_hash_result {
                Ok(s) => s,
                Err(e) => {
                    error!("error generating password hash at user creation {e}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        UserCommandResponse::Error { cause: None }.into(),
                    );
                }
            };
            let new_user_id = UserId::new();
            let r = diesel::insert_into(schema::user::table)
                .values((
                    schema::user::columns::id.eq(new_user_id.0),
                    schema::user::columns::password_hash.eq(password_hash),
                    schema::user::columns::name.eq(name),
                ))
                .execute(&mut conn)
                .await;
            match r {
                Ok(_) => (
                    StatusCode::OK,
                    UserCommandResponse::CreateOk { id: new_user_id }.into(),
                ),
                Err(e) => {
                    if let diesel::result::Error::DatabaseError(
                        DatabaseErrorKind::UniqueViolation,
                        e,
                    ) = e
                    {
                        (
                            StatusCode::BAD_REQUEST,
                            UserCommandResponse::Error {
                                cause: Some("usernameAlreadyTaken".to_string()),
                            }
                            .into(),
                        )
                    } else {
                        error!("error inserting new user into database {e}");
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            UserCommandResponse::Error { cause: None }.into(),
                        )
                    }
                }
            }
        }
        UserSubCommand::Read { id } => todo!(),
        UserSubCommand::Update { id, name, icon } => todo!(),
        UserSubCommand::Delete { id } => todo!(),
    }
}

async fn message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageCommand>,
) -> (StatusCode, Json<MessageCommandResponse>) {
    match command.subcommand {
        MessageSubCommand::Create {
            channel_id,
            content,
            attachments,
        } => todo!(),
        MessageSubCommand::Read { id } => todo!(),
        MessageSubCommand::Update {
            id,
            content,
            attachments,
        } => todo!(),
        MessageSubCommand::Delete { id } => todo!(),
    }
}

async fn react(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ReactCommand>,
) -> (StatusCode, Json<ReactCommandResponse>) {
    match command.subcommand {
        ReactSubCommand::Create { message_id, emoji } => todo!(),
        ReactSubCommand::Delete {
            message_id,
            emoji,
            user_id,
        } => todo!(),
    }
}

async fn channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelCommand>,
) -> (StatusCode, Json<ChannelCommandResponse>) {
    match command.subcommand {
        ChannelSubCommand::Create {
            parent_category,
            name,
            permissions,
            ty,
        } => todo!(),
        ChannelSubCommand::Read { id } => todo!(),
        ChannelSubCommand::Update {
            id,
            parent_category,
            name,
            permissions,
        } => todo!(),
        ChannelSubCommand::Delete { id } => todo!(),
    }
}

async fn category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryCommand>,
) -> (StatusCode, Json<CategoryCommandResponse>) {
    match command.subcommand {
        CategorySubCommand::Create { community, name } => todo!(),
        CategorySubCommand::Read { id } => todo!(),
        CategorySubCommand::Update { id, name } => todo!(),
        CategorySubCommand::Delete { id } => todo!(),
    }
}

async fn community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityCommand>,
) -> (StatusCode, Json<CommunityCommandResponse>) {
    match command.subcommand {
        CommunitySubCommand::Create { name, icon } => todo!(),
        CommunitySubCommand::Read { id } => todo!(),
        CommunitySubCommand::Update { id, name, icon } => todo!(),
        CommunitySubCommand::Delete { id } => todo!(),
    }
}

async fn icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconCommand>,
) -> (StatusCode, Json<IconCommandResponse>) {
    match command.subcommand {
        IconSubCommand::Create { data, mime_type } => todo!(),
        IconSubCommand::Read { id } => todo!(),
        IconSubCommand::Delete { id } => todo!(),
    }
}

macro_rules! id_type {
    ($type_name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Copy, Deserialize, Serialize, Hash)]
        #[serde(transparent)]
        pub struct $type_name(uuid::Uuid);

        impl $type_name {
            pub fn new() -> Self {
                Self(uuid::Uuid::now_v7())
            }
        }

        impl From<uuid::Uuid> for $type_name {
            fn from(value: uuid::Uuid) -> Self {
                Self(value)
            }
        }
    };
}

id_type!(CommunityId);

id_type!(UserId);

id_type!(ChannelId);

id_type!(MessageId);

id_type!(CategoryId);

id_type!(AttachmentId);

id_type!(IconId);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Attachment {
    mime_type: String,
    file_name: String,
    content: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AttachmentMeta {
    attachment_id: AttachmentId,
    mime_type: String,
    file_name: String,
    preview: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum ChannelType {
    Text,
    Voice,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelPermissions {
    // TODO
}

mod timestamp_serde {
    use chrono::{DateTime, Utc};
    use serde::Serializer;

    pub fn serialize<S>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_i64(t.timestamp_micros())
    }
}

pub async fn authenticated_user(
    mut conn: impl AsMut<AsyncPgConnection>,
    session_token: String,
) -> Result<Option<UserId>, anyhow::Error> {
    use schema::{refresh_token, session};

    let now = Utc::now().naive_utc();
    let maybe_user_id = session::table
        .inner_join(refresh_token::table)
        .select(refresh_token::dsl::user)
        .filter(
            session::token
                .eq(session_token)
                .and(session::expires.ge(now))
                .and(refresh_token::expires.ge(now)),
        )
        .limit(1)
        .load_stream::<uuid::Uuid>(conn.as_mut())
        .await?
        .next()
        .await
        .transpose()?
        .map(UserId::from);
    Ok(maybe_user_id)
}

#[derive(Clone)]
pub struct GlobalServerContext {
    pub connection_pool: Pool<AsyncPgConnection>,
}

impl GlobalServerContext {
    pub fn new() -> Self {
        Self {
            connection_pool: {
                let conn_manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(
                    &std::env::var("DATABASE_URL")
                        .expect("DATABASE_URL must be set in environment or .env file"),
                );
                Pool::builder(conn_manager)
                    .build()
                    .expect("Failed to init database connection pool")
            },
        }
    }
}

/// If a client misses this many messages at once it will be forcefully disconnected.
const MAILBOX_SIZE: usize = 512;

#[derive(Clone)]
pub struct CommunityMailboxManager {
    map: Arc<DashMap<CommunityId, broadcast::Sender<Arc<ServerEvent>>>>,
}

impl CommunityMailboxManager {
    pub fn new() -> Self {
        Self {
            map: Arc::new(DashMap::default()),
        }
    }

    pub fn subscribe_mailbox(&self, id: &CommunityId) -> broadcast::Receiver<Arc<ServerEvent>> {
        // First try to obtain in a read-only manner.
        let first_attempt = self.map.get(id);
        if let Some(s) = first_attempt {
            return s.subscribe();
        }
        // Someone else could have beat us to it, check again to see if it's initialized.
        let write_lock = self.map.entry(*id);
        match write_lock {
            dashmap::Entry::Occupied(occupied_entry) => occupied_entry.get().subscribe(),
            dashmap::Entry::Vacant(vacant_entry) => {
                let (sender, receiver) = broadcast::channel(MAILBOX_SIZE);
                vacant_entry.insert(sender);
                receiver
            }
        }
    }
}

pub struct SessionContext {
    pub signed_in_user: Option<UserId>,
    pub community_mailbox_subscribe_commands: mpsc::Sender<Vec<SubscribeCommand>>,
}

impl SessionContext {
    pub fn new(cm_subscribe_commands: mpsc::Sender<Vec<SubscribeCommand>>) -> Self {
        Self {
            signed_in_user: None,
            community_mailbox_subscribe_commands: cm_subscribe_commands,
        }
    }
}

pub struct SubscribeCommand {
    pub community: CommunityId,
    /// True means a subscription should be made. False means it should be
    /// unsubscribed.
    pub desire_subscribed: bool,
}
