use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261025_051906_identity_provider_distrust_email",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
