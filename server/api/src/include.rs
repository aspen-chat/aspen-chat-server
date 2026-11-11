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
//!
//! A list read that sideloads nothing answers a [`Listed`], and one read a page at a time by a
//! cursor a [`Page`], so every list the API answers is an object with its records under `data`.

use crate::attachment::Attachment;
use crate::category_collapse::CategoryCollapse;
use crate::category_mute::CategoryMute;
use crate::channel_mute::ChannelMute;
use crate::message_enum::{
    Category, CategoryOverride, Channel, ChannelOverride, Community, CustomEmoji, Message, Poll,
    Role, User, UserCommunity, VoiceParticipant, VoiceRing, VoiceSession,
};
use crate::notification_setting::NotificationSetting;
use crate::poll::{OwnWriteIn, PollVote};
use crate::react::ReactionSummary;
use crate::read_state::ReadState;
use serde::Serialize;
use serde::de::{Deserialize, Deserializer, IntoDeserializer};
use std::borrow::Cow;
use utoipa::openapi::schema::{ArrayBuilder, ObjectBuilder, Ref, SchemaType, Type};
use utoipa::openapi::{RefOr, Schema};
use utoipa::{PartialSchema, ToSchema};

/// The parsed value of an `include` query parameter: a set of relationship names drawn from the
/// endpoint's own enum `E`. Duplicates are collapsed, and a name this server does not know is
/// left out rather than refused: a newer client, or another deployment's, may ask for a
/// relationship this version lacks, and gets the rest. Deserializes from a comma-separated
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

impl<E: Copy> IncludeSet<E> {
    /// Every name the parameter gave, once each, in the order first given.
    pub fn iter(&self) -> impl Iterator<Item = E> + '_ {
        self.0.iter().copied()
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
            let parsed: Result<E, serde::de::value::Error> =
                E::deserialize(name.into_deserializer());
            let Ok(relation) = parsed else {
                tracing::debug!(name, "ignoring an include relationship this server lacks");
                continue;
            };
            if !relations.contains(&relation) {
                relations.push(relation);
            }
        }
        Ok(Self(relations))
    }
}

/// The parsed value of a filter that takes a comma-separated list of names from the enum `E`
/// (`filter[has]`). Unlike [`IncludeSet`], a name this server does not know is kept for the
/// handler to refuse: a sideload left out loses nothing, but a filter left out answers more than
/// was asked for, as though it were what was asked for.
#[derive(Debug, Clone)]
pub struct FilterSet<E> {
    known: Vec<E>,
    unknown: Option<String>,
}

impl<E> Default for FilterSet<E> {
    fn default() -> Self {
        Self {
            known: Vec::new(),
            unknown: None,
        }
    }
}

impl<E> FilterSet<E> {
    /// The longest unknown name kept for the refusal to quote, in characters.
    const QUOTED_CHARS: usize = 64;

    /// Every name the parameter gave, once each, in the order first given; or the first name
    /// this server does not know, cut to what a refusal quotes.
    pub fn known(self) -> Result<Vec<E>, String> {
        match self.unknown {
            Some(unknown) => Err(unknown),
            None => Ok(self.known),
        }
    }
}

impl<'de, E> Deserialize<'de> for FilterSet<E>
where
    E: Deserialize<'de> + PartialEq,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let mut set = Self::default();
        for name in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let parsed: Result<E, serde::de::value::Error> =
                E::deserialize(name.into_deserializer());
            match parsed {
                Ok(value) if !set.known.contains(&value) => set.known.push(value),
                Ok(_) => {}
                Err(_) if set.unknown.is_none() => {
                    set.unknown = Some(name.chars().take(Self::QUOTED_CHARS).collect());
                }
                Err(_) => {}
            }
        }
        Ok(set)
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
    /// Messages other than those read: the thread replies that echoes in the read name, and
    /// the messages the read links to that the caller may read.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub messages: Option<Vec<Message>>,
    /// What the caller finds at each message the read links to, one record per message linked.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub linked_messages: Option<Vec<aspen_app::message_link::LinkedMessage>>,
    /// The messages the warnings in the read are about, deleted ones included.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub warned_messages: Option<Vec<crate::report::ReviewedMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub attachments: Option<Vec<Attachment>>,
    /// What plugins say about the messages of the read.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub message_annotations: Option<Vec<crate::message_enum::MessageAnnotation>>,
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
    /// belong to: in a community or DM read, its channels, threads excepted; in a message read,
    /// the channels the messages were posted in, threads among them.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub read_states: Option<Vec<ReadState>>,
    /// What a notification says of each message in the read, ready to show: one record per
    /// message that has text or whose kind records something in words.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub notification_texts: Option<Vec<aspen_app::message::notification::NotificationText>>,
    /// The caller's own settings in force among the channels in the read: mutes, and channels
    /// kept unmuted while their category is muted.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub channel_mutes: Option<Vec<ChannelMute>>,
    /// The caller's mutes in force of the categories in the read.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub category_mutes: Option<Vec<CategoryMute>>,
    /// The caller's notification settings for the communities, categories, and channels in the
    /// read.
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

/// Response envelope of a list read that sideloads nothing: the whole list, or the window its
/// parameters ask for, under `data`. Every list answers in an object, never a bare array, so a
/// field can be added beside `data` without breaking those who read it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listed<T> {
    pub data: Vec<T>,
}

impl<T> From<Vec<T>> for Listed<T> {
    fn from(data: Vec<T>) -> Self {
        Self { data }
    }
}

impl<T> FromIterator<T> for Listed<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            data: iter.into_iter().collect(),
        }
    }
}

/// Response envelope of a list read a page at a time. `nextCursor` is always present: text to
/// give back as `?cursor=` for the page after this one, or `null` when this page is the last. A
/// reader decides whether there is more by `nextCursor` alone, never by how many records `data`
/// holds, and takes the cursor as opaque.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    pub fn new(data: Vec<T>, next_cursor: Option<String>) -> Self {
        Self { data, next_cursor }
    }
}

// The OpenAPI schemas of the envelopes are written by hand so that `data` is a `$ref` to the
// record's own component schema. Deriving `ToSchema` on a generic struct copies the record's
// schema inline instead, which would give the generated client an anonymous type for `data`
// rather than the named record type it uses everywhere else.

fn array_of<T: ToSchema>() -> RefOr<Schema> {
    ArrayBuilder::new()
        .items(Ref::from_schema_name(T::name()))
        .into()
}

fn next_cursor_schema() -> RefOr<Schema> {
    ObjectBuilder::new()
        .schema_type(SchemaType::from_iter([Type::String, Type::Null]))
        .description(Some(
            "Give it back as `cursor` for the next page; `null` when this page is the last",
        ))
        .into()
}

fn record_schemas<T: ToSchema>(schemas: &mut Vec<(String, RefOr<Schema>)>) {
    schemas.push((T::name().into_owned(), T::schema()));
    T::schemas(schemas);
}

impl<T: ToSchema> PartialSchema for Listed<T> {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .property("data", array_of::<T>())
            .required("data")
            .into()
    }
}

impl<T: ToSchema> ToSchema for Listed<T> {
    fn name() -> Cow<'static, str> {
        format!("Listed_{}", T::name()).into()
    }

    fn schemas(schemas: &mut Vec<(String, RefOr<Schema>)>) {
        record_schemas::<T>(schemas);
    }
}

impl<T: ToSchema> PartialSchema for Page<T> {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .property("data", array_of::<T>())
            .required("data")
            .property("nextCursor", next_cursor_schema())
            .required("nextCursor")
            .into()
    }
}

impl<T: ToSchema> ToSchema for Page<T> {
    fn name() -> Cow<'static, str> {
        format!("Page_{}", T::name()).into()
    }

    fn schemas(schemas: &mut Vec<(String, RefOr<Schema>)>) {
        record_schemas::<T>(schemas);
    }
}

fn envelope_schema(data: impl Into<RefOr<Schema>>) -> RefOr<Schema> {
    ObjectBuilder::new()
        .property("data", data)
        .required("data")
        .property("included", Ref::from_schema_name(Included::name()))
        .required("included")
        .into()
}

fn envelope_schemas<T: ToSchema>(schemas: &mut Vec<(String, RefOr<Schema>)>) {
    record_schemas::<T>(schemas);
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
        envelope_schema(array_of::<T>())
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
    use aspen_app::{CommunityId, UserId};
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
    fn ignores_relations_it_does_not_know() {
        // A newer client asks for what this version lacks and still gets what it has.
        let q = parse("include=channels,emoji,members").unwrap();
        assert!(q.include.contains(Relation::Channels));
        assert!(q.include.contains(Relation::Members));
        assert_eq!(q.include.0.len(), 2);
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

    #[derive(Debug, Deserialize)]
    struct Filtered {
        #[serde(default)]
        has: FilterSet<Relation>,
    }

    fn filtered(query: &str) -> Result<Vec<Relation>, String> {
        serde_urlencoded::from_str::<Filtered>(query)
            .unwrap()
            .has
            .known()
    }

    /// A filter keeps the name it does not know, so its handler can refuse rather than answer
    /// a broader search than was asked for; what it knows reads as an include's names do.
    #[test]
    fn a_filter_names_the_value_it_does_not_know() {
        assert_eq!(
            filtered("has=channels,%20channels,members"),
            Ok(vec![Relation::Channels, Relation::Members])
        );
        assert_eq!(filtered(""), Ok(Vec::new()));
        assert_eq!(filtered("has="), Ok(Vec::new()));
        assert_eq!(
            filtered("has=channels,video,audio"),
            Err("video".to_string())
        );
        assert_eq!(filtered("has=Channels"), Err("Channels".to_string()));
        let long = "ж".repeat(500);
        assert_eq!(filtered(&format!("has={long}")), Err("ж".repeat(64)));
    }

    /// A list is an object whatever it holds, and a page always says whether another follows.
    #[test]
    fn lists_and_pages_answer_in_objects() {
        assert_eq!(
            serde_json::to_value(Listed::from(vec![1, 2])).unwrap(),
            json!({"data": [1, 2]})
        );
        assert_eq!(
            serde_json::to_value(Vec::<u8>::new().into_iter().collect::<Listed<u8>>()).unwrap(),
            json!({"data": []})
        );
        assert_eq!(
            serde_json::to_value(Page::new(vec![1], Some("next".to_string()))).unwrap(),
            json!({"data": [1], "nextCursor": "next"})
        );
        assert_eq!(
            serde_json::to_value(Page::<u8>::new(Vec::new(), None)).unwrap(),
            json!({"data": [], "nextCursor": null})
        );
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
                nickname: None,
            }]),
            ..Included::default()
        };
        assert_eq!(
            serde_json::to_value(Sideloaded::new(json!({"id": 1}), included)).unwrap(),
            json!({
                "data": {"id": 1},
                "included": {
                    "users": [],
                    "userCommunities": [{"community": community.0, "user": user.0, "sortIndex": 0, "roles": [], "nickname": null}],
                },
            })
        );
    }
}
