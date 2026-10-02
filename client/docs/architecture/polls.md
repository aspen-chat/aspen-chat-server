# Polls

- Polls (`src/features/messages/PollCard.tsx`, `CreatePollDialog.tsx`, `PollClosedNotice.tsx`)
  are records of their own: a message of kind `poll` names one in its `poll` field and carries
  no text, and a message of kind `pollClosed` is the announcement the server posts when the
  deadline passes. The tally lives in the poll record's `results` and is republished as an
  update event on every vote, so a vote only records the caller's own choice locally
  (`store.setMyVote`) and leaves the numbers to the stream. Message reads sideload `polls` with
  the caller's `pollVotes`, which is the only way to learn one's own vote on an anonymous poll;
  `usePoll` fetches a poll the window did not bring. Each option is `{ label, emoji? }`; the
  emoji is chosen in the dialog from the same lazily loaded picker reactions use, and the server
  validates it as a single emoji. The outcome text of the announcement is
  composed on the client from the final tally (`src/features/messages/poll.ts`), so it is
  localized like everything else. The card offers Close poll to the poll's creator and to a
  holder of Manage messages while it is open (`AspenSync.closePoll`), whose closing comes as
  the poll's update and the announcement, like a deadline's.
  A poll whose creator allowed write-ins lists its `writeIns` after its `options`, and both
  share one index space that votes use: write-in `i` is answer `options.length + i`, and a
  removed one stays as `null` so no index moves (`pollChoices`, `choiceAt`). Each write-in
  notes who wrote it in, or only that it was written in on an anonymous poll. Reads with votes
  also sideload `ownWriteIns`, the caller's own standing write-ins (`store.myWriteIns`), which
  decide whether the card offers the field for one (one each) and a remove control, offered
  to the writer and the poll's creator. Writing in an answer the poll already has votes for
  it instead (`sync.writeIn` resolves to the answer voted for). A poll's `update` event that
  nulls an answer also drops the caller's vote and write-in on it, since anyone permitted may
  remove one.
