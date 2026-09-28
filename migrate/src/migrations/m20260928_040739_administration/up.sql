-- Who may open the Administration Dashboard. Granted and revoked only from the terminal
-- (`aspen-chat-server admin`), never over the API.
ALTER TABLE "user" ADD COLUMN admin BOOLEAN NOT NULL DEFAULT false;

-- Invites to create an account, required when `[registration] invite_required` is set. Made
-- from the dashboard, or from the terminal (`created_by` NULL) for a deployment's first account.
CREATE TABLE registration_invite (
    code TEXT PRIMARY KEY,
    created_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ,
    max_uses INTEGER NOT NULL CHECK (max_uses > 0),
    uses INTEGER NOT NULL DEFAULT 0 CHECK (uses >= 0),
    revoked_at TIMESTAMPTZ,
    note TEXT
);

-- Which invite each account was created with, when one was.
ALTER TABLE "user"
    ADD COLUMN registered_with TEXT REFERENCES registration_invite (code) ON DELETE SET NULL;
CREATE INDEX user_registered_with ON "user" (registered_with) WHERE registered_with IS NOT NULL;

-- The dashboard searches users and communities by any part of their names.
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE INDEX user_name_search ON "user" USING gin (lower(name) gin_trgm_ops);
CREATE INDEX user_display_name_search ON "user" USING gin (lower(display_name) gin_trgm_ops);
CREATE INDEX community_name_search ON community USING gin (lower(name) gin_trgm_ops);
