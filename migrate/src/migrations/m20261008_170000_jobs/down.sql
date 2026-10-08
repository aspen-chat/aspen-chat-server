ALTER TABLE react DROP CONSTRAINT react_custom_emoji_fkey;
ALTER TABLE react ADD CONSTRAINT react_custom_emoji_fkey
    FOREIGN KEY (custom_emoji) REFERENCES custom_emoji (id) ON DELETE CASCADE;
DROP INDEX custom_emoji_name_key;
CREATE UNIQUE INDEX custom_emoji_name_key ON custom_emoji (community, lower(name));
ALTER TABLE custom_emoji DROP COLUMN deleted_at;
ALTER TABLE mention DROP CONSTRAINT mention_target_role_fkey;
ALTER TABLE mention ADD CONSTRAINT mention_target_role_fkey
    FOREIGN KEY (target_role) REFERENCES community_role (id) ON DELETE CASCADE;
ALTER TABLE community_member_role DROP CONSTRAINT community_member_role_role_fkey;
ALTER TABLE community_member_role ADD CONSTRAINT community_member_role_role_fkey
    FOREIGN KEY (role) REFERENCES community_role (id) ON DELETE CASCADE;
ALTER TABLE community_role DROP COLUMN deleted_at;
DROP INDEX refresh_token_expires;
DROP INDEX session_expires;
UPDATE deployment_role SET permissions = permissions & ~(1::bigint << 15);
DROP TABLE job;
