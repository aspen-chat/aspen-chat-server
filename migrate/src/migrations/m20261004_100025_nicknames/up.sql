-- A member's name in one community, shown there in place of their display name.
ALTER TABLE community_user ADD COLUMN nickname TEXT;

-- Change nickname (bit 14) for every community's everyone role, and Manage nicknames (bit 15)
-- for every role that may remove members (bit 7), as the templates of a new community have them.
UPDATE community_role SET permissions = permissions | (1::BIGINT << 14) WHERE everyone;
UPDATE community_role SET permissions = permissions | (1::BIGINT << 15)
    WHERE permissions & (1::BIGINT << 7) <> 0;

-- Reports of a member's nickname gather in a case per member and community, which goes with
-- the community as a message's case goes with its message. A report keeps the nickname it found.
ALTER TABLE report_case
    DROP CONSTRAINT report_case_kind_check,
    ADD CONSTRAINT report_case_kind_check CHECK (kind IN ('message', 'profile', 'nickname')),
    ADD COLUMN community UUID REFERENCES community (id) ON DELETE CASCADE,
    ADD CONSTRAINT report_case_nickname_kind CHECK ((kind = 'nickname') = (community IS NOT NULL));
CREATE UNIQUE INDEX report_case_unresolved_nickname ON report_case (subject, community)
    WHERE status <> 'resolved' AND kind = 'nickname';
ALTER TABLE report ADD COLUMN nickname TEXT;
