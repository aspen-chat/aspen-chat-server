-- A moderator's mute of someone in a community's calls (`app::voice::mutes`): it stands in
-- every call of the community, joins and rejoins included, until a moderator lifts it, and
-- outlives the person leaving the community.
CREATE TABLE voice_mute (
    community UUID NOT NULL REFERENCES community (id) ON DELETE CASCADE,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    muted_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    muted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community, "user")
);
CREATE INDEX voice_mute_user ON voice_mute ("user");
CREATE INDEX voice_mute_muted_by ON voice_mute (muted_by);
