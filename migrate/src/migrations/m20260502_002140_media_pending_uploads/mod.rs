use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260502_002140_media_pending_uploads",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
