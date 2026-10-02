# The message box and drafts

- The message box (`Composer`) sits level with its buttons, all 42px tall; Send is an icon
  (paper plane), so the box keeps the room. Enter sends on a computer; on a touch-only device
  (`TOUCH_ONLY`), whose keyboard has no Shift+Enter, Enter writes a new line there and in a
  message being edited, and the button sends. On a narrow screen
  its other controls share one + button whose menu offers attaching a file and making a poll
  (`CreatePollModal`, which the wide screen's own poll button opens too). Its placeholder is
  drawn by the box itself on one line, cut short with an ellipsis, since the textarea's own
  would wrap and grow the box; the textarea carries it as `aria-placeholder`. When the list
  shrinks, as when a phone's keyboard opens, it keeps its bottom edge where it was (pinned to the
  newest message, or moved down by what it lost), so what is being answered stays in view; the
  web app asks for this with `interactive-widget=resizes-content` in its viewport. "Jump to
  latest" answers at once: the pill keeps its own state, so a press repaints it alone, saying
  the newest are on their way, before the list moves to the end of what it holds, and `AspenSync.loadLatest` follows any page already being read rather than
  settling for it.
- Drafts (`src/features/messages/drafts.ts`): what is written in a message box and not sent
  (text, picked tags, files already uploaded, and a thread's echo choice) waits in its channel
  on this device, per account, through going elsewhere, a notification opened, and a reload,
  and goes once sent. The Composer is keyed by its channel, so each box reads its own; it notes
  its draft in memory on every change (`noteDraft`), which `readDraft` reads first, and keeps it
  in `localStorage` after a short pause and at once when it unmounts or the page is hidden.
  `e2e/drafts.spec.ts` checks the notification path.
