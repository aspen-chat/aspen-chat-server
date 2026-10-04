UPDATE deployment_role SET permissions = permissions & ~4096;
ALTER TABLE message DROP COLUMN altered_by;
DROP TABLE user_annotation;
DROP TABLE message_annotation;
DROP TABLE plugin_storage;
DROP TABLE community_plugin;
DROP INDEX user_plugin_principal;
ALTER TABLE "user" DROP COLUMN plugin;
DROP TABLE plugin;
