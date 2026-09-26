use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260926_002117_poll_option_emoji",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
