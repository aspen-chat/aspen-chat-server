ALTER TABLE attachment
    DROP CONSTRAINT attachment_dimensions,
    DROP COLUMN height,
    DROP COLUMN width;
