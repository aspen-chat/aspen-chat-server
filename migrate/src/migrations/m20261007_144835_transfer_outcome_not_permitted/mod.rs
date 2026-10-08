use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_144835_transfer_outcome_not_permitted",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
