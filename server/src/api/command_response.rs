use serde::Serialize;

#[derive(Serialize)]
#[serde(tag = "responseType")]
pub enum CommandResponse {
    CreateOk(CreateOk),
    CommandSuccess,
    NotAllowed(NotAllowed),
    Error(Error),
}

#[derive(Serialize)]
pub struct CreateOk {
    pub new_id: Option<uuid::Uuid>,
}

#[derive(Serialize)]
pub struct NotAllowed {
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct Error {
    pub cause: Option<String>,
}