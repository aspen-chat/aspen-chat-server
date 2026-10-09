use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261009_080100_evidence_retention",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
