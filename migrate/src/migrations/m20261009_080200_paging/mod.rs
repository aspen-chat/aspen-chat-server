use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261009_080200_paging",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
