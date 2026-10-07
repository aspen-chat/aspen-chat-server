use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_181608_deployment_roles_own_people_only",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
