ALTER TABLE message_link_preview
    DROP CONSTRAINT message_link_preview_image_size,
    DROP COLUMN image_height,
    DROP COLUMN image_width;
