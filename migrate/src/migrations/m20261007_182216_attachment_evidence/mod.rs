use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_182216_attachment_evidence",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
