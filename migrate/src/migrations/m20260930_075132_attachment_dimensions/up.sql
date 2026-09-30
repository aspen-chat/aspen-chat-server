-- A picture's size in pixels, as its uploader measured it, so readers can make room for it
-- before it loads. Both or neither: files that are not pictures, and pictures uploaded by
-- clients that do not measure, have none.
ALTER TABLE attachment
    ADD COLUMN width INTEGER,
    ADD COLUMN height INTEGER,
    ADD CONSTRAINT attachment_dimensions CHECK (
        (width IS NULL AND height IS NULL) OR (width > 0 AND height > 0)
    );
