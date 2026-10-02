//! Sideloading related records on read endpoints.
//!
//! A read that supports `?include=` returns a [`Sideloaded`] envelope: the requested record or
//! records under `data`, and the related records the caller asked for under `included`, keyed by
//! record type. The envelope is always present on such endpoints, whether or not `include` was
//! given, so a generated client sees one response type per endpoint. Each key in `included` is
//! present exactly when the caller asked for it, so an empty array means "requested, none
//! exist" and an absent key means "not requested".
//!
//! Records inside `included` are the same wire types the entity's own endpoints return and the
//! event stream carries, so a client can drop them straight into its cache. Each endpoint that
//! supports `include` declares its own enum of accepted relationship names; the parameter is a
//! comma-separated list of those names (`?include=channels,categories`), which is how OpenAPI
//! describes an array query parameter with `style: form, explode: false`.

use crate::api::attachment::Attachment;
use crate::api::category_collapse::CategoryCollapse;
use crate::api::channel_mute::ChannelMute;
use crate::api::message_enum::{
    Category, CategoryOverride, Channel, ChannelOverride, Community, CustomEmoji, Message, Poll,
    Role, User, UserCommunity, VoiceParticipant, VoiceRing, VoiceSession,
};
use crate::api::notification_setting::NotificationSetting;
use crate::api::poll::{OwnWriteIn, PollVote};
use crate::api::react::ReactionSummary;
use crate::api::read_state::ReadState;
use serde::Serialize;
use serde::de::{Deserialize, Deserializer, IntoDeserializer};
use std::borrow::Cow;
use utoipa::openapi::schema::{ArrayBuilder, ObjectBuilder, Ref};
use utoipa::openapi::{RefOr, Schema};
use utoipa::{PartialSchema, ToSchema};

/// The parsed value of an `include` query parameter: a set of relationship names drawn from the
/// endpoint's own enum `E`. Duplicates are collapsed. Deserializes from a comma-separated
/// string; an empty or absent parameter is an empty set.
#[derive(Debug, Clone)]
pub struct IncludeSet<E>(Vec<E>);

impl<E> Default for IncludeSet<E> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<E: PartialEq> IncludeSet<E> {
    pub fn contains(&self, relation: E) -> bool {
        self.0.contains(&relation)
    }
}

impl<'de, E> Deserialize<'de> for IncludeSet<E>
where
    E: Deserialize<'de> + PartialEq,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let mut relations = Vec::new();
        for name in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let relation = E::deserialize(name.into_deserializer())?;
            if !relations.contains(&relation) {
                relations.push(relation);
            }
        }
        Ok(Self(relations))
    }
}

/// Related records sideloaded alongside a read, keyed by record type. A key is present exactly
/// when the caller asked for the relationship that produces it.
#[derive(Default, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Included {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub communities: Option<Vec<Community>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub channels: Option<Vec<Channel>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub categories: Option<Vec<Category>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub users: Option<Vec<User>>,
    /// Membership records linking `users` to the communities they belong to. Present with
    /// `users` whenever community members were requested, because a flat list of users cannot
    /// say which community each one is a member of.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub user_communities: Option<Vec<UserCommunity>>,
    /// Messages other than those read: the thread replies that echoes in the read name.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub messages: Option<Vec<Message>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub attachments: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub polls: Option<Vec<Poll>>,
    /// The calling user's own votes on the polls in `polls` (or on the poll read). Present
    /// whenever polls were requested, because an anonymous poll's record does not say who
    /// voted, and a reader still needs to see their own choice.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub poll_votes: Option<Vec<PollVote>>,
    /// The calling user's own standing write-ins on the same polls, present with `pollVotes`,
    /// since an anonymous poll's record does not say who wrote what.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub own_write_ins: Option<Vec<OwnWriteIn>>,
    /// How far the caller has read each channel in the read, one record per channel they
    /// belong to, threads excepted.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub read_states: Option<Vec<ReadState>>,
    /// The caller's mutes in force among the channels in the read.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub channel_mutes: Option<Vec<ChannelMute>>,
    /// The caller's notification settings for the communities and channels in the read.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub notification_settings: Option<Vec<NotificationSetting>>,
    /// The categories in the read the caller has collapsed in their channel list.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub category_collapses: Option<Vec<CategoryCollapse>>,
    /// The messages' reactions in brief, one per message and emoji, each message's emoji in the
    /// order they were first used on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub reactions: Option<Vec<ReactionSummary>>,
    /// The calls in progress on the communities' voice channels, with who is in them as
    /// `voiceParticipants`. Present together whenever voice was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub voice_sessions: Option<Vec<VoiceSession>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub voice_participants: Option<Vec<VoiceParticipant>>,
    /// Who the DMs' calls are ringing, as long as each ring lasts. Present with the calls
    /// whenever a DM list asks for voice.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub voice_rings: Option<Vec<VoiceRing>>,
    /// The communities' roles, lowest first within each, with their channel and category
    /// overrides as `channelOverrides` and `categoryOverrides`. Present together whenever
    /// roles were requested.
    /// The communities' own emoji, present whenever `emoji` was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub custom_emoji: Option<Vec<CustomEmoji>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub roles: Option<Vec<Role>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub channel_overrides: Option<Vec<ChannelOverride>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub category_overrides: Option<Vec<CategoryOverride>>,
}

/// Response envelope of a single-record read that supports `?include=`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sideloaded<T> {
    pub data: T,
    pub included: Included,
}

impl<T> Sideloaded<T> {
    pub fn new(data: T, included: Included) -> Self {
        Self { data, included }
    }
}

/// Response envelope of a list read that supports `?include=`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideloadedList<T> {
    pub data: Vec<T>,
    pub included: Included,
}

impl<T> SideloadedList<T> {
    pub fn new(data: Vec<T>, included: Included) -> Self {
        Self { data, included }
    }
}

// The OpenAPI schemas of the two envelopes are written by hand so that `data` is a `$ref` to the
// record's own component schema. Deriving `ToSchema` on a generic struct copies the record's
// schema inline instead, which would give the generated client an anonymous type for `data`
// rather than the named record type it uses everywhere else.

fn envelope_schema(data: impl Into<RefOr<Schema>>) -> RefOr<Schema> {
    ObjectBuilder::new()
        .property("data", data)
        .required("data")
        .property("included", Ref::from_schema_name(Included::name()))
        .required("included")
        .into()
}

fn envelope_schemas<T: ToSchema>(schemas: &mut Vec<(String, RefOr<Schema>)>) {
    schemas.push((T::name().into_owned(), T::schema()));
    T::schemas(schemas);
    schemas.push((Included::name().into_owned(), Included::schema()));
    Included::schemas(schemas);
}

impl<T: ToSchema> PartialSchema for Sideloaded<T> {
    fn schema() -> RefOr<Schema> {
        envelope_schema(Ref::from_schema_name(T::name()))
    }
}

impl<T: ToSchema> ToSchema for Sideloaded<T> {
    fn name() -> Cow<'static, str> {
        format!("Sideloaded_{}", T::name()).into()
    }

    fn schemas(schemas: &mut Vec<(String, RefOr<Schema>)>) {
        envelope_schemas::<T>(schemas);
    }
}

impl<T: ToSchema> PartialSchema for SideloadedList<T> {
    fn schema() -> RefOr<Schema> {
        envelope_schema(
            ArrayBuilder::new()
                .items(Ref::from_schema_name(T::name()))
                .build(),
        )
    }
}

impl<T: ToSchema> ToSchema for SideloadedList<T> {
    fn name() -> Cow<'static, str> {
        format!("SideloadedList_{}", T::name()).into()
    }

    fn schemas(schemas: &mut Vec<(String, RefOr<Schema>)>) {
        envelope_schemas::<T>(schemas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{CommunityId, UserId};
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    enum Relation {
        Channels,
        Members,
    }

    #[derive(Debug, Deserialize)]
    struct Query {
        #[serde(default)]
        include: IncludeSet<Relation>,
    }

    fn parse(query: &str) -> Result<Query, serde_urlencoded::de::Error> {
        serde_urlencoded::from_str(query)
    }

    #[test]
    fn parses_comma_separated_relations() {
        let q = parse("include=channels,members").unwrap();
        assert!(q.include.contains(Relation::Channels));
        assert!(q.include.contains(Relation::Members));
    }

    #[test]
    fn tolerates_whitespace_and_duplicates() {
        let q = parse("include=channels,%20channels,").unwrap();
        assert_eq!(q.include.0, vec![Relation::Channels]);
    }

    #[test]
    fn absent_and_empty_are_empty_sets() {
        assert!(parse("").unwrap().include.0.is_empty());
        assert!(parse("include=").unwrap().include.0.is_empty());
    }

    #[test]
    fn rejects_unknown_relation() {
        let err = parse("include=channels,pins").unwrap_err().to_string();
        assert!(err.contains("pins"), "{err}");
    }

    /// Only the requested relationship types appear, and requesting a relationship with no
    /// records yields an empty array rather than an absent key.
    #[test]
    fn included_serializes_only_requested_keys() {
        let community = CommunityId::new();
        let user = UserId::new();
        let included = Included {
            users: Some(Vec::new()),
            user_communities: Some(vec![UserCommunity {
                community,
                user,
                sort_index: Some(0),
                roles: Vec::new(),
            }]),
            ..Included::default()
        };
        assert_eq!(
            serde_json::to_value(Sideloaded::new(json!({"id": 1}), included)).unwrap(),
            json!({
                "data": {"id": 1},
                "included": {
                    "users": [],
                    "userCommunities": [{"community": community.0, "user": user.0, "sortIndex": 0, "roles": []}],
                },
            })
        );
    }
}
