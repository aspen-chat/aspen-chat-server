use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260930_150734_everyone_mention_limit",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
