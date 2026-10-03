use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261003_045506_deployment_profile",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
