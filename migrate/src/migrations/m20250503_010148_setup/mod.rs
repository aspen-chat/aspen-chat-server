use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20250503_010148_setup",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
