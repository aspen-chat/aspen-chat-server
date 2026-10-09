# Polls

A poll is a record of its own, shown in a message and kept current by the event stream.

## Where it lives

| Part | File |
| --- | --- |
| The poll in a message | `src/features/messages/PollCard.tsx` |
| Making a poll | `src/features/messages/CreatePollDialog.tsx` |
| The closing announcement | `src/features/messages/PollClosedNotice.tsx` |
| Outcome text, answer indexes | `src/features/messages/poll.ts` |

## Messages and records

- A message of kind `poll` names its poll in its `poll` field and carries no text.
- A message of kind `pollClosed` is the announcement the server posts when the deadline passes.
- Each option is `{ label, emoji? }`. The emoji is chosen in the dialog from the same lazily
  loaded picker reactions use. The server validates it as a single emoji.
- The outcome text of the announcement is composed on the client from the final tally
  (`src/features/messages/poll.ts`), so it is localized like everything else.

## Votes and the tally

- The tally lives in the poll record's `results`. The server republishes it as an update event
  on every vote.
- A vote only records the caller's own choice locally (`store.setMyVote`). The numbers come from
  the stream.
- Each answer's result names only its first five voters. `ChoiceRow` says how many more there
  are, and `VotersDialog` lists everyone who voted for it a page at a time
  (`AspenSync.loadVoters`).
- Message reads sideload `polls` with the caller's `pollVotes`. This is the only way to learn
  one's own vote on an anonymous poll.
- `usePoll` fetches a poll the window did not bring.

## Closing

The card offers Close poll while the poll is open, to:

- the poll's creator;
- a holder of Manage messages.

Closing calls `AspenSync.closePoll`. The closing arrives as the poll's update and the
announcement, as a deadline's does.

## Write-ins

A poll whose creator allowed write-ins lists its `writeIns` after its `options`.

- Options and write-ins share one index space that votes use. Write-in `i` is answer
  `options.length + i` (`pollChoices`, `choiceAt`).
- A removed write-in stays as `null`, so no index moves.
- Each write-in notes who wrote it in. On an anonymous poll it notes only that it was written in.
- Reads with votes also sideload `ownWriteIns`, the caller's own standing write-ins
  (`store.myWriteIns`). They decide:
  - whether the card offers the field for one (one each);
  - whether it offers a remove control, which goes to the writer and the poll's creator.
- Writing in an answer the poll already has votes for it instead. `sync.writeIn` resolves to the
  answer voted for.
- A poll's `update` event that nulls an answer also drops the caller's vote and write-in on it,
  since anyone permitted may remove one.
