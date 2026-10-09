# Tagging

Tags are `<@user>`, `<@&role>`, and `@everyone`.

## Where it lives

| Part | Where |
| --- | --- |
| Drawing tags | `remarkMentions`, `Mention` (`src/features/messages`) |
| Completing tags in a message box | `useTagging` (`src/features/mentions`), `SuggestionList` |
| Encoding and decoding | `encodeTags`, `decodeTags` |
| Unread tag counts | `MentionBadge`, `placeMentions`, `RecordStore.mentionsMe` |
| End-to-end test | `e2e/tagging.spec.ts` |

## Drawing tags in messages

- A tag the message's `mentions` says counts is a chip. A person's chip opens their card.
- Any other tag is plain text naming whom it would have tagged, as the server leaves it.
- A message that tags the reader is marked with `data-mentions-me`. `RecordStore.mentionsMe`
  decides: by name, by a role they hold, or by everyone.

## Completing tags in a message box

`useTagging` offers people, roles, and everyone as `@` is typed.

- Each is offered only with its permission in the channel.
- People come from the member sample. For those the server lets search a large community, they
  also come from `useMemberSearch`.
- It works from the keyboard: arrows, Enter or Tab, Escape.
- It uses `aria-activedescendant` and a polite status. React Aria's ComboBox cannot complete at a
  `TextArea`'s caret.
- Completions share `SuggestionList`. The box renders one status, saying what the active
  completion's `announcement` says.
- A pick shows as `@username` or `@Role` and is sent as its tag (`encodeTags`).
- Editing reads tags back (`decodeTags`).

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

`e2e/tagging.spec.ts` shows a screen reader user tagging from the keyboard alone. It checks the
box and its suggestions with axe (`@axe-core/playwright`).
