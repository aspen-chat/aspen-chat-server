DROP INDEX IF EXISTS attachment_pending_idx;
DROP INDEX IF EXISTS icon_pending_idx;

ALTER TABLE attachment DROP COLUMN ready_at;
ALTER TABLE icon DROP COLUMN ready_at;
