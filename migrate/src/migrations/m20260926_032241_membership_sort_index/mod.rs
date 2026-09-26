use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260926_032241_membership_sort_index",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
