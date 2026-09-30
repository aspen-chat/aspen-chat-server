use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260930_024332_dm_call_rings",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
