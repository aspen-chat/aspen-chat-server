ALTER TABLE channel_mute DROP CONSTRAINT channel_mute_kept_unmuted_lasts;
DELETE FROM channel_mute WHERE NOT muted;
ALTER TABLE channel_mute DROP COLUMN muted;
DROP TABLE category_mute;
DELETE FROM notification_setting WHERE category IS NOT NULL;
DROP INDEX notification_setting_by_category;
DROP INDEX notification_setting_all_category;
DROP INDEX notification_setting_category;
ALTER TABLE notification_setting DROP CONSTRAINT notification_setting_one_target;
ALTER TABLE notification_setting DROP COLUMN category;
ALTER TABLE notification_setting
    ADD CONSTRAINT notification_setting_check CHECK ((community IS NULL) <> (channel IS NULL));
