# Reactions

## Where it lives

| Part | Where |
| --- | --- |
| Store state | `RecordStore.reactions` (topic `reactions:<messageId>`), `setReactions` |
| Chips | `ReactionChips` (`src/features/messages/Reactions.tsx`) |
| Everyone who reacted | `ReactionsDialog`, `AspenSync.loadReactors` |
| Quick reactions | `MessageActionSheet`, `useFrequentEmoji`, `AspenSync.loadFrequentEmoji`, `quickReactions` |

## Store state

Reactions are store state per message. Per emoji the store holds:

- the count;
- whether the caller reacted;
- the first few to react.

They are installed from the `reactions` sideload of every message read (`setReactions`) and kept
current by `react` events.

- The caller's own reaction is applied from the request's answer and again from its event. The
  second time changes nothing.
- When one of the named few leaves, `AspenSync` reads the message's summary again.

## Chips

`ReactionChips` shows:

- the most popular first, ties in the order first used;
- at most twenty;
- then a `+N` chip;
- then an add chip that opens the picker at once.

Drawing:

- Each chip's tooltip names the first four and counts the rest.
- Emoji are drawn half again as large as the count.
- The reader's own reaction is marked by a darker outline in the chips' neutral colours.

## Everyone who reacted

`ReactionsDialog` opens from:

- the `+N` chip;
- the message's "View reactions" action;
- a right click or long press on a chip. It opens on that chip's emoji, and the long press
  toggles nothing.

It lists every emoji and, for the chosen one, everyone who reacted, a page at a time
(`AspenSync.loadReactors`).

## Quick reactions

Quick reactions sit atop a touch screen's message actions (`MessageActionSheet`): five emoji and
an unnamed Add a reaction button. The button opens the full picker in the sheet's place.

### Which five

The five are the reader's most used where the message is (`useFrequentEmoji`,
`AspenSync.loadFrequentEmoji`). They come from `GET /users/@me/frequent-emoji` with the message's
community, so its own custom emoji may be among them. In order, each once:

1. those used in the past 90 days;
2. those used most overall;
3. the defaults 😄 ❤️ 👍 👎 😮 (`quickReactions`, `DEFAULT_QUICK_REACTIONS`).

A custom emoji whose record is not held is left out.

### Caching

- The store keeps them per community: `RecordStore.frequentEmoji`, topic
  `frequentEmoji:<community>`, `""` for a DM.
- It marks every list stale when one of the reader's own `react` events arrives, from this device
  or another, or when the store resyncs.
- A stale list is still shown while it is read again.
- A list whose read started before the reader's latest reaction (`reactionChanges`) is kept stale.

### Loading

- They are read as a long press begins, so they are there by the time the sheet is.
- Until they are, the row holds skeletons.
- A read that fails offers the defaults.

### Pressing one

- An emoji the reader already reacted with is drawn pressed, with the chips' darker outline.
  Pressing it takes the reaction back.
- Any press closes the sheet.
- A refusal (a message at its 50 emoji) is said in a toast.
