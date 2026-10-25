use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261025_050226_held_message_made_thread",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
