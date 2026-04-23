use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260323_235015_channel_type_enum",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
