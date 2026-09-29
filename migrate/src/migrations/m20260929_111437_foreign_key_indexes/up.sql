-- Indexes on references the server looks rows up by, or that PostgreSQL checks when the row
-- referenced goes: without one, each lookup, and each deleted row, reads the whole table.
-- A user's sign-ins, ended together (signing out everywhere, deleting the account, a home that
-- no longer vouches for them), and the sessions of each sign-in.
CREATE INDEX refresh_token_user ON refresh_token ("user");
CREATE INDEX session_refresh_token ON session (refresh_token);
-- A community's channels, categories, and invites, which its reads list.
CREATE INDEX channel_community ON channel (community);
CREATE INDEX category_community ON category (community);
CREATE INDEX channel_parent_category ON channel (parent_category);
CREATE INDEX invite_community ON invite (community);
-- A channel's pins, polls, mutes, and read positions, and the threads started in it.
CREATE INDEX pin_channel ON pin (channel);
CREATE INDEX poll_channel ON poll (channel);
CREATE INDEX channel_mute_channel ON channel_mute (channel);
CREATE INDEX read_state_channel ON read_state (channel);
CREATE INDEX message_thread ON message (thread) WHERE thread IS NOT NULL;
-- What a user did, found when their account goes.
CREATE INDEX react_author ON react (author);
CREATE INDEX poll_vote_user ON poll_vote ("user");
