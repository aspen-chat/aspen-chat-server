use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_031305_react_by_message",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
