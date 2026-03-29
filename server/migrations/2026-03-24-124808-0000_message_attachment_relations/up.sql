-- Your SQL goes here
ALTER TABLE message_attachment ADD CONSTRAINT message_fk FOREIGN KEY (message_id) REFERENCES message(id);
ALTER TABLE message_attachment ADD CONSTRAINT attachment_fk FOREIGN KEY (attachment_id) REFERENCES attachment(id);