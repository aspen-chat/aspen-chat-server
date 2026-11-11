# Tagging

Tags are `<@user>`, `<@&role>`, and `@everyone`.

## Where it lives

| Part                             | Where                                                     |
| -------------------------------- | --------------------------------------------------------- |
| Drawing tags                     | `remarkMentions`, `Mention` (`src/features/messages`)     |
| Completing tags in a message box | `useTagging` (`src/features/mentions`), `SuggestionList`  |
| Encoding and decoding            | `picks.ts` (`src/features/messages`), `decodeTags`        |
| Unread tag counts                | `MentionBadge`, `placeMentions`, `RecordStore.mentionsMe` |
| End-to-end test                  | `e2e/tagging.spec.ts`                                     |

## Drawing tags in messages

- A tag the message's `mentions` says counts is a chip. A person's chip opens their card.
- Any other tag is plain text naming whom it would have tagged, as the server leaves it.
- A message that tags the reader is marked with `data-mentions-me`. `RecordStore.mentionsMe`
  decides: by name, by a role they hold, or by everyone.

## Completing tags in a message box

`useTagging` offers people, roles, and everyone as `@` is typed.

- Each is offered only with its permission in the channel.
- People come from the member sample. For those the server lets search a large community, they also
  come from `useMemberSearch`.
- It works from the keyboard: arrows, Enter or Tab, Escape.
- It uses `aria-activedescendant` and a polite status. React Aria's ComboBox cannot complete at a
  `TextArea`'s caret.
- Completions share `SuggestionList`. The box renders one status, saying what the active
  completion's `announcement` says.
- A pick shows as `@username` or `@Role` and is sent as its tag.
- Editing reads tags back, each a pick again (`decodePicked`).
- A line of a message shown elsewhere (a reply's quote, a notification) reads tags as names
  (`decodeTags`).

## Picks

`src/features/messages/picks.ts` is how a message box knows what was picked: tags, custom emoji, and
a command's arguments alike.

- A pick is a range of the box's text and what it stands for (`Picked`: `start`, `end`, `kind`,
  `id`). The text shows the plain name.
- **Why by range:** names repeat. A foreign account may share a local one's username, two roles may
  share a name, and the same word may be typed by hand in an address or a code span. Only the place
  says which text was picked, and what it is.
- `usePickedText` holds the text and its picks for a box. Completions write through its `splice`;
  the box's own edits go through `type` (`retype`).
- As the text changes, picks before the change stay, picks after it move, and a pick the change
  reaches into is plain text from then on. Text written right beside a pick leaves it a pick
  (`@kateさん`).
- `retype` is given the box's caret, which says where an edit was when the texts alone do not (a
  letter typed beside the same letter).
- Sending replaces exactly the picked ranges with their references (`encodePicked`). The same name
  anywhere else in the text is sent as written.
- A draft keeps its picks with its text ([Drafts](composer-and-drafts.md#drafts)).

## Unread tag counts

Each read state's `mentions` is the reader's unread tags.

- The store adds a live message that tags them.
- The store clears the count when they post or read to the newest.
- `AspenSync` reads the state again when only the server can say:
  - reading partway;
  - another device reading;
  - an unread message's tags edited, or the message deleted.

`MentionBadge` shows the count:

- on channel and DM rows;
- summed (`placeMentions`) on the rail;
- muted channels included;
- with each row saying it in its accessible name.

## Testing

`e2e/tagging.spec.ts` shows a screen reader user tagging from the keyboard alone. It checks the box
and its suggestions with axe (`@axe-core/playwright`).
