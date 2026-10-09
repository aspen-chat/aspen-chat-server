use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261009_080000_upkeep",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
