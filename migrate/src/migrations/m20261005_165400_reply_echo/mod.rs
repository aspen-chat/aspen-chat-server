use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261005_165400_reply_echo",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
