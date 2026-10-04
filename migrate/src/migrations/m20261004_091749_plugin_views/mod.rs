use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_091749_plugin_views",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
