# Tagging

- Tags (`<@user>`, `<@&role>`, `@everyone`) are drawn by `remarkMentions` and `Mention`
  (`src/features/messages`): a tag the message's `mentions` says counts is a chip, a person's
  opening their card; any other is plain text naming whom it would have tagged, as the server
  leaves it. A message that tags the reader (`RecordStore.mentionsMe`: by name, a role they
  hold, or everyone) is marked (`data-mentions-me`). In a message box, `useTagging`
  (`src/features/mentions`) offers people, roles, and everyone as `@` is typed, each only with
  its permission in the channel (people from the member sample and, for those the server lets
  search a large community, `useMemberSearch`), from the keyboard (arrows, Enter or Tab, Escape) with
  `aria-activedescendant` and a polite status, since React Aria's ComboBox cannot complete at a
  `TextArea`'s caret. Completions share `SuggestionList`, and the box renders one status saying
  what the active completion's `announcement` says. A pick shows as `@username` or `@Role` and is sent as its tag
  (`encodeTags`); editing reads tags back (`decodeTags`). Each read state's `mentions` is the
  unread tags of the reader: the store adds a live message that tags them, clears it when they
  post or read to the newest, and `AspenSync` reads the state again when only the server can
  say (reading partway, another device reading, an unread message's tags edited or it
  deleted). `MentionBadge` shows the count on channel and DM rows and, summed
  (`placeMentions`), on the rail, muted channels included, each row saying it in its
  accessible name. `e2e/tagging.spec.ts` shows a screen reader user tagging from the keyboard
  alone and checks the box and its suggestions with axe (`@axe-core/playwright`).
