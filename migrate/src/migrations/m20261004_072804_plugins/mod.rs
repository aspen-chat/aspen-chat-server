use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_072804_plugins",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
