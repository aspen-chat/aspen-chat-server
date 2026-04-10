ALTER TABLE icon
ADD COLUMN data BYTEA,
DROP COLUMN storage_key;

ALTER TABLE attachment
DROP COLUMN storage_key;
