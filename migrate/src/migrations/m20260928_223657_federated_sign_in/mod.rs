use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_223657_federated_sign_in",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
