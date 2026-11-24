DELETE FROM job WHERE kind IN ('queueSvgPreviews', 'makeSvgPreview');
DROP INDEX job_preview_account;
CREATE INDEX job_preview_account ON job (kind, (payload->>'account'))
    WHERE kind IN ('makePicturePreview', 'makeVideoPoster') AND failed_at IS NULL;
