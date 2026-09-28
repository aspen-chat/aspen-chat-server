use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_161434_user_block",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
