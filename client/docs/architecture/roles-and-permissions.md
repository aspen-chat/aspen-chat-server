# Roles, permissions, and pins

## Where it lives

| Part | Where |
| --- | --- |
| The resolver | `packages/protocol/src/permissions.ts` (mirrors the server's `app::permissions`) |
| Shared test cases | `spec/permission_vectors.json` at the repository root |
| Management UI | `src/features/community-settings` |
| Name colours | `src/features/users/nameColor.ts`, `src/theme/nameColors.ts`, `nameHueOf` (`packages/protocol/src/nameHue.ts`) |

## Resolving permissions

Roles and permissions resolve on the client with `packages/protocol/src/permissions.ts`, by the
same rules as the server's `app::permissions`.

**Both resolvers run the cases in `spec/permission_vectors.json`. A change to either resolver
must change the vectors and the other resolver with it.**

## Store state

The store holds:

- each community's roles;
- channel and category overrides;
- every known member's roles.

They come from the `roles` sideload, which the bootstrap and `loadCommunity` ask for, and from
`role`, `channelOverride`, `categoryOverride`, and `userCommunity` events.

| Question | Store method | Topic | Hooks |
| --- | --- | --- | --- |
| What may the caller do in a community? | `access(communityId)` | `access:<id>` | `useAccess`, `useCan` |
| What may the caller do in a channel? | `channelAccess(channelId)` | `channelAccess:<id>` | `useChannelAccess`, `useChannelCan` |

- A thread's channel permissions are its parent's.
- In a DM the caller has every channel permission, and pinning.
- `UserCommunity.sortIndex` is `null` on everyone's membership but the caller's.

## Hidden controls

Controls the caller may not use are not shown:

- adding and arranging channels;
- the invite manager's create and revoke;
- the composer's attach, poll, and send. A note replaces the box without sending;
- message delete, pin, reaction, and thread actions;
- voice moderation, joining, and sharing;
- renaming and deleting a channel from its menu;
- removing an attachment or someone's reaction.

**Why:** the server refuses anyway. Hiding only keeps the UI honest.

## Losing and gaining access

| Change | What the client does |
| --- | --- |
| The caller loses sight of a channel | The store lets it go at once, as on `channelRemoved` |
| The caller may gain some channels | `AspenSync` reads the community again after a random pause of at most `ACCESS_RELOAD_SPREAD_MS`, since the server sends nothing about channels a member could not see |
| A category's own overrides no longer leave the caller View channel | The store lets go of the category and its overrides. Its channels the caller may still view stay filed under it, and the sidebar shows them at the top level |
| A `categoryOverride` about a category the store does not hold | The store reads the community again |
| The caller loses Manage invites | The store lets go of every invite but the caller's own. Without it the server sends only the caller's own |

- Categories follow the same rule as channels, by their own overrides alone.
- A category the store does not hold counts in `channelAccess` as denying View channel. A
  channel's own overrides may grant it again.

## Voice

A join offer says whether the caller may speak and share (`VoiceCallState.canSpeak`, `canShare`).
Without Speak the call joins to listen, opening no microphone.

## Community settings

`src/features/community-settings` is the management UI.

### `CommunitySettingsDialog`

Opened from the sidebar gear.

| Section | Holds |
| --- | --- |
| Overview | Name and icon, the caller's own nickname, ownership, delete or leave |
| Roles | Templates, and a grouped `PermissionChecklist` whose unheld permissions are disabled |
| Members | Each member by their nickname, with their roles, Remove, Clear nickname (Manage nicknames; see [profiles-and-icons.md](profiles-and-icons.md)), and Ban for holders of Ban members |

### Bans

`BanDialog` takes:

- a reason;
- how long for;
- for a banner who also holds Manage messages, whether the member's messages from the last hour
  or day go too.

`BannedList`, beneath the members, shows the standing bans with Lift ban.

- Bans are `RecordStore.bans`, topic `bans:<communityId>`.
- `useBans` reads a page on first use. `ShowMore` reads the next while `RecordStore.listComplete`
  says there is one.
- `communityBan` events keep them, and reach holders of Ban members.

Someone banned who tries an invite is answered `banned` with the reason, which the invite screen
shows.

### `AccessDialog`

Opened from a channel's menu or a category's lock.

- It leads with three presets: everyone, only some roles, read-only.
- Advanced holds per-role allow, default, and deny.
- Check access holds a per-member explanation (`explain`).
- A preset lets its roles in before shutting everyone else out, so their members never lose the
  channel in between.

## Choosing a member

Wherever a member is chosen (the Members tab, handing over ownership, Check access),
`useMemberSearch`:

1. shows the member sample until something is typed;
2. then, for those the server lets search every member, searches with
   `AspenSync.searchMembers`.

- The server's search matches nicknames too.
- The rule for who may search is mirrored on the client, so the field is not offered to anyone
  refused.
- `AspenSync.searchMembers` caches the people found and their roles without adding them to the
  sample.
- `MemberPicker` is the combobox the dialogs share.

## Pins

Pins are store state per channel: topic `pins:<channelId>`, `usePins`. They are read once on
first use and kept by `pin` events.

`PinsButton` shows them in the channel and DM headers:

- each pin with its author's picture;
- drawn by `MessageBody`, the same body the channel draws (text, tags, pictures, files, poll, link
  cards);
- with Unpin for those who may pin.

## Name colours

Names are drawn in their roles' colours (`src/features/users/nameColor.ts`).

### Drawing a hue

A role carries only a hue around HSV's wheel. `src/theme/nameColors.ts`:

1. draws it in OKLCH at the hue of HSV's fully saturated colour, from one lightness for light
   grounds and one for dark;
2. lowers the chroma where sRGB cannot show it;
3. darkens or lightens until it reaches 4.6:1 against the hardest ground a name sits on;
4. gives both as one `light-dark()` colour.

Its test checks every hue against every palette's surfaces and `accent-soft` in `styles.css`.

### Choosing the hue

`nameHueOf` (`packages/protocol/src/nameHue.ts`) chooses, in order:

1. the user's `nameHue`, or a deployment role's, anywhere;
2. otherwise, inside a community, its highest held role that has one.

`useNameColor(userId, communityId)` applies it. It applies nothing at all while the device
preference `NAME_COLORS` (Settings, Appearance) is off.

### What is coloured

- message authors and their tags;
- the member list. An offline row keeps the plain ink, since the row is dimmed;
- the profile card;
- call participants;
- reactors;
- pins;
- search results;
- embeds;
- `PersonName` given a `community`.

A role tag takes its role's colour. `RoleSwatch` puts a dot of it beside a role's name.

Message reads ask for `memberships`, the authors' roles where the messages were posted.
`AspenSync` notes them without adding anyone to the member sample, so an author outside it is
coloured too.

### Editing a colour

The role editors offer `HuePicker`:

- whether the role has a colour;
- a hue slider (React Aria's `ColorSlider`);
- the name previewed on a light ground and a dark one, each a box whose `color-scheme` makes the
  palette's tokens resolve as in that scheme.

## Roles shown apart

A community role may be shown apart (`hoist`). The member list puts online holders under their
highest such role, highest first, above Online and Offline (`shownApartRole`).

The server puts connected holders first in the member sample. So `AspenSync` reads the sample
again (`loadMemberSample`, after a random pause of at most `MEMBER_RESAMPLE_SPREAD_MS`) when:

- a role comes to be shown apart, or stops being;
- a role shown apart moves or is deleted;
- a role is given or taken.
