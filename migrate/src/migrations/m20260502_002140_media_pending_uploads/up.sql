ALTER TABLE attachment ADD COLUMN ready_at TIMESTAMPTZ;
UPDATE attachment SET ready_at = timestamp;

ALTER TABLE icon ADD COLUMN ready_at TIMESTAMPTZ;
UPDATE icon SET ready_at = timestamp;

CREATE INDEX attachment_pending_idx
    ON attachment (timestamp) WHERE ready_at IS NULL;
CREATE INDEX icon_pending_idx
    ON icon (timestamp) WHERE ready_at IS NULL;
