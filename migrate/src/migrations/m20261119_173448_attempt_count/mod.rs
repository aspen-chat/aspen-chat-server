use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261119_173448_attempt_count",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
