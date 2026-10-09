-- How many days the files of deleted messages are kept for reviewing reports, after which the
-- recurring job `purgeEvidence` deletes them, unless a report case about their message is open
-- or closed within as long; 0 keeps them for good.
ALTER TABLE deployment_settings
    ADD COLUMN evidence_retention_days INTEGER NOT NULL DEFAULT 365
        CHECK (evidence_retention_days >= 0);

-- Evidence by when it became evidence, which the purge walks oldest first.
CREATE INDEX attachment_evidence_at ON attachment (evidence_at) WHERE evidence_at IS NOT NULL;

-- When a link preview's message was deleted, written as it is (`message::soft_delete_many`):
-- its picture, which stays on the public read path, is deleted after the same retention.
ALTER TABLE message_link_preview ADD COLUMN message_deleted_at TIMESTAMPTZ;
UPDATE message_link_preview p SET message_deleted_at = m.deleted_at
FROM message m WHERE m.id = p.message_id AND m.deleted_at IS NOT NULL;
CREATE INDEX message_link_preview_kept ON message_link_preview (message_deleted_at)
    WHERE message_deleted_at IS NOT NULL AND image_id IS NOT NULL;

-- Cases by the message they are about, whatever their status, for whether one holds its
-- evidence.
CREATE INDEX report_case_message ON report_case (message) WHERE message IS NOT NULL;
