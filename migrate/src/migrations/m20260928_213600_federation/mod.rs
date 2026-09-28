use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_213600_federation",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
