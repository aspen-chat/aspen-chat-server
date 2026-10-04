use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_203124_manage_plugins_permission",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
