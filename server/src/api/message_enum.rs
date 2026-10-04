use crate::api::link_preview::LinkPreview;
use crate::api::poll::{PollOption, PollOptionResult, PollWriteIn};
use crate::api::user::{CustomStatus, UserOnlineStatus};
use crate::api::voice::VoiceSessionEndReason;
use crate::app::channel::ChannelType;
use crate::app::deployment::DeploymentPermission;
use crate::app::message::MessageKind;
use crate::app::permissions::Permission;
use crate::app::{
    AttachmentId, CategoryId, ChannelId, CommunityId, CustomEmojiId, IconId, MessageId, PollId,
    ReportCaseId, RoleId, UserId, VoiceServerId, VoiceSessionId,
};
use chrono::Utc;
use message_gen::message_enum_source;

// WARNING: message_enum_source is a special macro. The below enum will not appear in the final program, but this is responsible
// for generating all record types, REST request bodies, and server events. This comment is not a doc comment. This is intentional.
#[message_enum_source]
enum MessageEnumSource {
    User {
        #[message_gen(id)]
        id: UserId,
        name: String,
        #[message_gen(secret)]
        password: String,
        // The registration invite, needed when `[registration] invite_required` is set; see
        // `app::registration_invite`.
        #[message_gen(secret)]
        invite_code: Option<String>,
        icon: Option<IconId>,
        #[message_gen(server_authoritative)]
        online_status: UserOnlineStatus,
        // The profile. Each is absent until the user sets it and cleared with `null`; the
        // bounds are `app::user`'s `DISPLAY_NAME_MAX_CHARS` and friends.
        display_name: Option<String>,
        pronouns: Option<String>,
        bio: Option<String>,
        status: Option<CustomStatus>,
        // A bot signs in only with a token and is labelled as one wherever it is named; see
        // `app::bot`.
        #[message_gen(server_authoritative)]
        bot: bool,
        // The deployment's own account, which sends notices from the deployment itself and is
        // labelled as the system wherever it is named; see `app::system_account`.
        #[message_gen(server_authoritative)]
        system: bool,
        // Who made the bot and manages it: `None` for a person, and for a bot whose maker
        // deleted their account. Handing the bot on changes it.
        #[message_gen(server_authoritative = "mutable")]
        bot_owner: Option<UserId>,
        // Whether anyone allowed to add bots to a community may add this one; a private bot
        // is added only by its owner.
        #[message_gen(server_authoritative = "mutable")]
        bot_public: bool,
        // For a user of another deployment, that deployment's domain, which clients show beside
        // their name; `None` for this deployment's own users. See `app::federation::abroad`.
        #[message_gen(server_authoritative)]
        home_domain: Option<String>,
        // Their id at home, which with `home_domain` names the same person on every deployment,
        // as clients need to hold a block across deployments; `None` for this deployment's own.
        #[message_gen(server_authoritative)]
        home_id: Option<uuid::Uuid>,
        // The hue of the highest deployment role they hold that has one, which clients draw
        // their name in everywhere, over any community role's; `None` without one. See
        // `app::deployment_role`.
        #[message_gen(server_authoritative = "mutable")]
        name_hue: Option<i16>,
    },
    // The user's account preferences were written, by one of their devices; the others fetch
    // them. The values themselves stay out of the stream, which everyone receives.
    #[message_gen(custom_event)]
    UserPreferencesChanged {
        user: UserId,
        updated_at: chrono::DateTime<Utc>,
    },
    // How far the user has read a channel moved forward, on one of their devices or by their
    // posting there; the others follow. `last_read` is a position among the channel's message
    // ids (see `app::read_state`), and it only ever moves forward.
    #[message_gen(custom_event)]
    ChannelRead {
        channel: ChannelId,
        last_read: MessageId,
    },
    // Something announced about the community may not have happened: a request published
    // events about it inside a transaction that was then rolled back. Whoever holds the
    // community's state reads it again. See `app::events::settle`.
    #[message_gen(custom_event)]
    CommunityResync { community: CommunityId },
    // Some of the user's sign-ins ended: signed out, or every other one when they changed their
    // password or second factor, or all of them. Sign-ins are named by `app::login::sign_in_id`.
    // `ended` names the one that ended; without it, every sign-in but `kept` did. Event streams
    // of an ended sign-in receive this and close as unauthorized; no other stream receives it.
    #[message_gen(custom_event)]
    SignInsEnded {
        ended: Option<String>,
        kept: Option<String>,
    },
    // What the user may do across the deployment changed: a deployment role of theirs was
    // given, taken, changed, or deleted. See `app::deployment`.
    #[message_gen(custom_event)]
    DeploymentAccessChanged {
        permissions: Vec<DeploymentPermission>,
    },
    // The user was banned from the deployment: each of their event streams closes after this,
    // and they cannot sign in again until `until` passes, or until the ban is lifted. See
    // `app::user_ban`.
    #[message_gen(custom_event)]
    AccountBanned {
        reason: Option<String>,
        until: Option<chrono::DateTime<Utc>>,
    },
    // What awaits review changed: a report was made, or the case `case` was resolved,
    // dismissed, or restored. Sent to each holder of Review reports, with how many cases are
    // open now. See `app::report`.
    #[message_gen(custom_event)]
    ReportsChanged { case: ReportCaseId, open: i64 },
    // The user collapsed or expanded a category in their channel list, on one of their
    // devices; the others follow. See `app::category_collapse`.
    #[message_gen(custom_event)]
    CategoryCollapseChanged {
        category: CategoryId,
        collapsed: bool,
    },
    // A bot published a new list of the commands it answers; clients that complete commands
    // read it again. See `app::bot_command`.
    #[message_gen(custom_event)]
    BotCommandsChanged { bot: UserId },
    // Someone invoked one of the bot's commands, which is published to the bot alone. The
    // arguments are checked against their parameters' types; `invocation` is the message
    // that shows the command in `channel`. See `app::bot_command`.
    #[message_gen(custom_event)]
    BotCommandInvoked {
        invocation: MessageId,
        channel: ChannelId,
        community: Option<CommunityId>,
        invoker: UserId,
        bot: UserId,
        command: String,
        arguments: Vec<crate::app::bot_command::Argument>,
    },
    // The user blocked or unblocked someone, on one of their devices; the others follow. The
    // blocked user is never told. See `app::block`.
    #[message_gen(custom_event)]
    UserBlockChanged { user: UserId, blocked: bool },
    // Another deployment the user signs in to says they are now in a DM there, started by, or
    // joined to them by, `by_name`: their devices sign in there if they are not, and read it.
    // `channel` is that deployment's id. See `app::federation::notices`.
    #[message_gen(custom_event)]
    ForeignDmJoined {
        domain: String,
        channel: ChannelId,
        by_name: String,
        by_display_name: Option<String>,
    },
    // The user muted or unmuted a channel, on one of their devices; the others follow. A mute
    // with no `until` lasts until it is lifted. See `app::channel_mute`.
    #[message_gen(custom_event)]
    ChannelMuteChanged {
        channel: ChannelId,
        muted: bool,
        until: Option<chrono::DateTime<Utc>>,
    },
    // The user changed what they want to be told of a community or a channel, on one of their
    // devices; the others follow. One of `community` and `channel` is set; `level` is `null`
    // when the setting was removed. See `app::notification_setting`.
    #[message_gen(custom_event)]
    NotificationSettingChanged {
        community: Option<CommunityId>,
        channel: Option<ChannelId>,
        level: Option<crate::app::notification_setting::NotificationLevel>,
    },
    Message {
        #[message_gen(id)]
        id: MessageId,
        #[message_gen(parent)]
        channel_id: ChannelId,
        content: String,
        #[message_gen(server_authoritative)]
        author: UserId,
        #[message_gen(server_authoritative)]
        timestamp: chrono::DateTime<Utc>,
        // Set whenever the content changes; `None` until the first edit.
        #[message_gen(server_authoritative = "mutable")]
        edited_at: Option<chrono::DateTime<Utc>>,
        attachments: Vec<AttachmentId>,
        // Empty at creation; a background fetch fills it in afterwards and a content edit
        // clears it, each announced by an `Update` event carrying the new set.
        #[message_gen(server_authoritative = "mutable")]
        link_previews: Vec<LinkPreview>,
        // `Standard` for anything a client posts; the poll kinds are created by `POST
        // /channels/{channel}/polls` and by the poll closer, with `poll` naming their poll.
        #[message_gen(server_authoritative)]
        kind: MessageKind,
        #[message_gen(server_authoritative)]
        poll: Option<PollId>,
        // The thread this message started, once someone replies to it in a thread; announced
        // by an `Update` event when the thread is made.
        #[message_gen(server_authoritative = "mutable")]
        thread: Option<ChannelId>,
        // For a `ThreadEcho`, the thread reply it shows in the parent channel; the echo has no
        // content of its own.
        #[message_gen(server_authoritative)]
        echo_of: Option<MessageId>,
        // Who it tags, as far as its author was allowed to (see `app::mention`); an edit that
        // changes the text tags afresh, announced by an `Update` event carrying the new set.
        #[message_gen(server_authoritative = "mutable")]
        mentions: crate::app::mention::Mentions,
        // For a `Call`, how long the DM's call lasted, in seconds; `None` for every other kind,
        // a `MissedCall` included.
        #[message_gen(server_authoritative)]
        call_seconds: Option<i32>,
        // For a `Command`, the bot it was sent to; `None` for every other kind, and once the
        // bot is gone.
        #[message_gen(server_authoritative)]
        command_bot: Option<UserId>,
        // The messages of this deployment its text links to, in order, at most
        // `app::message_link::MAX_LINKS`, which clients show beneath it; what each reader finds
        // at each is read with `include=linked`. An edit that changes the text links afresh,
        // announced by an `Update` event carrying the new set.
        #[message_gen(server_authoritative = "mutable")]
        linked_messages: Vec<MessageId>,
        // For a `Warning`, what it warns about: the person warned, and the message or the
        // profile as the reports found it. See `app::report`.
        #[message_gen(server_authoritative)]
        warning: Option<crate::app::report::Warning>,
        // On a reply posted to a thread, also show it in the thread's parent channel, as a
        // `ThreadEcho` message there.
        #[message_gen(secret)]
        echo_to_parent: Option<bool>,
    },
    Poll {
        #[message_gen(id)]
        id: PollId,
        #[message_gen(parent)]
        channel_id: ChannelId,
        // The message of kind `poll` this poll is shown in, created together with it.
        #[message_gen(server_authoritative)]
        message_id: MessageId,
        #[message_gen(server_authoritative)]
        created_by: UserId,
        #[message_gen(server_authoritative)]
        created_at: chrono::DateTime<Utc>,
        #[message_gen(server_authoritative)]
        closes_at: chrono::DateTime<Utc>,
        // Set by the closer once `closes_at` has passed; votes are refused from then on.
        #[message_gen(server_authoritative = "mutable")]
        closed_at: Option<chrono::DateTime<Utc>>,
        // One entry per option, the creator's then the written-in ones, in index order, updated
        // with every vote.
        #[message_gen(server_authoritative = "mutable")]
        results: Vec<PollOptionResult>,
        #[message_gen(permanent)]
        question: String,
        #[message_gen(permanent)]
        options: Vec<PollOption>,
        #[message_gen(permanent)]
        multiple_choice: bool,
        // Whether voters may add answers of their own, one each.
        #[message_gen(permanent)]
        allow_write_ins: bool,
        // The answers voters added, in the order they were added, after `options` in the index
        // space votes use: the first is option `options.len()`. A removed one stays as `null`,
        // keeping its index, so an answer's index never changes.
        #[message_gen(server_authoritative = "mutable")]
        write_ins: Vec<Option<PollWriteIn>>,
        // An anonymous poll reports counts only; who voted is never sent to any client.
        #[message_gen(permanent)]
        anonymous: bool,
        // How long the poll stays open, given by the creator. Only the resulting `closes_at`
        // is stored and sent, so this is accepted on create and appears nowhere else.
        #[message_gen(secret)]
        duration_seconds: u32,
    },
    Pin {
        #[message_gen(id = "client_authoritative")]
        message_id: MessageId,
        #[message_gen(server_authoritative)]
        timestamp: chrono::DateTime<Utc>,
        sort_index: i32,
    },
    React {
        #[message_gen(id = "client_authoritative")]
        message_id: MessageId,
        #[message_gen(id = "client_authoritative")]
        emoji: String,
        #[message_gen(id)]
        user_id: UserId,
    },
    Channel {
        #[message_gen(id)]
        id: ChannelId,
        parent_category: Option<CategoryId>,
        community: Option<CommunityId>,
        name: String,
        #[message_gen(permanent)]
        ty: ChannelType,
        sort_index: i32,
        // A thread's parent channel and the message in it the thread started.
        #[message_gen(server_authoritative)]
        parent_channel: Option<ChannelId>,
        #[message_gen(server_authoritative)]
        starter_message: Option<MessageId>,
        // A thread's replies and the time of the latest, for the summary under its starter.
        #[message_gen(server_authoritative = "mutable")]
        reply_count: i32,
        #[message_gen(server_authoritative = "mutable")]
        last_reply_at: Option<chrono::DateTime<Utc>>,
        // The people in a DM or group DM; empty for every other channel.
        #[message_gen(server_authoritative = "mutable")]
        recipients: Vec<UserId>,
        // The overrides a community channel starts with, one per role, written with it so it is
        // never open to more people than these allow. Each is checked as setting it afterwards
        // would be; they are then the channel's overrides like any other.
        #[message_gen(secret)]
        overrides: Option<Vec<crate::app::role::RoleOverride>>,
    },
    Category {
        #[message_gen(id)]
        id: CategoryId,
        #[message_gen(parent)]
        community: CommunityId,
        name: String,
        sort_index: i32,
    },
    Community {
        #[message_gen(id)]
        id: CommunityId,
        name: String,
        icon: Option<IconId>,
        // Who owns it: every permission, and alone may delete it or hand it on. `None` for a
        // community from before owners were recorded, until the terminal names one.
        #[message_gen(server_authoritative = "mutable")]
        owner: Option<UserId>,
    },
    // A role in a community (`app::permissions`). Roles rank by `position`; the everyone role,
    // every member's, is at 0.
    /// A community's own emoji: a named picture used in its messages as `<:id>` and as a
    /// reaction. Changed only by holders of Manage custom emoji; a rename is announced as its
    /// `update`, and deleting it takes its reactions with it.
    CustomEmoji {
        #[message_gen(id)]
        id: CustomEmojiId,
        #[message_gen(parent)]
        community: CommunityId,
        // What the UI calls it, unique within the community ignoring case: 2 to 32 characters
        // of any script, without whitespace or colons.
        name: String,
        // The picture, an icon uploaded first (PNG, JPEG, WebP, or GIF, at most 256 KiB and
        // 128 by 128).
        #[message_gen(permanent)]
        icon: IconId,
        #[message_gen(server_authoritative)]
        created_by: Option<UserId>,
    },
    Role {
        #[message_gen(id)]
        id: RoleId,
        #[message_gen(parent)]
        community: CommunityId,
        name: String,
        #[message_gen(server_authoritative = "mutable")]
        position: i32,
        permissions: Vec<Permission>,
        #[message_gen(server_authoritative)]
        everyone: bool,
        // The bot the role was made for when it was added: it is that bot's alone, cannot be
        // given to anyone else or deleted, and goes when the bot leaves.
        #[message_gen(server_authoritative)]
        bot: Option<UserId>,
        // The hue, 0 to 359, that holders' names are drawn in within the community when this is
        // the highest role they hold that has one; clients choose the saturation and lightness so
        // that every hue reads. Everyone's role has none.
        hue: Option<i16>,
        // Whether holders are shown under this role, apart from other members, in the member
        // list, and come first in the member sample. Everyone's role is never shown apart.
        #[message_gen(default)]
        hoist: bool,
    },
    // One role's channel permissions allowed or denied in one channel, over what the role
    // grants across the community.
    ChannelOverride {
        #[message_gen(id = "client_authoritative")]
        channel: ChannelId,
        #[message_gen(id = "client_authoritative")]
        role: RoleId,
        allow: Vec<Permission>,
        deny: Vec<Permission>,
    },
    // The same, for every channel of a category.
    CategoryOverride {
        #[message_gen(id = "client_authoritative")]
        category: CategoryId,
        #[message_gen(id = "client_authoritative")]
        role: RoleId,
        allow: Vec<Permission>,
        deny: Vec<Permission>,
    },
    UserCommunity {
        #[message_gen(id = "client_authoritative")]
        community: CommunityId,
        #[message_gen(id)]
        user: UserId,
        #[message_gen(secret)]
        invite_code: String,
        // Where the community sits in this member's own list. Set by `PATCH
        // /communities/{community}/members/@me`; a new membership goes at the end. It is the
        // member's alone: `null` in every record and event anyone else receives.
        #[message_gen(server_authoritative = "mutable")]
        sort_index: Option<i32>,
        // The roles the member holds besides everyone's.
        #[message_gen(server_authoritative = "mutable")]
        roles: Vec<RoleId>,
    },
    // A channel's call while anyone is in it. Created and ended by the voice server's reports,
    // never by a client request; clients join through `POST /channels/{channel}/voice/join`.
    #[message_gen(no_commands)]
    VoiceSession {
        #[message_gen(id)]
        id: VoiceSessionId,
        #[message_gen(server_authoritative)]
        channel: ChannelId,
        #[message_gen(server_authoritative)]
        voice_server: VoiceServerId,
        #[message_gen(server_authoritative)]
        created_at: chrono::DateTime<Utc>,
    },
    // Someone in a call. `muted` and `deafened` follow the voice server's reports.
    #[message_gen(no_commands)]
    VoiceParticipant {
        #[message_gen(id = "client_authoritative")]
        session: VoiceSessionId,
        #[message_gen(id)]
        user: UserId,
        #[message_gen(server_authoritative)]
        channel: ChannelId,
        #[message_gen(server_authoritative)]
        joined_at: chrono::DateTime<Utc>,
        #[message_gen(server_authoritative = "mutable")]
        muted: bool,
        #[message_gen(server_authoritative = "mutable")]
        deafened: bool,
        // Whether they are sharing a screen, window, or game into the call.
        #[message_gen(server_authoritative = "mutable")]
        sharing_screen: bool,
    },
    // Someone a DM's call is ringing: everyone in the DM who was not in the call when it
    // started, until they join it or decline it, or `until` passes, which every client
    // observes by its own clock, with no event. Its `delete` says they joined or declined; the
    // session's own `delete` ends every ring of it.
    #[message_gen(no_commands)]
    VoiceRing {
        #[message_gen(id = "client_authoritative")]
        session: VoiceSessionId,
        #[message_gen(id)]
        user: UserId,
        #[message_gen(server_authoritative)]
        channel: ChannelId,
        // Who started the call.
        #[message_gen(server_authoritative)]
        caller: UserId,
        #[message_gen(server_authoritative)]
        until: chrono::DateTime<Utc>,
    },
    // Why a call ended, sent just before the session's `delete` event. A client that was in
    // the call uses `reason` to tell its user, in particular that an idle call was ended to
    // free the voice server.
    #[message_gen(custom_event)]
    VoiceSessionEnded {
        id: VoiceSessionId,
        channel: ChannelId,
        reason: VoiceSessionEndReason,
    },
    // Someone in a call started or stopped speaking. Not stored; the client keeps the last
    // time each participant spoke from these.
    #[message_gen(custom_event)]
    VoiceSpeaking {
        channel: ChannelId,
        user: UserId,
        speaking: bool,
    },
    /// A ban from a community (`app::ban`): the person is removed and refused every way back in
    /// until `until`, or until the ban is lifted. Made and lifted through `PUT` and `DELETE
    /// /communities/{community}/bans/{user}`; its events reach holders of Ban members.
    #[message_gen(no_commands)]
    CommunityBan {
        #[message_gen(id = "client_authoritative")]
        community: CommunityId,
        #[message_gen(id)]
        user: UserId,
        // What the banned person is told, if anything.
        #[message_gen(permanent)]
        reason: Option<String>,
        // When the ban ends by itself; `null` for one that lasts until it is lifted.
        #[message_gen(permanent)]
        until: Option<chrono::DateTime<Utc>>,
        #[message_gen(server_authoritative)]
        banned_by: Option<UserId>,
        #[message_gen(server_authoritative)]
        banned_at: chrono::DateTime<Utc>,
    },
    #[message_gen(no_commands)]
    Invite {
        #[message_gen(id = "client_authoritative")]
        code: String,
        #[message_gen(permanent)]
        community: CommunityId,
        #[message_gen(server_authoritative)]
        created_by: UserId,
        #[message_gen(server_authoritative)]
        created_at: chrono::DateTime<Utc>,
        expires_at: Option<chrono::DateTime<Utc>>,
    },
}

#[cfg(test)]
mod tests {
    use crate::api;
    use crate::app::{IconId, MessageId, UserId};
    use serde_json::json;

    use super::request::CommunityUpdateRequest;
    use super::server_event::{CommunityEvent, MessageEvent, ReactEvent, ServerEvent};

    /// Events are internally tagged twice: `serverEvent` names the entity, `type` names the
    /// operation, and the record's own fields sit beside them at the top level.
    #[test]
    fn create_event_is_flattened() {
        let message_id = MessageId::new();
        let user_id = UserId::new();
        let e = ServerEvent::React(ReactEvent::Create(api::message_enum::React {
            message_id,
            emoji: "😁".to_string(),
            user_id,
        }));
        assert_eq!(
            serde_json::to_value(e).unwrap(),
            json!({
                "serverEvent": "react",
                "type": "create",
                "messageId": message_id.0,
                "emoji": "😁",
                "userId": user_id.0,
            })
        );
    }

    /// An update event only carries the fields that changed; `null` means a nullable field was
    /// cleared, absence means it was left alone.
    #[test]
    fn update_event_omits_unchanged_fields() {
        let id = crate::app::CommunityId::new();
        let e = ServerEvent::Community(CommunityEvent::Update {
            id,
            name: None,
            icon: Some(None),
            owner: None,
        });
        assert_eq!(
            serde_json::to_value(e).unwrap(),
            json!({
                "serverEvent": "community",
                "type": "update",
                "id": id.0,
                "icon": null,
            })
        );
    }

    /// A server-set field marked mutable rides along in update events, and only when it changed.
    #[test]
    fn mutable_server_field_appears_in_update_events_only_when_set() {
        let id = MessageId::new();
        let edited_at = chrono::DateTime::parse_from_rfc3339("2026-09-25T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let edited = ServerEvent::Message(MessageEvent::Update {
            id,
            content: Some("new".to_string()),
            attachments: None,
            edited_at: Some(Some(edited_at)),
            link_previews: None,
            thread: None,
            mentions: None,
            linked_messages: None,
        });
        assert_eq!(
            serde_json::to_value(edited).unwrap(),
            json!({
                "serverEvent": "message",
                "type": "update",
                "id": id.0,
                "content": "new",
                "editedAt": "2026-09-25T12:00:00Z",
            })
        );
        let attachments_only = ServerEvent::Message(MessageEvent::Update {
            id,
            content: None,
            attachments: Some(Vec::new()),
            edited_at: None,
            link_previews: None,
            thread: None,
            mentions: None,
            linked_messages: None,
        });
        assert_eq!(
            serde_json::to_value(attachments_only).unwrap(),
            json!({
                "serverEvent": "message",
                "type": "update",
                "id": id.0,
                "attachments": [],
            })
        );
    }

    #[test]
    fn update_request_distinguishes_absent_from_null() {
        let untouched: CommunityUpdateRequest = serde_json::from_str("{}").unwrap();
        assert!(untouched.name.is_none());
        assert!(untouched.icon.is_none());

        let cleared: CommunityUpdateRequest = serde_json::from_str(r#"{"icon": null}"#).unwrap();
        assert_eq!(cleared.icon, Some(None));

        let icon = IconId::new();
        let set: CommunityUpdateRequest =
            serde_json::from_value(json!({ "name": "x", "icon": icon.0 })).unwrap();
        assert_eq!(set.name.as_deref(), Some("x"));
        assert_eq!(set.icon, Some(Some(icon)));
    }
}
