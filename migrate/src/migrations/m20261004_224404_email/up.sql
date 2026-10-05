-- Email (`app::email`): the deployment's policies for it, each account's address and what it
-- receives there, the mail waiting to be sent, and the newsletter's posts.

ALTER TABLE deployment_settings
    ADD COLUMN email_required BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN email_verification_required BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN newsletter_enabled BOOLEAN NOT NULL DEFAULT false;

-- The address shown on the account's profile: set exactly while the account has chosen to show
-- its address and that address is verified, so every read of a user carries it without a join.
ALTER TABLE "user" ADD COLUMN public_email TEXT;

CREATE TABLE user_email (
    "user" UUID PRIMARY KEY REFERENCES "user" (id) ON DELETE CASCADE,
    address TEXT NOT NULL CHECK (length(address) BETWEEN 3 AND 254),
    verified_at TIMESTAMPTZ,
    shown BOOLEAN NOT NULL DEFAULT false,
    newsletter BOOLEAN NOT NULL DEFAULT false,
    digest BOOLEAN NOT NULL DEFAULT false,
    -- An IANA time zone name, and the hour of the day there the digest is sent.
    digest_time_zone TEXT NOT NULL DEFAULT 'UTC',
    digest_hour SMALLINT NOT NULL DEFAULT 8 CHECK (digest_hour BETWEEN 0 AND 23),
    -- When the next digest is due; NULL while the digest is off.
    digest_next_at TIMESTAMPTZ,
    -- Messages posted after this are new to the next digest.
    digest_since TIMESTAMPTZ,
    -- The language mail to this address is written in.
    locale TEXT NOT NULL DEFAULT 'en',
    -- The secret the unsubscribe links in its mail carry.
    unsubscribe_token TEXT NOT NULL UNIQUE
);

CREATE INDEX user_email_digest_due ON user_email (digest_next_at) WHERE digest;
CREATE INDEX user_email_newsletter ON user_email ("user")
    WHERE newsletter AND verified_at IS NOT NULL;

CREATE TABLE email_outbox (
    id UUID PRIMARY KEY,
    -- Higher is sent first: someone resetting their password is waiting for it.
    priority SMALLINT NOT NULL,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    -- The address it goes to; NULL for the account's verified address when it is sent.
    address TEXT,
    mail JSONB NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    -- Not sent before this: after a failure, or while a sender holds it.
    not_before TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX email_outbox_next ON email_outbox (priority DESC, not_before, id);

CREATE TABLE newsletter_post (
    id UUID PRIMARY KEY,
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    author UUID REFERENCES "user" (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ,
    sent_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    -- How far through the subscribers, by user id, its mail has been queued; NULL before any.
    queued_through UUID,
    -- When every subscriber's mail had been queued.
    queued_at TIMESTAMPTZ,
    recipients BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX newsletter_post_queueing ON newsletter_post (sent_at)
    WHERE sent_at IS NOT NULL AND queued_at IS NULL;

-- Send newsletters (bit 14) goes to every role holding Manage deployment settings (bit 11),
-- which chose whether the deployment has a newsletter.
UPDATE deployment_role SET permissions = permissions | (1::BIGINT << 14)
    WHERE permissions & (1::BIGINT << 11) <> 0;
