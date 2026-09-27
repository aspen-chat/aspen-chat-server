use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260927_055541_benchmark_runs",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
