use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261119_172748_used_once",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
