use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_022402_category_collapse",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
