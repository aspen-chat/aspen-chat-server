use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_065931_deployment_roles",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
