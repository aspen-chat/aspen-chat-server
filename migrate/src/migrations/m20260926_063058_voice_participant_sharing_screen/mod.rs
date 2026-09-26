use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260926_063058_voice_participant_sharing_screen",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
