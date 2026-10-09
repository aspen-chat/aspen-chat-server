# Blocking

A user may block another. The app hides the blocked person's messages, reactions, and voice, and the server counts without them.

For blocks across deployments, see [Deployments and federation](deployments.md#blocks-across-deployments).

## State

| What | Store | Topic | Hook |
| --- | --- | --- | --- |
| Whether one user is blocked | `RecordStore.blocked` | `block:<userId>` | `useBlocked` |
| The list of blocked users | `RecordStore.blockedUsers` | `blocks` | `useBlockedUsers` |

- Read at bootstrap from `GET /users/@me/blocks`, with the blocked users sideloaded.
- Kept by `userBlockChanged` events.
- `AspenSync.blockUser` and `unblockUser` apply their answer at once.

## When a block changes

A change from either source (an answer or an event):

1. Sets the user's call gain (`#userGain`). A blocked user is silent, whatever their volume and "mute for me" say.
2. Reads again what the server counts without them:
   - every read state
   - the reactions of each held message window, one `around` read per window

The event stream is filtered the same way. A blocked user's reactions are never counted, and their messages never move `lastMessage`.

## Messages

- `MessageList` collapses each run of consecutive messages by blocked people into one `BlockedRun` row ("2 blocked messages", Show/Hide).
- The row stands for the run's last message when anchoring the view and marking it read.
- A linked message inside a run opens the run.
- A thread's starter collapses the same way.
- A pin by someone blocked is not quoted.

## DMs

In a one-to-one DM with someone the caller blocked, the resolver grants only `viewChannel` (`blockedDmPeer`). Every write control goes, and the composer gives way to a note offering to unblock.

A block the other person made is known only from the server's `blocked` refusal.

## Calls

- A blocked person is marked on their tile and row.
- Their screens are left out of `VoiceScreen`.
- Their menu offers no volume.

## Where a user blocks and unblocks

- The profile card blocks, after a confirming second press that says what blocking does, and unblocks.
- The member list marks blocked people.
- Settings lists them with Unblock.
