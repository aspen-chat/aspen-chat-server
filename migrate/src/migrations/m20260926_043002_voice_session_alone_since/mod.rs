use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260926_043002_voice_session_alone_since",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
