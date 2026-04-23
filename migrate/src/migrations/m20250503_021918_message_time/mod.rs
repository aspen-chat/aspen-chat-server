use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20250503_021918_message_time",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
