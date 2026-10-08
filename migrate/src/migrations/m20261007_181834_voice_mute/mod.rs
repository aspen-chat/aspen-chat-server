use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261007_181834_voice_mute",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
