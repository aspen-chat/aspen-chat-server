use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261012_043722_stream_sizes",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
