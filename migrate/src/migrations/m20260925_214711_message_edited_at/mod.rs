use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260925_214711_message_edited_at",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
