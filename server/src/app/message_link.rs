//! Links from one message to others. A message whose text links to messages of this deployment
//! (`https://{deployment}/communities/{c}/channels/{ch}/messages/{m}`, or `/dms/{ch}/messages/
//! {m}`, under any thread route the clients give them) records their ids in order as its
//! `linked_messages`, at most `MAX_LINKS`, and clients show each beneath it, as they show a
//! link preview. The ids are read whatever the link's host: a message id is unique, so a link to
//! another deployment's message names nothing here, and a link under `/at/{domain}` is to
//! another deployment's message by its own route and is left alone.
//!
//! Who may see what is decided when the links are read (`read_linked`), for each reader: a
//! message they may not read is `unavailable`, and a deleted one `deleted`. Links are recorded,
//! never followed: the messages a linked message links to are not read with it.

use crate::app::context::GlobalServerContext;
use crate::app::link_preview::extract_urls;
use crate::app::message::{MessageWithRelations, read_messages};
use crate::app::{self, ChannelId, CommunityId, MessageId, UserId};
use crate::database::schema::{channel, message};
use diesel::deserialize::FromSql;
use diesel::pg::{Pg, PgValue};
use diesel::prelude::*;
use diesel::serialize::{Output, ToSql};
use diesel::sql_types::{Array, Nullable, Uuid as SqlUuid};
use diesel::{AsExpression, FromSqlRow};
use diesel_async::RunQueryDsl;
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use url::Url;
use utoipa::ToSchema;

/// The most messages one message may link to and show; links beyond are plain links.
pub const MAX_LINKS: usize = 5;

/// The messages a message links to, in the order its text names them, stored as a `UUID[]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, FromSqlRow, AsExpression)]
#[diesel(sql_type = Array<Nullable<SqlUuid>>)]
pub struct MessageLinks(pub Vec<MessageId>);

impl FromSql<Array<Nullable<SqlUuid>>, Pg> for MessageLinks {
    fn from_sql(value: PgValue<'_>) -> diesel::deserialize::Result<Self> {
        let ids =
            <Vec<Option<uuid::Uuid>> as FromSql<Array<Nullable<SqlUuid>>, Pg>>::from_sql(value)?;
        Ok(Self(ids.into_iter().flatten().map(MessageId).collect()))
    }
}

impl ToSql<Array<Nullable<SqlUuid>>, Pg> for MessageLinks {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        let ids: Vec<Option<uuid::Uuid>> = self.0.iter().map(|id| Some(id.0)).collect();
        <Vec<Option<uuid::Uuid>> as ToSql<Array<Nullable<SqlUuid>>, Pg>>::to_sql(
            &ids,
            &mut out.reborrow(),
        )
    }
}

/// The message a URL links to, when it is a link to a message by one of the routes the clients
/// give messages: under `/communities/` or `/dms/`, ending in `messages/{id}`.
pub fn message_of(url: &Url) -> Option<MessageId> {
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let [first, .., kind, id] = segments.as_slice() else {
        return None;
    };
    if !matches!(*first, "communities" | "dms") || *kind != "messages" {
        return None;
    }
    uuid::Uuid::parse_str(id).ok().map(MessageId)
}

/// The messages `content` links to, in order, each once, at most `MAX_LINKS`; `own` is the
/// linking message's id, which a message cannot show beneath itself.
pub fn parse(content: &str, own: MessageId) -> MessageLinks {
    let mut seen = HashSet::new();
    MessageLinks(
        extract_urls(content, MAX_LINKS, |url| {
            message_of(url).is_some_and(|id| id != own && seen.insert(id))
        })
        .iter()
        .filter_map(message_of)
        .collect(),
    )
}

/// What a reader finds at the end of a message link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LinkedState {
    /// The reader may read it; its record is among the read's `messages`.
    Available,
    /// It was deleted.
    Deleted,
    /// It does not exist here, or the reader may not read it.
    Unavailable,
}

/// One message link as its reader finds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkedMessage {
    pub id: MessageId,
    pub state: LinkedState,
    /// For an available message in a community, the community, which its route names; `null`
    /// for one in a DM, and for one the reader cannot read.
    pub community: Option<CommunityId>,
}

/// What the messages `links` name hold for `caller`: each link's state, and the records of
/// those they may read.
pub async fn read_linked(
    state: &GlobalServerContext,
    caller: UserId,
    links: &[MessageId],
) -> app::Result<(Vec<LinkedMessage>, Vec<MessageWithRelations>)> {
    let ids: Vec<MessageId> = links
        .iter()
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let readable = read_messages(state, caller, &ids).await?;
    let found: HashSet<MessageId> = readable.iter().map(|m| m.message.id).collect();
    // A deleted message is told apart from one the reader may not see only where they may see
    // its channel; anywhere else, its having existed is not theirs to learn.
    let deleted: Vec<MessageId> = {
        let missing: Vec<MessageId> = ids
            .iter()
            .filter(|id| !found.contains(id))
            .copied()
            .collect();
        let mut conn = state.connection_pool.get().await?;
        let deleted: Vec<(MessageId, crate::app::ChannelId)> = message::table
            .select((message::id, message::channel))
            .filter(message::id.eq_any(&missing))
            .filter(message::deleted_at.is_not_null())
            .load(conn.as_mut())
            .await?;
        let mut seen = Vec::new();
        for (id, channel) in deleted {
            if crate::app::permissions::channel_access(state, conn.as_mut(), caller, channel)
                .await
                .is_ok()
            {
                seen.push(id);
            }
        }
        seen
    };
    let deleted_ids: HashSet<MessageId> = deleted.into_iter().collect();
    let channels: Vec<ChannelId> = readable.iter().map(|m| *m.message.channel.id()).collect();
    let communities: HashMap<ChannelId, Option<CommunityId>> = channel::table
        .select((channel::id, channel::community))
        .filter(channel::id.eq_any(&channels))
        .load(state.connection_pool.get().await?.as_mut())
        .await?
        .into_iter()
        .collect();
    let community_of: HashMap<MessageId, Option<CommunityId>> = readable
        .iter()
        .map(|m| {
            let channel = m.message.channel.id();
            (m.message.id, communities.get(channel).copied().flatten())
        })
        .collect();
    let linked = ids
        .into_iter()
        .map(|id| LinkedMessage {
            id,
            state: if found.contains(&id) {
                LinkedState::Available
            } else if deleted_ids.contains(&id) {
                LinkedState::Deleted
            } else {
                LinkedState::Unavailable
            },
            community: community_of.get(&id).copied().flatten(),
        })
        .collect();
    Ok((linked, readable))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(content: &str) -> Vec<MessageId> {
        parse(content, MessageId::new()).0
    }

    #[test]
    fn reads_message_links_by_every_route_the_clients_give_them() {
        let (a, b, c) = (MessageId::new(), MessageId::new(), MessageId::new());
        let content = format!(
            "see https://chat.example/communities/{c1}/channels/{ch}/messages/{a} and \
             [this](https://chat.example/dms/{ch}/messages/{b}) and \
             https://chat.example/communities/{c1}/channels/{ch}/threads/{ch}/messages/{c}",
            c1 = uuid::Uuid::now_v7(),
            ch = uuid::Uuid::now_v7(),
            a = a.0,
            b = b.0,
            c = c.0,
        );
        assert_eq!(links(&content), vec![a, b, c]);
    }

    #[test]
    fn leaves_other_links_and_other_deployments_routes_alone() {
        let id = uuid::Uuid::now_v7();
        assert!(links(&format!("https://example.com/messages/{id}")).is_empty());
        assert!(
            links(&format!(
                "https://chat.example/at/other.example/dms/{id}/messages/{id}"
            ))
            .is_empty()
        );
        assert!(links(&format!("`https://chat.example/dms/{id}/messages/{id}`")).is_empty());
    }

    #[test]
    fn names_each_message_once_up_to_the_limit_and_never_itself() {
        let own = MessageId::new();
        let ids: Vec<MessageId> = (0..7).map(|_| MessageId::new()).collect();
        let channel = uuid::Uuid::now_v7();
        let link = |id: MessageId| format!("https://chat.example/dms/{channel}/messages/{}", id.0);
        let content = std::iter::once(link(own))
            .chain(std::iter::once(link(ids[0])))
            .chain(ids.iter().map(|id| link(*id)))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(parse(&content, own).0, ids[..MAX_LINKS].to_vec());
    }

    #[test]
    fn previews_skip_message_links() {
        let id = uuid::Uuid::now_v7();
        assert!(
            crate::app::link_preview::extract_preview_urls(&format!(
                "https://chat.example/dms/{id}/messages/{id}"
            ))
            .is_empty()
        );
    }
}
