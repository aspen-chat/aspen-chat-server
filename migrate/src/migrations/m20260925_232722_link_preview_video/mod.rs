use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260925_232722_link_preview_video",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
