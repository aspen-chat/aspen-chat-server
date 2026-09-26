-- Where the community sits in the member's own list; the client renumbers on drag and drop.
ALTER TABLE community_user ADD COLUMN sort_index INTEGER NOT NULL DEFAULT 0;
