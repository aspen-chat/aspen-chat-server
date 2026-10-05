# Profiles and icons

- Icons (user avatars and community icons) are uploaded through `AspenSync.uploadIcon` (`upload.ts`), the
  server's two-phase flow, and then named by id on the user or community. `IconPicker.tsx` in
  `src/features/media` runs the whole thing: file chooser, the `CropDialog` where the reader
  places a circle that never leaves the picture (geometry in `crop.ts`), a square PNG crop of
  that circle at most `ICON_MAX_SIZE` wide, upload, and the id. `Avatar` takes an `iconId` and
  shows the picture once `useIcon` has the record, initials until then.
- Profiles are fields on the user record (`displayName`, `pronouns`, `bio`, `status` as
  `{ text, emoji? }`, and `publicEmail`, the verified address a user chose to show, which the
  card offers as a `mailto:` link and which only the email settings change; see Sign-in),
  edited by `AspenSync.updateProfile` as a merge patch built by
  `src/features/users/profile.ts`, which also decides what to call a user (`displayNameOf`).
  Show that name wherever a user is named, never `name` directly; `ProfileCard.tsx` is the card
  any user control opens, and `EditProfileDialog.tsx` the signed-in user's editor. Opened within
  a community (a `communityId` in the route) the card lists the roles the user holds there,
  read with `AspenSync.loadMember` when they are outside the member sample, and offers those
  who may change them the ones they could give and an × on each they could take away
  (`useAssignableRoles`, which the members panel's role picker shares). Someone else's card
  offers Message and Call, which a block takes away except Message for a holder of Message any
  user, Block, and Report profile (see Reports).
- Nicknames are a member's name in one community (`nickname` on their `userCommunity`), which
  the store keeps per community and member (`RecordStore.nickname`, `nicknames`, topic
  `nicknames:<communityId>`) from every membership a read or event carries, an update's field
  applied as a merge patch. Within a community a member is called by it in place of their
  display name: `PersonName` with `community`, `useNameIn` (`src/features/users/nameIn.ts`) for
  one person and `useNicknames` for many, in message headers, tags and their completion, the
  member list (sorted by it), calls, reactions, polls, embeds, the members panel, and
  notifications. The card opened in a community shows the nickname with the display name
  beneath; on the reader's own card, and in community settings' Overview, `NicknameForm`
  (`src/features/users/Nickname.tsx`) sets theirs with Change nickname
  (`AspenSync.setNickname`) and clears it with nothing; on anyone else's, and in the members
  panel, `ClearNicknameButton` clears it for holders of Manage nicknames over someone ranked
  below them, never the owner (`useMayClearNickname`, `AspenSync.clearNickname`).
