# Reactions

- Reactions are store state per message (`RecordStore.reactions`, topic `reactions:<messageId>`):
  per emoji, the count, whether the caller reacted, and the first few to react, installed from
  the `reactions` sideload of every message read (`setReactions`) and kept current by `react`
  events. The caller's own reaction is applied from the request's answer and again from its
  event, which changes nothing the second time. When one of the named few leaves, `AspenSync`
  reads the message's summary again. `ReactionChips` (`src/features/messages/Reactions.tsx`)
  shows the most popular first (ties in the order first used), at most twenty, then a `+N` chip
  and an add chip that opens the picker at once; each chip's tooltip names the first four and
  counts the rest; emoji are drawn half again as large as the count, and the reader's own
  reaction is marked by a darker outline in the chips' neutral colours. `ReactionsDialog`,
  opened from the `+N` chip, the message's "View reactions" action, or a right click or long
  press on a chip (opening on that chip's emoji; the long press toggles nothing), lists every emoji and, for the chosen one, everyone who reacted, a page at
  a time (`AspenSync.loadReactors`).
- Quick reactions sit atop a touch screen's message actions (`MessageActionSheet`): five
  emoji and an unnamed Add a reaction button, which opens the full picker in the sheet's
  place. The five are the reader's most used where the message is (`useFrequentEmoji`,
  `AspenSync.loadFrequentEmoji`, `GET /users/@me/frequent-emoji` with the message's
  community, so its own custom emoji may be among them): those used in the past 90 days, then
  those used most overall, then the defaults 😄 ❤️ 👍 👎 😮 (`quickReactions`,
  `DEFAULT_QUICK_REACTIONS`), each once, leaving out a custom emoji whose record is not held.
  The store keeps them per community (`RecordStore.frequentEmoji`, topic
  `frequentEmoji:<community>`, `""` for a DM) and marks every list stale when one of the
  reader's own `react` events arrives, from this device or another, or the store resyncs; a
  stale list is still shown while it is read again, and one whose read started before the
  reader's latest reaction (`reactionChanges`) is kept stale. They are read as a long press
  begins, so they are there by the time the sheet is; until they are, the row holds
  skeletons, and a read that fails offers the defaults. An emoji the reader already reacted
  with is drawn pressed, with the chips' darker outline, and pressing it takes the reaction
  back; any press closes the sheet, and a refusal (a message at its 50 emoji) is said in a
  toast.
