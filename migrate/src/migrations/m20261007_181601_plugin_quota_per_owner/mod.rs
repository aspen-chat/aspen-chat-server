use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_181601_plugin_quota_per_owner",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
