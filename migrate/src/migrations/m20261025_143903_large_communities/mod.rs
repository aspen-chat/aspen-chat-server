use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261025_143903_large_communities",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
