-- A preview picture's size in pixels, read from its header when the server stored it, so
-- readers can make room for it before it loads. Both or neither.
ALTER TABLE message_link_preview
    ADD COLUMN image_width INTEGER,
    ADD COLUMN image_height INTEGER,
    ADD CONSTRAINT message_link_preview_image_size CHECK (
        (image_width IS NULL AND image_height IS NULL) OR (image_width > 0 AND image_height > 0)
    );
