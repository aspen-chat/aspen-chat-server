CREATE TABLE message_attachment(
    message_id UUID NOT NULL,
    attachment_id UUID NOT NULL,
    PRIMARY KEY (message_id, attachment_id)
);