use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261012_162005_database_latency",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
