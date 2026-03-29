use crate::api::message::message_to_api;
use crate::api::message_enum::command::{
    ChannelCreateCommand, ChannelCreateCommandResponse, ChannelDeleteCommand,
    ChannelDeleteCommandResponse, ChannelReadCommand, ChannelReadCommandResponse,
    ChannelUpdateCommand, ChannelUpdateCommandResponse,
};
use crate::api::message_enum::{Category, Message, Pin};
use crate::api::{GlobalServerContext, message_enum};
use crate::app::{ChannelId, CommunityId, MaybeLoaded, MessageId};
use crate::database::schema::message::channel;
use crate::{api, app};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use futures_util::stream;
use futures_util::{StreamExt, TryStreamExt};
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use tracing::error;
use utoipa::ToSchema;

#[utoipa::path(post, path = "/channel", responses((status = OK, body=ChannelCreateCommandResponse)))]
pub async fn create_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelCreateCommand>,
) -> (StatusCode, Json<ChannelCreateCommandResponse>) {
    match app::channel::create_channel(
        &state,
        command.name,
        command.sort_index,
        command.ty,
        command.community,
        command.parent_category,
    )
    .await
    {
        Ok(c) => (
            StatusCode::OK,
            ChannelCreateCommandResponse::CreateOk(channel_to_api(c)).into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "channel create command error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ChannelCreateCommandResponse::Error {
                    cause: None,
                }
                .into(),
            )
        }
    }
}

pub fn channel_to_api(c: app::channel::Channel) -> message_enum::Channel {
    message_enum::Channel {
        id: c.id,
        parent_category: c.parent_category.as_ref().map(MaybeLoaded::id).cloned(),
        community: c.community.as_ref().map(MaybeLoaded::id).cloned(),
        name: c.name,
        sort_index: c.sort_index,
        ty: c.ty,
    }
}

#[utoipa::path(get, path = "/channel", responses((status = OK, body=ChannelReadCommandResponse)))]
pub async fn read_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelReadCommand>,
) -> (StatusCode, Json<ChannelReadCommandResponse>) {
    match app::channel::read_channel(&state, command.id).await {
        Ok(c) => (
            StatusCode::OK,
            ChannelReadCommandResponse::Channel(channel_to_api(c)).into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error reading channel");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ChannelReadCommandResponse::Error {
                    cause: None,
                }
                .into(),
            )
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessagesReadCommand {
    channel: ChannelId,
    view_description: ChannelViewDescription,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", tag = "adjective")]
pub enum ChannelViewDescription {
    Before {
        message: MessageId,
        count: u32,
    },
    After {
        message: MessageId,
        count: u32,
    },
    Around {
        message: MessageId,
        radius: u32,
    },
    Search {
        // TODO
    },
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ChannelMessagesReadCommandResponse {
    Messages { data: Vec<Message> },
    NotAllowed { reason: Option<String> },
    Error { cause: Option<String> },
}

#[utoipa::path(get, path = "/channel/messages", responses((status = OK, body=ChannelMessagesReadCommandResponse)))]
pub async fn read_channel_messages(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelMessagesReadCommand>,
) -> (StatusCode, Json<ChannelMessagesReadCommandResponse>) {
    match app::channel::read_channel_messages(&state, command.channel, command.view_description)
        .await
    {
        Ok(messages) => (
            StatusCode::OK,
            ChannelMessagesReadCommandResponse::Messages {
                data: messages
                    .into_iter()
                    .map(|m| message_to_api(m.message, m.attachments))
                    .collect(),
            }
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error reading channel messages");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ChannelMessagesReadCommandResponse::Error {
                    cause: None,
                }
                .into(),
            )
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPinsReadCommand {
    channel: ChannelId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ChannelPinsReadCommandResponse {
    Pins { data: Vec<Pin> },
    NotAllowed { reason: Option<String> },
    Error { cause: Option<String> },
}

#[utoipa::path(get, path = "/channel/pins", responses((status = OK, body=ChannelPinsReadCommandResponse)))]
pub async fn read_channel_pins(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelPinsReadCommand>,
) -> (StatusCode, Json<ChannelPinsReadCommandResponse>) {
    todo!()
}

#[utoipa::path(patch, path = "/channel", responses((status = OK, body=ChannelUpdateCommandResponse)))]
pub async fn update_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelUpdateCommand>,
) -> (StatusCode, Json<ChannelUpdateCommandResponse>) {
    todo!()
}

#[utoipa::path(delete, path = "/channel", responses((status = OK, body=ChannelDeleteCommandResponse)))]
pub async fn delete_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelDeleteCommand>,
) -> (StatusCode, Json<ChannelDeleteCommandResponse>) {
    todo!()
}
