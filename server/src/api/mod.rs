use axum::{
    Json,
    http::StatusCode,
    routing::{get, post},
};

mod command_response;
mod event_stream;
mod login;
mod message_enum;

use command_response::CommandResponse;
use login::{Login, LoginResponse};
use message_enum::command::{
    CategoryCommand, ChannelCommand, CommunityCommand, MessageCommand, ReactCommand, UserCommand,
};

pub(crate) fn make_router() -> axum::Router {
    axum::Router::new()
        .route("/login", post(login))
        .route("/user", post(user))
        .route("/message", post(message))
        .route("/react", post(react))
        .route("/channel", post(channel))
        .route("/category", post(category))
        .route("/community", post(community))
        .route("/event_stream", get(event_stream::event_stream))
}

async fn login(Json(login): Json<Login>) -> (StatusCode, Json<LoginResponse>) {
    let resp = login::try_login(&login).await;
    if matches!(resp, LoginResponse::InvalidCredentials) {
        (StatusCode::UNAUTHORIZED, resp.into())
    } else {
        (StatusCode::OK, resp.into())
    }
}

async fn user(Json(command): Json<UserCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        UserCommand::Create { name } => todo!(),
        UserCommand::Read { id } => todo!(),
        UserCommand::Update { id, name } => todo!(),
        UserCommand::Delete { id } => todo!(),
    }
}

async fn message(Json(command): Json<MessageCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        MessageCommand::Create {
            channel_id,
            content,
            attachments,
        } => todo!(),
        MessageCommand::Read { id } => todo!(),
        MessageCommand::Update {
            id,
            content,
            attachments,
        } => todo!(),
        MessageCommand::Delete { id } => todo!(),
    }
}

async fn react(Json(command): Json<ReactCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        ReactCommand::Create { message_id, emoji } => todo!(),
        ReactCommand::Delete {
            message_id,
            emoji,
            user_id,
        } => todo!(),
    }
}

async fn channel(Json(command): Json<ChannelCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        ChannelCommand::Create {
            parent_category,
            name,
            permissions,
            ty,
        } => todo!(),
        ChannelCommand::Read { id } => todo!(),
        ChannelCommand::Update {
            id,
            parent_category,
            name,
            permissions,
        } => todo!(),
        ChannelCommand::Delete { id } => todo!(),
    }
}

async fn category(Json(command): Json<CategoryCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        CategoryCommand::Create { community, name } => todo!(),
        CategoryCommand::Read { id } => todo!(),
        CategoryCommand::Update { id, name } => todo!(),
        CategoryCommand::Delete { id } => todo!(),
    }
}

async fn community(Json(command): Json<CommunityCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        CommunityCommand::Create { name } => todo!(),
        CommunityCommand::Read { id } => todo!(),
        CommunityCommand::Update { id, name } => todo!(),
        CommunityCommand::Delete { id } => todo!(),
    }
}
