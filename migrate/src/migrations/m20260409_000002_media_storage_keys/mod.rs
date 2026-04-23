use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260409_000002_media_storage_keys",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
