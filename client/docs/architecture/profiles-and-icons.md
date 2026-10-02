# Profiles and icons

- Icons (user avatars and community icons) are uploaded through `AspenSync.uploadIcon` (`upload.ts`), the
  server's two-phase flow, and then named by id on the user or community. `IconPicker.tsx` in
  `src/features/media` runs the whole thing: file chooser, the `CropDialog` where the reader
  places a circle that never leaves the picture (geometry in `crop.ts`), a square PNG crop of
  that circle at most `ICON_MAX_SIZE` wide, upload, and the id. `Avatar` takes an `iconId` and
  shows the picture once `useIcon` has the record, initials until then.
- Profiles are fields on the user record (`displayName`, `pronouns`, `bio`, `status` as
  `{ text, emoji? }`), edited by `AspenSync.updateProfile` as a merge patch built by
  `src/features/users/profile.ts`, which also decides what to call a user (`displayNameOf`).
  Show that name wherever a user is named, never `name` directly; `ProfileCard.tsx` is the card
  any user control opens, and `EditProfileDialog.tsx` the signed-in user's editor. Opened within
  a community (a `communityId` in the route) the card lists the roles the user holds there,
  read with `AspenSync.loadMember` when they are outside the member sample, and offers those
  who may change them the ones they could give and an × on each they could take away
  (`useAssignableRoles`, which the members panel's role picker shares).
