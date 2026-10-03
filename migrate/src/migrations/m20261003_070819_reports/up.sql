-- A deployment-wide ban reaches this deployment's own users as well as other deployments', with
-- a reason they are told when they try to sign in and an end, after which it no longer stands.
ALTER TABLE "user"
    DROP CONSTRAINT user_ban_is_foreign,
    ADD COLUMN ban_reason TEXT,
    ADD COLUMN banned_until TIMESTAMPTZ,
    ADD CONSTRAINT user_ban_details_need_ban
        CHECK (banned_at IS NOT NULL OR (ban_reason IS NULL AND banned_until IS NULL));
CREATE INDEX user_banned ON "user" (banned_at) WHERE banned_at IS NOT NULL;

UPDATE moderation_log SET action = 'banUser' WHERE action = 'banForeignUser';
UPDATE moderation_log SET action = 'liftUserBan' WHERE action = 'liftForeignUserBan';

-- Ban users (1 << 9, 512) goes to every role that holds Moderate any community (1 << 4, 16),
-- which banned users of other deployments until now; Manage report categories (1 << 8, 256) to
-- every role holding the first four administrative permissions (15), as the terminal's
-- Administrator role does. Review reports (1 << 7) and Message any user (1 << 10) are given
-- deliberately.
UPDATE deployment_role SET permissions = permissions | 512 WHERE permissions & 16 = 16;
UPDATE deployment_role SET permissions = permissions | 256 WHERE permissions & 15 = 15;

-- A moderator's warning, sent in a DM to the person warned (`warning` names what about).
ALTER TYPE message_kind ADD VALUE 'warning';

-- The messages of this deployment a message links to, in the order its text names them, and,
-- for a warning, what it warns about.
ALTER TABLE message
    ADD COLUMN linked_messages UUID[] NOT NULL DEFAULT '{}',
    ADD COLUMN warning JSONB;

-- What a report says is wrong: the built-in categories, named by `builtin`, and the ones the
-- deployment added, named by `name`. Hidden ones are no longer offered, and reports made with
-- them keep them.
CREATE TABLE report_category (
    id UUID PRIMARY KEY,
    builtin TEXT UNIQUE,
    name TEXT,
    description TEXT,
    position INTEGER NOT NULL DEFAULT 0,
    hidden BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT report_category_named CHECK ((builtin IS NULL) <> (name IS NULL)),
    CONSTRAINT report_category_other_shown CHECK (builtin IS DISTINCT FROM 'other' OR NOT hidden)
);
INSERT INTO report_category (id, builtin)
SELECT gen_random_uuid(), builtin
FROM unnest(ARRAY[
    'spam', 'harassment', 'hateSpeech', 'violence', 'selfHarm', 'illegalContent',
    'impersonation', 'other'
]) AS builtin;

-- Reports of one message, or of one person's profile, gathered for review. At most one case of
-- each is unresolved (open or dismissed) at a time; a report of something whose case was
-- dismissed reopens it, and one of something whose case was resolved opens a new one.
CREATE TABLE report_case (
    id UUID PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('message', 'profile')),
    subject UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    message UUID REFERENCES message (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'resolved', 'dismissed')),
    opened_at TIMESTAMPTZ NOT NULL,
    last_reported_at TIMESTAMPTZ NOT NULL,
    closed_at TIMESTAMPTZ,
    closed_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    resolution JSONB,
    CONSTRAINT report_case_message_kind CHECK ((kind = 'message') = (message IS NOT NULL))
);
CREATE UNIQUE INDEX report_case_unresolved_message ON report_case (message)
    WHERE status <> 'resolved' AND kind = 'message';
CREATE UNIQUE INDEX report_case_unresolved_profile ON report_case (subject)
    WHERE status <> 'resolved' AND kind = 'profile';
CREATE INDEX report_case_by_status ON report_case (status, last_reported_at DESC, id DESC);

-- One person's report in a case, which they make once. A profile report names the aspects it
-- finds objectionable and holds the profile as it stood.
CREATE TABLE report (
    id UUID PRIMARY KEY,
    "case" UUID NOT NULL REFERENCES report_case (id) ON DELETE CASCADE,
    reporter UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    category UUID NOT NULL REFERENCES report_category (id),
    explanation TEXT,
    aspects TEXT[] NOT NULL DEFAULT '{}',
    profile JSONB,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE ("case", reporter)
);
CREATE INDEX report_by_reporter ON report (reporter, created_at);
