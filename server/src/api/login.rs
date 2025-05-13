use serde::{Deserialize, Serialize};

use crate::aspen_protocol::UserId;


#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LoginResponse {
    Ok {
        user_id: UserId,
        auth_token: String, 
    },
    InvalidCredentials,
}

#[derive(Deserialize)]
pub struct Login {
    pub username: String,
    pub password: String,
}

pub async fn try_login(l: &Login) -> LoginResponse {

}