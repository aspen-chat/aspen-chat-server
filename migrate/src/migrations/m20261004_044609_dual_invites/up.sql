-- A dual invite: a registration invite that also joins the community of `community_invite` when
-- it makes an account (`app::registration_invite`).
ALTER TABLE registration_invite
    ADD COLUMN community_invite TEXT REFERENCES invite(code) ON DELETE SET NULL;
