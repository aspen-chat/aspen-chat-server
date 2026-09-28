-- What people may do across the whole deployment, as roles ranked by `position` (a holder may
-- manage only roles below their own highest). Permissions are bits of a BIGINT,
-- `app::deployment::DeploymentPermissions`; the numbers below must match its constants:
--
--   1 view the dashboard, 2 manage registration invites, 4 manage voice servers,
--   8 manage deployment roles, 16 moderate any community
CREATE TABLE deployment_role (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    permissions BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX deployment_role_by_position ON deployment_role (position);

CREATE TABLE user_deployment_role (
    "user" UUID NOT NULL REFERENCES "user" (id) ON DELETE CASCADE,
    role UUID NOT NULL REFERENCES deployment_role (id) ON DELETE CASCADE,
    PRIMARY KEY ("user", role)
);
CREATE INDEX user_deployment_role_by_role ON user_deployment_role (role);

-- Every use of Moderate any community that a community's own permissions would not have
-- allowed, and every reading of a DM by someone not in it.
CREATE TABLE moderation_log (
    id UUID PRIMARY KEY,
    actor UUID REFERENCES "user" (id) ON DELETE SET NULL,
    action TEXT NOT NULL,
    community UUID REFERENCES community (id) ON DELETE SET NULL,
    channel UUID REFERENCES channel (id) ON DELETE SET NULL,
    subject TEXT,
    at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX moderation_log_by_time ON moderation_log (at DESC);

-- Administrators become holders of an Administrator role with every deployment permission but
-- moderation, which is given deliberately.
INSERT INTO deployment_role (id, name, position, permissions)
VALUES (uuidv7(), 'Administrator', 1, 15);
INSERT INTO user_deployment_role ("user", role)
SELECT u.id, r.id FROM "user" u, deployment_role r WHERE u.admin;
ALTER TABLE "user" DROP COLUMN admin;
