use crate::SqlMigration;

pub static M: SqlMigration = SqlMigration {
    id: "20260928_043652_registration_invite_used_up_at",
    up: include_str!("up.sql"),
    down: include_str!("down.sql"),
};
