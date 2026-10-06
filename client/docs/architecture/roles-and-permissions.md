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
  sends nothing about channels a member could not see. Categories follow the same rule by
  their own overrides alone: the store lets go of one (and its overrides) once they no longer
  leave the caller View channel, keeping its channels the caller may still view filed under
  it, which the sidebar shows at the top level; a `categoryOverride` about a category the
  store does not hold reads the community again. A category the store does not hold counts in
  `channelAccess` as denying View channel, which a channel's own overrides may grant again.
  The same holds for invites: without
  Manage invites the server sends only the caller's own, and the store lets the rest go when
  the permission is lost. `UserCommunity.sortIndex` is `null` on everyone's membership but the
  caller's. A join offer says whether the caller
  may speak and share (`VoiceCallState.canSpeak`, `canShare`); without Speak the call joins to
  listen, opening no microphone. `src/features/community-settings` is the management UI: the
  sidebar gear's `CommunitySettingsDialog` (name and icon, the caller's own nickname,
  ownership, delete or leave; roles, with templates and a grouped `PermissionChecklist` whose
  unheld permissions are disabled; and members, each by their nickname, with their roles,
  Remove, Clear nickname (Manage nicknames; see Profiles and icons), and, for holders of Ban
  members, Ban: `BanDialog` takes a
  reason, how long for, and, for a banner who also holds Manage messages, whether their messages
  from the last hour or day go too, and `BannedList` beneath the members shows the standing
  bans (`RecordStore.bans`, topic `bans:<communityId>`, read on first use by `useBans` and kept
  by `communityBan` events, which reach holders of Ban members) with Lift ban; someone banned
  who tries an invite is answered `banned` with the reason, which the invite screen shows), and
  `AccessDialog`, opened from a channel's menu or a
  category's lock, which leads with three presets (everyone, only some roles, read-only) and
  keeps per-role allow, default, and deny under Advanced and a per-member explanation
  (`explain`) under Check access. Wherever a member is chosen (the Members tab, handing over
  ownership, Check access), `useMemberSearch` shows the member sample until something is typed
  (the server's search matches nicknames too),
  and then, for those the server lets search every member (the same rule, mirrored so the field
  is not offered to anyone refused), searches with `AspenSync.searchMembers`, which caches the
  people found and their roles without adding them to the sample; `MemberPicker` is the
  combobox the dialogs share. A preset lets its roles in before shutting everyone else out,
  so their members never lose the channel in between. Pins are store state per channel (topic
  `pins:<channelId>`, `usePins`, read once on first use and kept by `pin` events), shown by
  `PinsButton` in the channel and DM headers, each pin with its author's picture and drawn by
  `MessageBody`, the same body the channel draws (text, tags, pictures, files, poll, link
  cards), with Unpin for those who may pin.
- Names are drawn in their roles' colours (`src/features/users/nameColor.ts`). A role carries
  only a hue around HSV's wheel; `src/theme/nameColors.ts` draws it in OKLCH at the hue of
  HSV's fully saturated colour, from one lightness for light grounds and one for dark, lowering
  the chroma where sRGB cannot show it and then darkening or lightening until it reaches 4.6:1
  against the hardest ground a name sits on, and gives both as one `light-dark()` colour; its
  test checks every hue against every palette's surfaces and `accent-soft` in `styles.css`.
  `nameHueOf` (`packages/protocol/src/nameHue.ts`) chooses the hue: the user's `nameHue`, a
  deployment role's, anywhere; otherwise, inside a community, its highest held role that has
  one. `useNameColor(userId, communityId)` applies it, and nothing at all while the device
  preference `NAME_COLORS` (Settings, Appearance) is off. Message authors and their tags, the
  member list (an offline row keeps the plain ink, since the row is dimmed), the profile card,
  call participants, reactors, pins, search results, embeds, and `PersonName` given a
  `community` are coloured; a role tag takes its role's colour, and `RoleSwatch` puts a dot of
  it beside a role's name. Message reads ask for `memberships`, the authors' roles where the
  messages were posted, which `AspenSync` notes without adding anyone to the member sample, so
  an author outside it is coloured too. The role editors offer `HuePicker`: whether the role
  has a colour, a hue slider (React Aria's `ColorSlider`), and the name previewed on a light
  ground and a dark one, each a box whose `color-scheme` makes the palette's tokens resolve as
  in that scheme. A community role may also be shown apart (`hoist`): the member list puts
  online holders under their highest such role, highest first, above Online and Offline
  (`shownApartRole`), and since the server puts connected holders first in the member sample,
  `AspenSync` reads the sample again (`loadMemberSample`, after a random pause of at most
  `MEMBER_RESAMPLE_SPREAD_MS`) when a role comes to be shown apart or not, one shown apart
  moves or is deleted, or one is given or taken.
