use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260930_083535_bot_commands",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
