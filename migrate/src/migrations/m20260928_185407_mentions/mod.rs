use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_185407_mentions",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
