use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260929_035231_message_search",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
