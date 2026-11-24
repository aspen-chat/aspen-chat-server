-- SVGs' previews are made by jobs of their own (`makeSvgPreview`), counted against each
-- account's waiting previews (`preview::MAX_PENDING_PER_ACCOUNT`) as the other kinds are.
DROP INDEX job_preview_account;
CREATE INDEX job_preview_account ON job (kind, (payload->>'account'))
    WHERE kind IN ('makePicturePreview', 'makeVideoPoster', 'makeSvgPreview') AND failed_at IS NULL;

-- One `queueSvgPreviews` job (`preview::queue_svgs_step`) queues the previews of the SVGs
-- uploaded before SVGs had them, in bounded steps.
INSERT INTO job (id, kind, key, class, due, not_before, payload)
VALUES (gen_random_uuid(), 'queueSvgPreviews', NULL, 4, now(), now(), '{}');
