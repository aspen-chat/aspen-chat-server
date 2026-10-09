# The message box and drafts

The message box (`Composer`) is where a user writes and sends messages. This page covers its layout, the typing line above it, the length limit, messages held for their previews, and drafts.

## Where it lives

| Part | Code |
| --- | --- |
| Message box | `Composer` |
| Typing line | `TypingIndicator`, `TypingDots`, `typingSentence` |
| Length counting | `messageLength.ts`, `MessageLengthNote` |
| Held messages | `HeldMessages.tsx` |
| Drafts | `src/features/messages/drafts.ts` |

## Layout

- The box sits level with its buttons, all 42px tall.
- Send is an icon (a paper plane), so the box keeps the room.
- On a narrow screen, the other controls share one + button. Its menu offers attaching a file and making a poll (`CreatePollModal`, which the wide screen's own poll button opens too).

### Enter

| Device | Enter | Sends |
| --- | --- | --- |
| A computer | Sends | Enter or the button |
| A touch-only device (`TOUCH_ONLY`) | Writes a new line, in the box and in a message being edited | The button |

**Why:** a touch-only device's keyboard has no Shift+Enter.

### Placeholder

The box draws its placeholder itself, on one line, cut short with an ellipsis. The textarea carries it as `aria-placeholder`. **Why:** the textarea's own placeholder would wrap and grow the box.

### Width

The box grows with what is written, up to its row's width and no further:

- It may shrink below its content (`min-w-0`).
- A word too long for the line (a link, say) breaks anywhere.
- The buttons beside it never shrink, so Send stays on screen however wide the text.

### When the list shrinks

When the message list shrinks, as when a phone's keyboard opens, it keeps its bottom edge where it was. It stays pinned to the newest message, or moves down by what it lost, so what is being answered stays in view. The web app asks for this with `interactive-widget=resizes-content` in its viewport.

### Jump to latest

"Jump to latest" answers at once:

1. The pill keeps its own state, so a press repaints the pill alone, saying the newest are on their way.
2. The list then moves to the end of what it holds.
3. `AspenSync.loadLatest` follows any page already being read rather than settling for it.

## Who is typing

### The line

`TypingIndicator` shows who else is typing, on a line of its own just above the message box. `Composer` puts it there whether or not the caller may write.

- It is a sunken band ruled off from the messages above it (`bg-surface-sunken`, `border-t`). Empty, it reads as part of the box rather than a gap in the list.
- It is in message text at the reader's message text size (`message-text`).
- It is kept one line tall (`h-[1lh]`) whether or not anyone is typing, so nothing moves when someone starts or stops.
- It is not a live region. **Why:** a live region would talk over the conversation at every start and stop.

### The words

| Typing | Text |
| --- | --- |
| One person | "*name* is typing…" |
| Two or three | Names joined as the language lists things, then "are typing…" |
| Four or more | "Several people are typing…" |

- Names come through `PersonName` in the channel's community, joined by `typingSentence`.
- When the line runs out of room, each name is shortened, never the words around them, so it always says what is happening.

### The dots

`TypingDots` draws three dots beside the words, in the accent's colour. Each is lit and dimmed again in turn, from the start of the line.

- The animation is `typing-dot` in `styles.css`, timed by the motion tokens, so it follows the animation speed.
- It is a fade alone, so it goes on where motion is reduced.

### Telling the server

- As its user writes, the box calls `AspenSync.noteTyping`, at most every `TYPING_REFRESH_MS`. It never does while `TYPING_NOTICES` is off.
- It calls `stopTyping` when the box empties, a message is sent, or the box goes.
- A draft the box opens with is not typing.

### Others typing

The stream tells of others only in channels the app says it shows typing for:

1. `useTypers` registers its channel with `AspenSync.watchTyping` while it is used.
2. Sync sends the open set as a `viewing` frame on every change and every `ready`.

What it tells is kept in `RecordStore.typers` (topic `typing:<channelId>`):

- The user, and whoever they block on any deployment, are left out.
- Each person is kept for `TYPING_EXPIRY_MS` after their last word.
- They are gone at once when they stop or a message of theirs arrives.
- All of it is forgotten when the stream's connection drops.

For the server's side, see `docs/architecture/typing.md` at the repository's root.

## Message length

| Limit | Value | Where |
| --- | --- | --- |
| Text length | `MESSAGE_MAX_CHARS` (10,000) characters, the server's limit | `messageLength.ts` |
| Nesting of quotes and lists | 32 levels | `app::message::MAX_NESTING` on the server; `markdownLimits.ts` renders beyond it as plain text |

### Counting length

Length is counted as the server counts it: Unicode scalar values, in the text as sent, with tags and custom emoji written out as references.

- Within the last thousand characters, the message box and the editor count beneath themselves (`MessageLengthNote`).
- Over the limit, they say by how much and what to do. Send and Save wait until the text is shortened.

### Nesting

The server refuses text nesting quotes and lists more than 32 levels deep, with `messageNestingTooDeep`. The message box shows it as it shows any refusal.

## Messages held for their previews

`AspenSync.sendMessage` sends with `mayHold`. A message whose picture or video is still having its preview made is held by the server for up to twenty seconds from the upload.

1. The server answers `202` with the held message.
2. The store keeps it (`heldMessages`, topic `held:<place>`; see [Where a held message waits](#where-a-held-message-waits)).
3. The message box of that place shows it above itself, waiting (`HeldMessages.tsx`).
4. It ends with one of:
   - `heldMessagePosted`: the message itself arrives in the list by its own event.
   - `heldMessageFailed`: the box says why, and offers to send it again (`AspenSync.sendHeldAgain`) or let it go.

- The box clears as soon as the server has the message, held or posted.
- Held messages are read at bootstrap (`GET /users/@me/held-messages`).
- A dropped held message stays, on this device, until it is sent again or let go.

### Where a held message waits

A held message's place (`heldPlace`) is one of:

| Place | When |
| --- | --- |
| The channel | Usually |
| `thread-of:<message>` | A dropped first reply whose thread went with it. `RecordStore` moves it there as the thread is removed |

`AspenSync.sendHeldAgain` sends such a first reply with `replyInThread`, making the thread anew (see [Threads, DMs, and the system account](threads-and-dms.md#starting-a-thread)).

## Drafts

`src/features/messages/drafts.ts` keeps what is written in a message box and not sent:

- text
- picked tags
- files already uploaded
- a thread's echo choice

A draft waits in its channel, on this device, per account. It survives going elsewhere, a notification opened, and a reload. It goes once sent.

### How drafts are kept

1. The Composer is keyed by its channel, so each box reads its own draft.
2. On every change it notes the draft in memory (`noteDraft`). `readDraft` reads memory first.
3. It keeps the draft in `localStorage` after a short pause, and at once when it unmounts or the page is hidden.

`e2e/drafts.spec.ts` checks the notification path.
