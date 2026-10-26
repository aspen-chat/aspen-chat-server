-- Erasing a community (`app::community_takedown`) walks its members and its channels in order of
-- id, a batch a step, so each batch starts where the last ended; these lead with the community
-- and serve everything the indexes they replace did.
CREATE INDEX community_user_by_community_user ON community_user (community, "user");
DROP INDEX community_user_by_community;
CREATE INDEX channel_by_community ON channel (community, id);
DROP INDEX channel_community;

-- Erasing deletes channels, messages, categories, and community invites outright, so each table
-- whose foreign key reaches one of them, or that it deletes from by one of them, needs an index
-- leading with that column or each deletion reads the whole table.
CREATE INDEX mention_by_channel ON mention (channel);
CREATE INDEX notification_setting_by_channel ON notification_setting (channel)
    WHERE channel IS NOT NULL;
CREATE INDEX notification_setting_by_community ON notification_setting (community)
    WHERE community IS NOT NULL;
CREATE INDEX plugin_notice_by_channel ON plugin_notice (channel);
CREATE INDEX plugin_notice_by_message ON plugin_notice (message) WHERE message IS NOT NULL;
CREATE INDEX category_collapse_by_category ON category_collapse (category);
CREATE INDEX moderation_log_by_channel ON moderation_log (channel) WHERE channel IS NOT NULL;
CREATE INDEX registration_invite_by_community_invite ON registration_invite (community_invite)
    WHERE community_invite IS NOT NULL;
