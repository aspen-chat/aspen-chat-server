use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261004_070339_deployment_settings",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
