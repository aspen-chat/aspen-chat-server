-- Objects already moved under `evidence/` stay there, named by `storage_key`.
DROP INDEX attachment_evidence_unmoved;
DROP INDEX attachment_removed_from;
ALTER TABLE attachment DROP COLUMN removed_from, DROP COLUMN evidence_at;
