# Profiles and icons

## Where it lives

| Part | Where |
| --- | --- |
| Uploading an icon | `AspenSync.uploadIcon` (`upload.ts`) |
| Picking and cropping | `IconPicker.tsx`, `CropDialog`, `crop.ts` (`src/features/media`) |
| Showing an icon | `Avatar`, `useIcon` |
| Profile patches and names | `src/features/users/profile.ts` (`displayNameOf`) |
| The profile card | `ProfileCard.tsx` |
| Editing one's profile | `EditProfileDialog.tsx` |
| Nicknames | `src/features/users/nameIn.ts`, `src/features/users/Nickname.tsx` |

## Icons

Icons are user avatars and community icons. They are uploaded through `AspenSync.uploadIcon`
with the server's two-phase flow, then named by id on the user or community.

`IconPicker.tsx` runs the whole thing:

1. the file chooser;
2. the `CropDialog`, where the reader places a circle that never leaves the picture (geometry in
   `crop.ts`);
3. a square PNG crop of that circle, at most `ICON_MAX_SIZE` wide;
4. the upload;
5. the id.

`Avatar` takes an `iconId`. It shows initials until `useIcon` has the record, then the picture.

## Profiles

Profiles are fields on the user record.

| Field | What it is |
| --- | --- |
| `displayName` | The name shown |
| `pronouns` | Pronouns |
| `bio` | Bio |
| `status` | `{ text, emoji? }` |
| `publicEmail` | The verified address the user chose to show. The card offers it as a `mailto:` link. Only the email settings change it; see [sign-in.md](sign-in.md) |

- `AspenSync.updateProfile` edits them as a merge patch built by `src/features/users/profile.ts`.
- `profile.ts` also decides what to call a user (`displayNameOf`). **Show that name wherever a
  user is named, never `name` directly.**
- `EditProfileDialog.tsx` is the signed-in user's editor.

## The profile card

`ProfileCard.tsx` is the card any user control opens.

### Roles on the card

Opened within a community (a `communityId` in the route), the card:

- lists the roles the user holds there, read with `AspenSync.loadMember` when they are outside
  the member sample;
- offers those who may change them the roles they could give, and an × on each they could take
  away (`useAssignableRoles`, which the members panel's role picker shares).

### Actions on someone else's card

| Action | Notes |
| --- | --- |
| Message | A block takes it away, except for a holder of Message any user |
| Call | A block takes it away |
| Block | |
| Report profile | See [reports.md](reports.md) |

## Nicknames

A nickname is a member's name in one community (`nickname` on their `userCommunity`).

### In the store

- The store keeps nicknames per community and member: `RecordStore.nickname`, `nicknames`, topic
  `nicknames:<communityId>`.
- They come from every membership a read or event carries.
- An update's field is applied as a merge patch.

### Where they show

Within a community a member is called by their nickname in place of their display name.

- `PersonName` with `community`;
- `useNameIn` (`src/features/users/nameIn.ts`) for one person;
- `useNicknames` for many.

These cover message headers, tags and their completion, the member list (sorted by it), calls,
reactions, polls, embeds, the members panel, and notifications.

The card opened in a community shows the nickname with the display name beneath.

### Setting and clearing

| Where | Who | What |
| --- | --- | --- |
| The reader's own card, and community settings' Overview | The reader | `NicknameForm` (`src/features/users/Nickname.tsx`) sets theirs with Change nickname (`AspenSync.setNickname`), and clears it with nothing |
| Anyone else's card, and the members panel | Holders of Manage nicknames over someone ranked below them, never the owner | `ClearNicknameButton` clears it (`useMayClearNickname`, `AspenSync.clearNickname`) |
