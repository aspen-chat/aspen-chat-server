use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20261116_034814_voice_participant_held",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
