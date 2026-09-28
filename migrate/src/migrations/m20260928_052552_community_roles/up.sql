-- Roles and permissions within communities.
-- Permissions are bits of a BIGINT, `app::permissions::Permissions`; the numbers below must
-- match its constants.
--
--   community bits:  1 manage community, 2 manage channels, 4 manage categories,
--                    8 create invites, 16 manage invites, 32 manage roles, 64 assign roles,
--                    128 remove members, 256 manage messages, 512 pin messages,
--                    1024 manage calls
--   channel bits:    65536 view channel, 131072 send messages, 262144 attach files,
--                    524288 add reactions, 1048576 start threads,
--                    2097152 send messages in threads, 4194304 create polls,
--                    8388608 join voice, 16777216 speak, 33554432 share screen

-- Who owns the community: every permission, and alone may delete it or hand it on.
ALTER TABLE community ADD COLUMN owner UUID REFERENCES "user" (id) ON DELETE SET NULL;

-- A community's roles, ranked by `position`: a member may act only on members and roles
-- ranked below their own highest role. The one role with `everyone` set is every member's,
-- at position 0.
CREATE TABLE community_role (
    id UUID PRIMARY KEY,
    community UUID NOT NULL REFERENCES community (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    permissions BIGINT NOT NULL DEFAULT 0,
    everyone BOOLEAN NOT NULL DEFAULT false
);
CREATE INDEX community_role_by_community ON community_role (community, position);
CREATE UNIQUE INDEX community_role_everyone ON community_role (community) WHERE everyone;

-- Which roles each member holds, besides everyone's. Leaving the community drops them.
CREATE TABLE community_member_role (
    "user" UUID NOT NULL,
    community UUID NOT NULL,
    role UUID NOT NULL REFERENCES community_role (id) ON DELETE CASCADE,
    PRIMARY KEY ("user", role),
    FOREIGN KEY ("user", community) REFERENCES community_user ("user", community) ON DELETE CASCADE
);
CREATE INDEX community_member_role_by_role ON community_member_role (role);
CREATE INDEX community_member_role_by_member ON community_member_role (community, "user");

-- Per role, channel permissions allowed or denied in one channel, or in every channel of a
-- category, over what the role grants across the community.
CREATE TABLE channel_override (
    channel UUID NOT NULL REFERENCES channel (id) ON DELETE CASCADE,
    role UUID NOT NULL REFERENCES community_role (id) ON DELETE CASCADE,
    allow BIGINT NOT NULL DEFAULT 0,
    deny BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (channel, role)
);
CREATE INDEX channel_override_by_role ON channel_override (role);
CREATE TABLE category_override (
    category UUID NOT NULL REFERENCES category (id) ON DELETE CASCADE,
    role UUID NOT NULL REFERENCES community_role (id) ON DELETE CASCADE,
    allow BIGINT NOT NULL DEFAULT 0,
    deny BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (category, role)
);
CREATE INDEX category_override_by_role ON category_override (role);

-- Existing communities get the same roles a new one does: everyone (the member template),
-- Moderator, and Admin. Nothing records who created them, so they have no owner until one is
-- set from the terminal (`aspen-chat-server communities set-owner`), and every existing member
-- is made an Admin, so no one loses anything they could do before.
INSERT INTO community_role (id, community, name, position, permissions, everyone)
SELECT uuidv7(), id, 'everyone', 0, 67043336, true FROM community;
INSERT INTO community_role (id, community, name, position, permissions, everyone)
SELECT uuidv7(), id, 'Moderator', 1, 67045272, false FROM community;
INSERT INTO community_role (id, community, name, position, permissions, everyone)
SELECT uuidv7(), id, 'Admin', 2, 67045375, false FROM community;
INSERT INTO community_member_role ("user", community, role)
SELECT cu."user", cu.community, r.id
FROM community_user cu
JOIN community_role r ON r.community = cu.community AND r.name = 'Admin' AND NOT r.everyone;
