-- Whether readers' apps hide the attachment behind a spoiler until it is revealed, as its
-- uploader chose before sending it (`app::attachment`).
ALTER TABLE attachment ADD COLUMN spoiler BOOLEAN NOT NULL DEFAULT false;
