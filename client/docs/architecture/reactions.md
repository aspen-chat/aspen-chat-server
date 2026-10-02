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
