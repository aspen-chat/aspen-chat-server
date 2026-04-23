use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260417_200000_message_channel_id_index",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
