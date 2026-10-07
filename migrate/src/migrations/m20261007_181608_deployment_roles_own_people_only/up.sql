-- Only this deployment's own people hold deployment roles: a bot's or a foreign user's are
-- taken away. The servers read none of them either (`app::deployment::deployment_access`), so
-- nobody held their powers once a server of this version started.
DELETE FROM user_deployment_role
WHERE "user" IN (SELECT id FROM "user" WHERE bot OR home_domain IS NOT NULL);
