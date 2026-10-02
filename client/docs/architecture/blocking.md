# Blocking

- Blocks are store state (`RecordStore.blocked`, topic `block:<userId>`, and `blockedUsers`,
  topic `blocks`; `useBlocked`, `useBlockedUsers`), read at bootstrap from `GET
/users/@me/blocks` with the blocked users sideloaded and kept by `userBlockChanged` events.
  `AspenSync.blockUser` and `unblockUser` apply their answer at once; a change from either
  source sets the user's call gain (`#userGain`: silent while blocked, whatever their volume
  and "mute for me" say) and reads again what the server counts without them: every read
  state, and the reactions of each held message window, one `around` read per window. The
  stream is filtered the same way, so a blocked user's reactions are never counted and their
  messages never move `lastMessage`. `MessageList` collapses each run of consecutive messages
  by blocked people into one `BlockedRun` row ("2 blocked messages", Show/Hide), which stands
  for the run's last message when anchoring the view and marking it read; a linked message
  inside a run opens it, and a thread's starter collapses the same way. A pin by someone
  blocked is not quoted. In a one-to-one DM with someone the caller blocked the resolver
  grants only `viewChannel` (`blockedDmPeer`), so every write control goes and the composer
  gives way to a note offering to unblock; a block the other person made is known only from
  the server's `blocked` refusal. In calls a blocked person is marked on their tile and row,
  their screens are left out of `VoiceScreen`, and their menu offers no volume. The profile
  card blocks (after a confirming second press that says what blocking does) and unblocks,
  the member list marks blocked people, and Settings lists them with Unblock.
