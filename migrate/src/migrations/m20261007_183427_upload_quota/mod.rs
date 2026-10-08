use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_183427_upload_quota",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
