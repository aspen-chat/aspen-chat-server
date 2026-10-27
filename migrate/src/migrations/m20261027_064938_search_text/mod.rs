//! Gives every message what it is searched by (`aspen_search_text`): its content with a space
//! between every two words written without one, so that a word of a language written without
//! spaces between words is searched for on its own, and rebuilds the index `message_search`
//! over it. The column is null where the content is searched as written, which it is for
//! nearly every message in a script that writes spaces.
//!
//! It reads every message of the deployment, a batch at a time, and rebuilds the index once
//! they are written.

use crate::Migration;
use anyhow::Result;
use async_trait::async_trait;
use diesel::sql_types::{Array, Text, Uuid as PgUuid};
use diesel::{QueryableByName, sql_query};
use diesel_async::{AsyncPgConnection, RunQueryDsl, SimpleAsyncConnection};
use uuid::Uuid;

pub struct SearchText;

pub static M: SearchText = SearchText;

/// How many messages are read, and their search text written, at a time: what bounds the
/// migration's memory, since it runs in one transaction either way.
const BATCH: i64 = 10_000;

#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type = PgUuid)]
    id: Uuid,
    #[diesel(sql_type = Text)]
    content: String,
}

/// The index `message_search` over `expression`, which `app::search` writes out literally in
/// each statement that reads it.
fn index_over(expression: &str) -> String {
    format!(
        "CREATE INDEX message_search ON message USING gin (home_channel, to_tsvector('simple', {expression})) \
         WHERE deleted_at IS NULL"
    )
}

#[async_trait]
impl Migration for SearchText {
    fn id(&self) -> &'static str {
        "20261027_064938_search_text"
    }

    async fn up(&self, conn: &mut AsyncPgConnection) -> Result<()> {
        conn.batch_execute("ALTER TABLE message ADD COLUMN search_text TEXT")
            .await?;
        let mut after = Uuid::nil();
        loop {
            let rows: Vec<Row> =
                sql_query("SELECT id, content FROM message WHERE id > $1 ORDER BY id LIMIT $2")
                    .bind::<PgUuid, _>(after)
                    .bind::<diesel::sql_types::BigInt, _>(BATCH)
                    .load(conn)
                    .await?;
            let Some(last) = rows.last() else {
                break;
            };
            after = last.id;
            let (ids, texts): (Vec<Uuid>, Vec<String>) = rows
                .iter()
                .filter_map(|row| Some((row.id, aspen_search_text::search_text(&row.content)?)))
                .unzip();
            if !ids.is_empty() {
                sql_query(
                    "UPDATE message AS m SET search_text = v.search_text \
                     FROM unnest($1::uuid[], $2::text[]) AS v(id, search_text) WHERE m.id = v.id",
                )
                .bind::<Array<PgUuid>, _>(&ids)
                .bind::<Array<Text>, _>(&texts)
                .execute(conn)
                .await?;
            }
        }
        conn.batch_execute(&format!(
            "DROP INDEX message_search; {}",
            index_over("COALESCE(search_text, content)")
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, conn: &mut AsyncPgConnection) -> Result<()> {
        conn.batch_execute(&format!(
            "DROP INDEX message_search; {}; ALTER TABLE message DROP COLUMN search_text",
            index_over("content")
        ))
        .await?;
        Ok(())
    }
}
