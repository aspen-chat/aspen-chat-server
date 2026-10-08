DROP INDEX message_attachment_attachment_idx;
DROP INDEX attachment_unsent_idx;
ALTER TABLE attachment DROP COLUMN sent;
