-- The previews each account's uploads have waiting, by kind, which `preview::queue` counts
-- against `preview::MAX_PENDING_PER_ACCOUNT`.
CREATE INDEX job_preview_account ON job (kind, (payload->>'account'))
    WHERE kind IN ('makePicturePreview', 'makeVideoPoster') AND failed_at IS NULL;
