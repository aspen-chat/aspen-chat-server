# Reports, warnings, and message links

- Reporting. A message's actions offer Report (`ReportMessageButton`, or the `report` sheet on a
  touch screen) to anyone but its author, for every message but the system account's notices
  and a call's record; the profile card offers Report profile (`ReportProfileButton`) to anyone
  but the person themselves and the system account, and, opened in a community where they have
  a nickname, Report nickname (`ReportNicknameButton`, `src/features/users/Nickname.tsx`). Each
  opens `ReportModal` (`src/features/reports/ReportDialog.tsx`): a profile report first asks
  which aspects are wrong (`PROFILE_ASPECTS`, `src/features/users/profileAspects.ts`), while a
  nickname is reported whole, as the server keeps it with its community; then the category, from
  `GET /report-categories` as the server offers them in the reader's language, each with what it
  covers, then anything to add, which the Other category needs. Sent, a toast says so; a second
  report of the same thing is the server's `alreadyReported`, shown as its detail. A report goes
  to the deployment whose message, profile, or community it is, through that scope's
  `AspenSync`.
- Message links. Copy link (`MessageActions`) copies `messageUrl`: the message's route on its
  deployment's own address (`AspenClient.baseUrl`), `/communities/{c}/channels/{ch}/messages/{m}`
  or `/dms/{ch}/messages/{m}`, which the server reads as a link to it in any message
  (`parseMessageUrl` reads the same two routes back; `links.test.ts`). In a message's text such
  a link to the home deployment (the client's address for it, or the page's own) or another the
  user uses opens here (`LinkToMessage` in `Markdown.tsx`), and written as its bare address it
  shows as a short Message chip, since the message itself shows beneath; to any other host it is
  an ordinary link. A message's `linkedMessages` show beneath it as `LinkedMessages`
  (`src/features/messages/EmbeddedMessage.tsx`): each is `EmbeddedMessage`, the author, the time
  as the way to jump to it, and its body drawn `still` (no poll to vote in, no actions, and no
  links of its own followed), or a line saying it was deleted or cannot be viewed. What the
  reader finds at each comes from the store's `linkedMessage` (topic `link:<id>`), filled by
  history reads with `include=linked`, and for a message that arrived or was edited since by
  `AspenSync.loadLinks`, one read per linking message however many embeds ask; a delete event
  for a linked message marks its link deleted.
- Warnings. A message of kind `warning` is drawn by `WarningBody`
  (`src/features/messages/WarningBody.tsx`) in place of its body: labelled a moderator's
  warning, then what it is about, the reported message (from the store's `warnedMessage`,
  topic `warned:<id>`, which history reads fill with `include=warnings`, deleted or not, and
  marked deleted when it is), the profile as the reports found it (`ProfileSnapshotCard`,
  the reported aspects marked), or the nickname with the community it was chosen in, as the
  warning names it (so it reads the same after the person leaves), then the moderator's words.
- Review. The dashboard's Reports tab (`src/features/admin/Reports.tsx`, Review reports) lists
  the open, resolved, and dismissed cases, the open ones counted on the tab and on the rail's
  dashboard button (`useOpenReports`, read from `GET /admin/report-counts` when first asked and
  kept by `reportsChanged` events, `RecordStore.openReports`), and reads the page again on each
  such event (`useReportsChanges`). A case shows what was reported (the message, deleted or not,
  where it was posted, and the conversation around it on request, a page at a time either way,
  the reported message outlined; the profile as reported beside how it is now; or the nickname
  as reported, in which community, and what it is now when the reviewer may read that
  community's members (`NicknameCase`)) and every report of it. Take action opens
  `ResolveDialog`, offering only what the reviewer may do: Warn, with their words, which every reviewer
  may and the system account sends without their name; delete the message, clear the
  nickname, or reset chosen aspects of a profile of this deployment (each Remove content); and
  ban from the server, through the same `BanFields` as a community's ban (Ban users, and
  Remove content to delete their recent messages). Dismiss sets a case aside; the
  Dismissed list restores it. A case the reviewer may not act on (about them, or someone not
  below their rank) says another moderator must. The people and files the dashboard's reads
  name are put in the cache, so messages there draw as everywhere else; channels and
  communities are not, since they would join the reviewer's own lists.
- Categories. The Report categories tab (`src/features/admin/ReportCategories.tsx`, Manage
  report categories) hides and shows categories (Other excepted), adds the deployment's own,
  renames and describes them, and moves them up and down among themselves.
- Bans from the deployment. The user directory filters to banned users (Banned only), shows each
  standing ban and its end, and for holders of Ban users offers Ban (`UserBanDialog`, the
  shared `BanFields` with the deletion window for holders of Moderate any community and, for a
  bot, its owner too) or Lift ban, for anyone but the system account and the caller. A banned
  user's event stream closes with `4410` after an `accountBanned` event; `EventStream` treats it
  as a rejected token, so the refresh that follows fails and the app signs out, and signing in
  shows the server's `deploymentBanned` detail with the reason and the end.
