-- The voice servers this deployment may place calls on. Operators seed them from `aspen.toml`
-- or manage them through the API; `enabled` is cleared by the failure threshold.
CREATE TABLE voice_server (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    url TEXT NOT NULL,
    capacity INTEGER NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    -- From the server's load reports; NULL until it has reported once.
    last_report_at TIMESTAMPTZ,
    reported_participants INTEGER NOT NULL DEFAULT 0
);

-- A channel is bound to one server for as long as anyone is in the call.
CREATE TABLE voice_session (
    id UUID PRIMARY KEY,
    channel UUID NOT NULL UNIQUE REFERENCES channel (id) ON DELETE CASCADE,
    voice_server UUID NOT NULL REFERENCES voice_server (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE voice_participant (
    session UUID NOT NULL REFERENCES voice_session (id) ON DELETE CASCADE,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    joined_at TIMESTAMPTZ NOT NULL,
    muted BOOLEAN NOT NULL DEFAULT FALSE,
    deafened BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (session, "user")
);

-- One row per user who failed to start a session on a server, kept at their latest failure so
-- the threshold counts distinct users within a window rather than retries.
CREATE TABLE voice_server_failure (
    voice_server UUID NOT NULL REFERENCES voice_server (id) ON DELETE CASCADE,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    reported_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (voice_server, "user")
);
