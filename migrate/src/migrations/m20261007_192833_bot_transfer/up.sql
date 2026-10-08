-- A bot its owner offers to hand to someone else: it changes hands only when the recipient
-- accepts, before `expires_at`, while the one who offered it still owns it. A bot has at most
-- one offer at a time; offering it again replaces it.
CREATE TABLE bot_transfer (
    bot UUID PRIMARY KEY REFERENCES "user" (id) ON DELETE CASCADE,
    from_owner UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    to_user UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT bot_transfer_to_someone_else CHECK (from_owner <> to_user)
);

CREATE INDEX bot_transfer_to_user ON bot_transfer (to_user);
CREATE INDEX bot_transfer_from_owner ON bot_transfer (from_owner);
