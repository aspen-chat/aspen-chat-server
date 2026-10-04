use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_044609_dual_invites",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
