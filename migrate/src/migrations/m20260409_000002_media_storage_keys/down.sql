-- `icon.data` was declared `BYTEA NOT NULL` in the original `icon`
-- table (see m20260216_052527_uncaptured up.sql); restore that exact
-- nullability on rollback so the reverted schema matches what the up
-- chain produced before this migration ran. Safe on empty tables; if
-- this migration is ever rolled back on a populated database the
-- `NOT NULL` will need an intermediate data backfill step.
ALTER TABLE icon
ADD COLUMN data BYTEA NOT NULL,
DROP COLUMN storage_key;

ALTER TABLE attachment
DROP COLUMN storage_key;
