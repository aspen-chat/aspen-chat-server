use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260925_235811_polls",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
