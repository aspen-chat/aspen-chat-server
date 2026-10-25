use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261025_060007_deployment_documents",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
