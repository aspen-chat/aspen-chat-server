use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_182423_bot_transfer",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
