use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261022_231920_message_replies",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
