use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261003_203845_username_ignores_case",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
