CREATE TABLE message_link_preview (
    message_id UUID NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    url TEXT NOT NULL,
    title TEXT,
    description TEXT,
    site_name TEXT,
    image_id UUID,
    image_mime_type TEXT,
    theme_color TEXT,
    PRIMARY KEY (message_id, position),
    CHECK ((image_id IS NULL) = (image_mime_type IS NULL))
);

CREATE INDEX message_link_preview_message_id_idx
    ON message_link_preview (message_id);

CREATE UNIQUE INDEX message_link_preview_image_id_idx
    ON message_link_preview (image_id) WHERE image_id IS NOT NULL;
