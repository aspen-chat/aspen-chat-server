# Roles, permissions, and pins

- Roles and permissions resolve on the client with `packages/protocol/src/permissions.ts`, the
  same rules as the server's `app::permissions`; both run the cases in
  `spec/permission_vectors.json` at the repository root, so a change to either resolver must
  change the vectors and the other resolver with it. The store holds each community's roles,
  channel and category overrides, and every known member's roles (from the `roles` sideload,
  which the bootstrap and `loadCommunity` ask for, and from `role`, `channelOverride`,
  `categoryOverride`, and `userCommunity` events), and answers `access(communityId)` (topic
  `access:<id>`, hook `useAccess` / `useCan`) and `channelAccess(channelId)` (topic
  `channelAccess:<id>`, `useChannelAccess` / `useChannelCan`; a thread's are its parent's, a
  DM's every channel permission and pinning). Controls the caller may not use are not shown:
  adding and arranging channels, the invite manager's create and revoke, the composer's
  attach, poll, and send (a note replaces the box without sending), message delete, pin,
  reaction, and thread actions, voice moderation, joining, and sharing; renaming and deleting a
  channel from its menu, and removing an attachment or someone's reaction. The server refuses
  anyway; hiding only keeps the UI honest. When the caller loses sight of a channel the store
  lets it go at once (as `channelRemoved`); when they may gain some, `AspenSync` reads the
  community again after a random pause of at most `ACCESS_RELOAD_SPREAD_MS`, since the server
  sends nothing about channels a member could not see. The same holds for invites: without
  Manage invites the server sends only the caller's own, and the store lets the rest go when
  the permission is lost. `UserCommunity.sortIndex` is `null` on everyone's membership but the
  caller's. A join offer says whether the caller
  may speak and share (`VoiceCallState.canSpeak`, `canShare`); without Speak the call joins to
  listen, opening no microphone. `src/features/community-settings` is the management UI: the
  sidebar gear's `CommunitySettingsDialog` (name and icon, ownership, delete or leave; roles,
  with templates and a grouped `PermissionChecklist` whose unheld permissions are disabled; and
  members, with their roles, Remove, and, for holders of Ban members, Ban: `BanDialog` takes a
  reason, how long for, and, for a banner who also holds Manage messages, whether their messages
  from the last hour or day go too, and `BannedList` beneath the members shows the standing
  bans (`RecordStore.bans`, topic `bans:<communityId>`, read on first use by `useBans` and kept
  by `communityBan` events, which reach holders of Ban members) with Lift ban; someone banned
  who tries an invite is answered `banned` with the reason, which the invite screen shows), and
  `AccessDialog`, opened from a channel's menu or a
  category's lock, which leads with three presets (everyone, only some roles, read-only) and
  keeps per-role allow, default, and deny under Advanced and a per-member explanation
  (`explain`) under Check access. Wherever a member is chosen (the Members tab, handing over
  ownership, Check access), `useMemberSearch` shows the member sample until something is typed,
  and then, for those the server lets search every member (the same rule, mirrored so the field
  is not offered to anyone refused), searches with `AspenSync.searchMembers`, which caches the
  people found and their roles without adding them to the sample; `MemberPicker` is the
  combobox the dialogs share. A preset lets its roles in before shutting everyone else out,
  so their members never lose the channel in between. Pins are store state per channel (topic
  `pins:<channelId>`, `usePins`, read once on first use and kept by `pin` events), shown by
  `PinsButton` in the channel and DM headers, each pin with its author's picture and drawn by
  `MessageBody`, the same body the channel draws (text, tags, pictures, files, poll, link
  cards), with Unpin for those who may pin.
