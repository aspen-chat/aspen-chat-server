use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_142523_plugin_timer_scope",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
