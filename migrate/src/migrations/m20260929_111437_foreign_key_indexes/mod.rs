use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260929_111437_foreign_key_indexes",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
