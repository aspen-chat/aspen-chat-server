use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260929_011737_federation_standing",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
