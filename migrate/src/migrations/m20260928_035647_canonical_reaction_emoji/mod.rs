//! Stores every reaction's emoji in the one form Unicode lists for it (fully qualified), as
//! the server now stores new ones, so an emoji once saved both with and without its variation
//! selector (`❤` and `❤️`) is one reaction. Where a person reacted to a message in both forms,
//! the two become one reaction, keeping the earlier time. The canonical form comes from the
//! same `emojis` crate, at the same version, that the server uses.
//!
//! There is nothing to undo: `down` leaves the merged reactions as they are.

use crate::Migration;
use anyhow::Result;
use async_trait::async_trait;
use diesel::sql_types::Text;
use diesel::{QueryableByName, sql_query};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

pub struct CanonicalReactionEmoji;

pub static M: CanonicalReactionEmoji = CanonicalReactionEmoji;

#[derive(QueryableByName)]
struct Stored {
    #[diesel(sql_type = Text)]
    emoji: String,
}

#[async_trait]
impl Migration for CanonicalReactionEmoji {
    fn id(&self) -> &'static str {
        "20260928_035647_canonical_reaction_emoji"
    }

    async fn up(&self, conn: &mut AsyncPgConnection) -> Result<()> {
        let stored: Vec<Stored> = sql_query("SELECT DISTINCT emoji FROM react")
            .load(conn)
            .await?;
        for Stored { emoji } in stored {
            let Some(canonical) = emojis::get(&emoji).map(|e| e.as_str()) else {
                continue;
            };
            if canonical == emoji {
                continue;
            }
            sql_query(
                r#"
                INSERT INTO react (emoji, author, message, "timestamp")
                SELECT $1, author, message, "timestamp" FROM react WHERE emoji = $2
                ON CONFLICT (emoji, author, message)
                DO UPDATE SET "timestamp" = LEAST(react."timestamp", excluded."timestamp")
                "#,
            )
            .bind::<Text, _>(canonical)
            .bind::<Text, _>(&emoji)
            .execute(conn)
            .await?;
            sql_query("DELETE FROM react WHERE emoji = $1")
                .bind::<Text, _>(&emoji)
                .execute(conn)
                .await?;
        }
        Ok(())
    }

    async fn down(&self, _conn: &mut AsyncPgConnection) -> Result<()> {
        Ok(())
    }
}
