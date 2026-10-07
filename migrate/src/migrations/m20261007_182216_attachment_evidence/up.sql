-- The files of deleted messages, and attachments taken off their messages, are kept as evidence
-- for reviewing reports, out of public reach (`app::attachment::evidence`). `evidence_at` is when
-- an attachment became evidence; its objects are moved under `evidence/` soon after, which its
-- `storage_key` then names. `removed_from` is the message it was taken off, so a case about that
-- message can still show it.
ALTER TABLE attachment
    ADD COLUMN evidence_at TIMESTAMPTZ,
    ADD COLUMN removed_from UUID REFERENCES message (id) ON DELETE SET NULL;
CREATE INDEX attachment_removed_from ON attachment (removed_from) WHERE removed_from IS NOT NULL;
-- What is waiting to be moved under `evidence/`.
CREATE INDEX attachment_evidence_unmoved ON attachment (evidence_at)
    WHERE evidence_at IS NOT NULL AND storage_key NOT LIKE 'evidence/%';
