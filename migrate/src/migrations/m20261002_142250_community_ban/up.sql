-- A ban from a community: the member is removed and refused every way back in, with the reason
-- they are told, until `until` passes (never, when it is null) or the ban is lifted.
CREATE TABLE community_ban (
    community UUID NOT NULL REFERENCES community (id) ON DELETE CASCADE,
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    banned_by UUID REFERENCES "user" (id) ON DELETE SET NULL,
    reason TEXT,
    banned_at TIMESTAMPTZ NOT NULL,
    until TIMESTAMPTZ,
    PRIMARY KEY (community, "user")
);

-- Ban members (1 << 13, 8192) goes to every role that holds Remove members (1 << 7, 128),
-- which the Moderator and Admin templates give and the everyone role does not.
UPDATE community_role SET permissions = permissions | 8192 WHERE permissions & 128 = 128;
