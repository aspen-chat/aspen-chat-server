use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261025_044332_user_identity_issuer",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
