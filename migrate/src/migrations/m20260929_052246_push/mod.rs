use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260929_052246_push",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
