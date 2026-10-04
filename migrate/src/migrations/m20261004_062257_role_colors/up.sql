-- A role's hue, 0 to 359 on the colour wheel, which names of those holding it are drawn in;
-- clients choose the saturation and lightness, so every hue reads against their backgrounds.
-- A community role may also be shown separately in the member list (`hoist`), and its holders
-- come first in the member sample. Everyone's role has neither.
ALTER TABLE community_role
    ADD COLUMN hue SMALLINT CHECK (hue BETWEEN 0 AND 359),
    ADD COLUMN hoist BOOLEAN NOT NULL DEFAULT false,
    ADD CONSTRAINT community_role_everyone_plain CHECK (NOT everyone OR (hue IS NULL AND NOT hoist));

ALTER TABLE deployment_role ADD COLUMN hue SMALLINT CHECK (hue BETWEEN 0 AND 359);

-- The hue of the highest deployment role the user holds that has one, which their name is drawn
-- in everywhere; kept by `app::deployment_role` whenever roles, their order, or who holds them
-- change.
ALTER TABLE "user" ADD COLUMN name_hue SMALLINT CHECK (name_hue BETWEEN 0 AND 359);
