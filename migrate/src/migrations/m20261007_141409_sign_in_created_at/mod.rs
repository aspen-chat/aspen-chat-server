use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_141409_sign_in_created_at",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
