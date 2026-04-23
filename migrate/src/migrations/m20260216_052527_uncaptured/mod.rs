use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260216_052527_uncaptured",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
