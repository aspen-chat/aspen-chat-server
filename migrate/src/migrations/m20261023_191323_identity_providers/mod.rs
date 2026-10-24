use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261023_191323_identity_providers",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
