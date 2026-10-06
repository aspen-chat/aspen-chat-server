DROP TABLE held_message;
DROP TABLE attachment_preview_job;
ALTER TABLE attachment
    DROP CONSTRAINT attachment_preview_whole,
    DROP COLUMN preview_storage_key,
    DROP COLUMN preview_mime_type,
    DROP COLUMN preview_width,
    DROP COLUMN preview_height;
