use uuid::Uuid;

pub struct AttachmentInput {
    mime_type: String,
    file_name: String,
    content: Vec<u8>,
}

pub struct AttachmentData {
    mime_type: String,
    file_name: String,
    attachment_id: Uuid,
}
