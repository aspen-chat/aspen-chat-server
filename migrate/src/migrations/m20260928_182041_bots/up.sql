-- Bots are users that sign in only with a token. `bot_owner` is who made the bot, and who
-- manages it; the server clears it when they delete their account, leaving the bot working
-- but ownerless, for a holder of Manage bots to delete. A public bot may be added to a
-- community by anyone allowed to add bots there; a private one only by its owner.
ALTER TABLE "user"
    ADD COLUMN bot BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN bot_owner UUID REFERENCES "user" (id) ON DELETE SET NULL,
    ADD COLUMN bot_public BOOLEAN NOT NULL DEFAULT false,
    ADD CONSTRAINT user_bot_owner_is_bot CHECK (bot OR bot_owner IS NULL),
    ADD CONSTRAINT user_bot_public_is_bot CHECK (bot OR NOT bot_public);

CREATE INDEX user_bot_owner ON "user" (bot_owner) WHERE bot_owner IS NOT NULL;

-- Each bot's one token, kept only as its SHA-256 digest, so a leaked table signs no one in.
CREATE TABLE bot_token (
    bot UUID PRIMARY KEY REFERENCES "user" (id) ON DELETE CASCADE,
    digest BYTEA NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The role a bot was given the permissions it was added with; it is the bot's alone, and goes
-- when the bot leaves.
ALTER TABLE community_role
    ADD COLUMN bot UUID REFERENCES "user" (id) ON DELETE CASCADE,
    ADD CONSTRAINT community_role_one_per_bot UNIQUE (community, bot);

-- Add bots (2048) goes to every role that may remove members, which the Moderator and Admin
-- templates give.
UPDATE community_role SET permissions = permissions | 2048 WHERE permissions & 128 = 128;

-- Manage bots (32) is part of administration: every role that holds all of it so far (view
-- the dashboard, registration invites, voice servers, and deployment roles) gains it.
UPDATE deployment_role SET permissions = permissions | 32 WHERE permissions & 15 = 15;
