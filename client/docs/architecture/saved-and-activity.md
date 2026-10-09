# Saved messages and activity

Two pages of the reader's own, across every deployment they use (`useSources`), opened from
the top of the community rail (`CommunityRail`: Activity, then Saved messages, then the DMs,
each a round link drawn alike, `railLinkClass`, ringed while its page is open) and routed at
`/activity` and `/saved` (never under `/at/{domain}`, since they span deployments). Both are `PersonalPage`s
(`src/features/activity`), a `RailPage` (`src/features/layout`, the layout the Administration
Dashboard uses too) whose rail goes between the two and holds whatever more the page puts in
it, with the user bar at its foot. Their content scrolls only up and down, so a phone never
pans them sideways; what is wider than the column (a code block) scrolls in itself, and the
filters' long names are cut short. Each message they list is a `ListedMessage`
(`src/features/messages`, which a channel's pins use too): its author, when, where it was said
(`useMessagePlace`, which search's results use too: the governing channel, the thread, the
link that goes to it, and a line naming the place, with `on {domain}` for another deployment),
a way to go to it, actions beside that, and the message drawn as its channel draws it. Each
row runs in its deployment's `SourceScope`, and reads its message on demand
(`useMessageOnDemand`); a message the server refuses or no longer has is marked missing in the
store (`MissingKind` `message`) and its row leaves the list, so a message deleted, or a channel
lost, takes its rows with it.

- Saved messages are store state (`RecordStore.saves`, topic `saved`, newest save first, and
  `isSaved`, topic `saved:<messageId>`), read whole at bootstrap (`replaceSaves`, from
  `GET /users/@me/saved-messages`, which a deployment without it answers with nothing) and kept
  current by `savedMessageChanged`. `AspenSync.setSaved` saves and unsaves, following at once.
  Every message's actions offer Save message or Remove from saved (`SaveButton`, in the hover
  bar and the long-press sheet alike), a refusal (the deployment keeps as many as it will)
  said in a toast; a saved message carries a quiet bookmark after whatever it ends with, kept
  with a grouped message's time (`SavedMark` in `MessageBody`), named for assistive technology. `SavedScreen` merges every deployment's saves
  by their ids (UUIDv7s, which compare by time across deployments), shows `SAVED_PAGE` at a time
  with Show more, and reads each deployment's messages a page at a time
  (`AspenSync.loadSavedMessages`) as far as its saves among those shown; each row removes its
  save.
- The activity feed (`ActivityScreen`) reads each deployment's page of what tells the reader
  (`AspenSync.readActivity`, `ACTIVITY_PAGE` at a time, with read positions sideloaded),
  merges them as search does (`mergeResults`), and reads older pages on request. What
  `AspenSync.onNotify` announces while it is open, the same rule, joins the top of its
  deployment's feed if the filter shows it. A message after the reader's position where it was
  said (`useLastRead`: a channel's, or a thread's own) carries an unread dot; opening the feed
  marks nothing read. A reply in a thread has a Reply button that opens the thread's `Composer`
  in the row, so the reader replies without leaving. What the feed shows is chosen in its rail
  (`ActivityFilters`, folded away on a one-pane screen): each deployment, and within it the
  reader's DMs and each community, left out by `ACTIVITY_HIDDEN`, a device preference of what
  is hidden (so a community joined later shows), which `readFilter` turns into each read's
  filter (a deployment wholly hidden is not asked; one whose communities are all hidden but
  whose DMs show is asked for `NO_COMMUNITY`, a community no one belongs to); Unread only is the
  device preference `ACTIVITY_UNREAD_ONLY`, which reads again with `filter[unread]`.
