DELETE FROM job WHERE kind = 'purgeEvidence';
DROP INDEX report_case_message;
DROP INDEX message_link_preview_kept;
ALTER TABLE message_link_preview DROP COLUMN message_deleted_at;
DROP INDEX attachment_evidence_at;
ALTER TABLE deployment_settings DROP COLUMN evidence_retention_days;
