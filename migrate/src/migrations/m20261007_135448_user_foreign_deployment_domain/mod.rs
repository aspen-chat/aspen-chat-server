use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_135448_user_foreign_deployment_domain",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
