-- How many GiB one person may upload in any 24 hours (`app::upload_quota`); 0 sets no limit.
ALTER TABLE deployment_settings
    ADD COLUMN upload_quota_gib INTEGER NOT NULL DEFAULT 25 CHECK (upload_quota_gib >= 0);

-- Each upload started (attachments and icons), with the bytes it was signed for, kept for a day
-- so the quota counts a rolling day whatever becomes of the upload.
CREATE TABLE upload_usage (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    bytes BIGINT NOT NULL CHECK (bytes >= 0),
    at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX upload_usage_user_at_idx ON upload_usage (user_id, at);
CREATE INDEX upload_usage_at_idx ON upload_usage (at);
