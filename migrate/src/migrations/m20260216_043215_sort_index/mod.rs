use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260216_043215_sort_index",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
