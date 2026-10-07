# The message box and drafts

- The message box (`Composer`) sits level with its buttons, all 42px tall; Send is an icon
  (paper plane), so the box keeps the room. Enter sends on a computer; on a touch-only device
  (`TOUCH_ONLY`), whose keyboard has no Shift+Enter, Enter writes a new line there and in a
  message being edited, and the button sends. On a narrow screen
  its other controls share one + button whose menu offers attaching a file and making a poll
  (`CreatePollModal`, which the wide screen's own poll button opens too). Its placeholder is
  drawn by the box itself on one line, cut short with an ellipsis, since the textarea's own
  would wrap and grow the box; the textarea carries it as `aria-placeholder`. The box grows with
  what is written up to its row's width and no further: it may shrink below its content
  (`min-w-0`), a word too long for the line (a link, say) breaks anywhere, and the buttons
  beside it never shrink, so Send stays on screen however wide the text. When the list
  shrinks, as when a phone's keyboard opens, it keeps its bottom edge where it was (pinned to the
  newest message, or moved down by what it lost), so what is being answered stays in view; the
  web app asks for this with `interactive-widget=resizes-content` in its viewport. "Jump to
  latest" answers at once: the pill keeps its own state, so a press repaints it alone, saying
  the newest are on their way, before the list moves to the end of what it holds, and `AspenSync.loadLatest` follows any page already being read rather than
  settling for it.
- Who else is typing shows on a line of its own just above the message box (`TypingIndicator`,
  which `Composer` puts there whether or not the caller may write), a sunken band ruled off from
  the messages above it (`bg-surface-sunken`, `border-t`) so that, empty, it reads as part of the
  box rather than a gap in the list, in message text at the
  reader's message text size (`message-text`), kept one line tall (`h-[1lh]`) whether or not
  anyone is typing, so nothing moves when someone starts or stops. Up to three people are named,
  through `PersonName` in the channel's community and joined as the language lists things
  (`typingSentence`), "is typing…" for one and "are typing…" for more; four or more are
  "Several people are typing…". When the line runs out of room each name is shortened, never the
  words around them, so it always says what is happening. Beside the words,
  `TypingDots` draws three dots in the accent's colour, each lit and dimmed again in turn from
  the start of the line (`typing-dot` in `styles.css`, timed by the motion tokens, so it follows
  the animation speed; a fade alone, so it goes on where motion is reduced).
  It is not a live region, which would talk over the conversation at every start and stop. The
  box tells the server as its user writes (`AspenSync.noteTyping`, at most every
  `TYPING_REFRESH_MS`, and never while `TYPING_NOTICES` is off), and that they stopped
  (`stopTyping`) when the box empties, a message is sent, or the box goes; a draft it opens
  with is not typing. What the stream tells of others is kept in `RecordStore.typers` (topic
  `typing:<channelId>`, leaving out the user and whoever they block on any deployment), each
  for `TYPING_EXPIRY_MS` after the last word, gone at once when they stop or a message of
  theirs arrives, and all of it forgotten when the stream's connection drops. See
  `docs/architecture/typing.md` at the repository's root for the server's side.
- A message's text may be at most `MESSAGE_MAX_CHARS` (10,000) characters, the server's limit,
  counted as the server counts them (Unicode scalar values) in the text as sent, its tags and
  custom emoji written out as references (`messageLength.ts`). Within its last thousand, the
  message box and the editor count beneath themselves (`MessageLengthNote`); over it, they say
  by how much and what to do, and Send and Save wait until it is shortened.
  The server also refuses text nesting quotes and lists more than 32 levels deep
  (`app::message::MAX_NESTING`, the depth `markdownLimits.ts` renders as plain text beyond), with
  `messageNestingTooDeep`, which the message box shows as it shows any refusal.
- Messages held for their previews (`HeldMessages.tsx`): `AspenSync.sendMessage` sends with
  `mayHold`, so a message whose picture or video is still having its preview made is held by
  the server for up to twenty seconds from the upload and answered `202` with the held message,
  which the store keeps (`heldMessages`, topic `held:<channelId>`) and the message box shows
  above itself, waiting, until `heldMessagePosted` (the message itself arriving in the list by
  its own event) or `heldMessageFailed`, when it says why and offers to send it again
  (`AspenSync.sendHeldAgain`) or let it go. The box clears as soon as the server has it, held or
  posted. Held messages are read at bootstrap (`GET /users/@me/held-messages`), and a dropped one
  stays, on this device, until it is sent again or let go.
- Drafts (`src/features/messages/drafts.ts`): what is written in a message box and not sent
  (text, picked tags, files already uploaded, and a thread's echo choice) waits in its channel
  on this device, per account, through going elsewhere, a notification opened, and a reload,
  and goes once sent. The Composer is keyed by its channel, so each box reads its own; it notes
  its draft in memory on every change (`noteDraft`), which `readDraft` reads first, and keeps it
  in `localStorage` after a short pause and at once when it unmounts or the page is hidden.
  `e2e/drafts.spec.ts` checks the notification path.
