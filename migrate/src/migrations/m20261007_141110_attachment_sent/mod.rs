use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_141110_attachment_sent",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
