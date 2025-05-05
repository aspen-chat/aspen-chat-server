use serde::{Deserialize, Serialize};

use super::{
    Attachment, CategoryId, ChannelId, ChannelPermissions, ChannelType, CommunityId, MessageId,
    UserId,
};

/// Describes events that the client notifies the server of.
#[derive(Deserialize)]
#[serde(tag = "clientEvent")]
pub enum ClientEvent {
    Login(Login),
    RegisterUser(RegisterUser),
    ChangePassword(ChangePassword),
    SendMessage(Message),
    SendReact(React),
    CreateChannel(CreateChannel),
    CreateCategory(CreateCategory),
    CreateCommunity(CreateCommunity),
    JoinCommunity(JoinCommunity),
    LeaveCommunity(LeaveCommunity),
}

#[derive(Deserialize)]
pub struct Login {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct RegisterUser {
    pub username: String,
    pub password: String,
    pub invite_code: String,
}

#[derive(Deserialize)]
pub struct ChangePassword {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Deserialize)]
pub struct Message {
    pub channel_id: ChannelId,
    pub content: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Deserialize)]
pub struct React {
    pub message_id: MessageId,
    pub emoji: String,
}

#[derive(Deserialize)]
pub struct CreateChannel {
    pub community: CommunityId,
    pub parent_category: Option<CategoryId>,
    pub name: String,
    pub permissions: ChannelPermissions,
    pub ty: ChannelType,
}

#[derive(Deserialize)]
pub struct CreateCategory {
    pub community: CommunityId,
}

#[derive(Deserialize)]
pub struct CreateCommunity {
    pub name: String,
}

#[derive(Deserialize)]
pub struct JoinCommunity {
    pub community_name: String,
    pub invite_code: String,
}

#[derive(Deserialize)]
pub struct LeaveCommunity {
    pub community: CommunityId,
}

/// Possible responses from the server
#[derive(Serialize)]
#[serde(tag = "serverResponse")]
pub enum ServerResponse {
    CreateOk(CreateOk),
    LoginSuccess(LoginSuccess),
    LoginFailed(LoginFailed),
    NotAllowed(NotAllowed),
    Error(Error),
}

#[derive(Serialize)]
pub struct CreateOk {
    pub new_id: Option<uuid::Uuid>,
}

#[derive(Serialize)]
pub struct LoginSuccess {
    pub user_id: UserId,
}

#[derive(Serialize)]
pub struct LoginFailed {}

#[derive(Serialize)]
pub struct NotAllowed {
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct Error {
    pub cause: Option<String>,
}
