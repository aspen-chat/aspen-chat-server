//! Replies to messages.
//!
//! A message posted with `replyTo` names one message of its own channel, which clients show
//! above it with a jump to it. Only a live message of the same channel may be replied to, so
//! whoever may read the reply may read what it answers, and no reader needs a check of their
//! own: what each finds there, the message or its having been deleted, is read as a link's is
//! (`app::message_link::read_linked`, `include=linked`). A thread is a channel of its own, so a
//! reply in one answers a message of that thread.
//!
//! A reply tags the author of the message it answers unless it is posted with `tagReplied`
//! false, as `<@id>` in its text would, with the same permission and membership
//! (`app::mention::resolve`); never its own author. `message.tags_replied` keeps the choice, so
//! an edit that tags afresh tags them again while their message is live.
//!
//! The reply is permanent: an edit never changes what it answers, and it outlives that
//! message's deletion, which is soft.

use crate::t;
use crate::{ChannelId, MessageId, UserId};
use aspen_schema::message;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

/// What a message posted as a reply answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reply {
    pub to: MessageId,
    /// Whether it tags the author of `to`.
    pub tag: bool,
}

impl Reply {
    /// The reply a create request asks for: none without `reply_to`, and tagging unless
    /// `tag_replied` says not to.
    pub fn requested(reply_to: Option<MessageId>, tag_replied: Option<bool>) -> Option<Reply> {
        reply_to.map(|to| Reply {
            to,
            tag: tag_replied.unwrap_or(true),
        })
    }
}

/// Checks that `to` is a message of `channel`, in the transaction that posts or holds the reply,
/// answering its author while it is live. It must be live unless `sent_earlier`: a message held
/// for its previews (`super::held`) was checked when it was sent, and is posted replying to what
/// it answered then even if that has been deleted since, tagging no one for it.
pub(super) async fn check(
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
    to: MessageId,
    sent_earlier: bool,
) -> crate::Result<Option<UserId>> {
    // A message of another channel is refused as one that does not exist is, so the refusal
    // tells nothing of channels the author may not read.
    let Some((author, deleted_at)): Option<(UserId, Option<chrono::DateTime<chrono::Utc>>)> =
        message::table
            .select((message::author, message::deleted_at))
            .filter(message::id.eq(to).and(message::channel.eq(channel)))
            .first(conn)
            .await
            .optional()?
    else {
        return Err(crate::Error::Validation(t!("replyElsewhere")));
    };
    if deleted_at.is_some() && !sent_earlier {
        return Err(crate::Error::Validation(t!("replyTargetDeleted")));
    }
    Ok(deleted_at.is_none().then_some(author))
}

/// Whom an edit of `message` tags for its reply, as it tags afresh: the author of the message it
/// replies to when it was posted tagging them, that message is still live, and they are not the
/// editor.
pub(super) async fn tagged_on_edit(
    conn: &mut AsyncPgConnection,
    message: MessageId,
    editor: UserId,
) -> crate::Result<Option<UserId>> {
    let (original, reply) = diesel::alias!(message as original, message as reply);
    Ok(reply
        .inner_join(
            original.on(original
                .field(message::id)
                .nullable()
                .eq(reply.field(message::reply_to))),
        )
        .select(original.field(message::author))
        .filter(
            reply
                .field(message::id)
                .eq(message)
                .and(reply.field(message::tags_replied))
                .and(original.field(message::deleted_at).is_null())
                .and(original.field(message::author).ne(editor)),
        )
        .first(conn)
        .await
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel_async::AsyncConnection;
    use diesel_async::SimpleAsyncConnection;

    const ANN: &str = "4e910000-0000-4000-8000-000000000001";
    const BEN: &str = "4e910000-0000-4000-8000-000000000002";
    const COMMUNITY: &str = "4e910000-0000-4000-8000-000000000010";
    const CHANNEL: &str = "4e910000-0000-4000-8000-000000000011";
    const ELSEWHERE: &str = "4e910000-0000-4000-8000-000000000012";
    const ORIGINAL: &str = "4e910000-0000-4000-8000-000000000020";
    const DELETED: &str = "4e910000-0000-4000-8000-000000000021";
    const OTHER: &str = "4e910000-0000-4000-8000-000000000022";
    const REPLY: &str = "4e910000-0000-4000-8000-000000000023";
    const QUIET: &str = "4e910000-0000-4000-8000-000000000024";
    const TO_DELETED: &str = "4e910000-0000-4000-8000-000000000025";

    fn id(text: &str) -> uuid::Uuid {
        text.parse().unwrap()
    }

    /// Runs against the database `DATABASE_URL` names, inside a transaction that is never
    /// committed; without one it checks nothing.
    #[tokio::test]
    async fn a_reply_answers_a_live_message_of_its_channel_and_tags_its_author() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(&format!(
            r#"INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                 VALUES ('{ANN}', '{ANN}', '', now(), now()), ('{BEN}', '{BEN}', '', now(), now());
             INSERT INTO community (id, name) VALUES ('{COMMUNITY}', 'c');
             INSERT INTO channel (id, name, ty, sort_index, community)
                 VALUES ('{CHANNEL}', 'a', 'text', 0, '{COMMUNITY}'),
                        ('{ELSEWHERE}', 'b', 'text', 1, '{COMMUNITY}');
             INSERT INTO message (id, author, channel, content, "timestamp", deleted_at)
                 VALUES ('{ORIGINAL}', '{ANN}', '{CHANNEL}', 'question', now(), NULL),
                        ('{DELETED}', '{ANN}', '{CHANNEL}', 'gone', now(), now()),
                        ('{OTHER}', '{ANN}', '{ELSEWHERE}', 'elsewhere', now(), NULL);
             INSERT INTO message (id, author, channel, content, "timestamp", reply_to, tags_replied)
                 VALUES ('{REPLY}', '{BEN}', '{CHANNEL}', 'answer', now(), '{ORIGINAL}', true),
                        ('{QUIET}', '{BEN}', '{CHANNEL}', 'answer', now(), '{ORIGINAL}', false),
                        ('{TO_DELETED}', '{BEN}', '{CHANNEL}', 'answer', now(), '{DELETED}', true);"#,
        ))
        .await
        .unwrap();
        let (channel, ann, ben) = (ChannelId(id(CHANNEL)), UserId(id(ANN)), UserId(id(BEN)));

        let live = check(&mut conn, channel, MessageId(id(ORIGINAL)), false).await;
        assert_eq!(live.unwrap(), Some(ann));
        let refused = |result: crate::Result<Option<UserId>>| {
            matches!(result, Err(crate::Error::Validation(_)))
        };
        // Deleted, refused when sent and taken without a tag when a held message is posted.
        assert!(refused(
            check(&mut conn, channel, MessageId(id(DELETED)), false).await
        ));
        let held = check(&mut conn, channel, MessageId(id(DELETED)), true).await;
        assert_eq!(held.unwrap(), None);
        // Another channel's message, or none, is refused alike.
        assert!(refused(
            check(&mut conn, channel, MessageId(id(OTHER)), true).await
        ));
        assert!(refused(
            check(&mut conn, channel, MessageId::new(), false).await
        ));

        let on_edit = async |message: &str, editor: UserId, conn: &mut AsyncPgConnection| {
            tagged_on_edit(conn, MessageId(id(message)), editor)
                .await
                .unwrap()
        };
        assert_eq!(on_edit(REPLY, ben, &mut conn).await, Some(ann));
        assert_eq!(on_edit(REPLY, ann, &mut conn).await, None);
        assert_eq!(on_edit(QUIET, ben, &mut conn).await, None);
        assert_eq!(on_edit(TO_DELETED, ben, &mut conn).await, None);
        assert_eq!(on_edit(ORIGINAL, ann, &mut conn).await, None);
    }
}
