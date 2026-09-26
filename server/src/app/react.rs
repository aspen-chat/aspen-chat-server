use crate::api::message_enum::server_event::{ReactEvent, ServerEvent};
use crate::api::{GlobalServerContext, message_enum};
use crate::app;
use crate::app::{MessageId, UserId, publish_event};
use crate::database::schema::react;
use diesel::{BoolExpressionMethods, ExpressionMethods, Insertable, Queryable, Selectable};
use diesel_async::RunQueryDsl;
use rust_i18n::t;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = react)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct React {
    pub emoji: String,
    pub author: UserId,
    pub message: MessageId,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub fn validate_emoji(s: &str) -> Result<(), app::Error> {
    match emojis::get(s) {
        Some(_) => Ok(()),
        None => Err(app::Error::Validation(t!("reactMustBeSingleEmoji"))),
    }
}

pub async fn create_react(
    state: &GlobalServerContext,
    author: UserId,
    message_id: MessageId,
    emoji: String,
) -> app::error::Result<React> {
    validate_emoji(&emoji)?;
    let mut conn = state.connection_pool.get().await?;
    let react = React {
        emoji: emoji.clone(),
        author,
        message: message_id,
        timestamp: chrono::Utc::now(),
    };
    diesel::insert_into(react::table)
        .values(&react)
        .execute(conn.as_mut())
        .await?;
    let event = ServerEvent::React(ReactEvent::Create(message_enum::React {
        message_id,
        emoji,
        user_id: author,
    }));
    publish_event(state, &event).await?;
    Ok(react)
}

pub async fn delete_react(
    state: &GlobalServerContext,
    author: UserId,
    message_id: MessageId,
    emoji: String,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let deleted = diesel::delete(react::table)
        .filter(
            react::author
                .eq(author)
                .and(react::message.eq(message_id))
                .and(react::emoji.eq(&emoji)),
        )
        .execute(conn.as_mut())
        .await?;
    if deleted > 0 {
        let event = ServerEvent::React(ReactEvent::Delete {
            message_id,
            emoji,
            user_id: author,
        });
        publish_event(state, &event).await?;
    }
    Ok(())
}
