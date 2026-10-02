# Custom emoji

- Custom emoji are a community's own (`CustomEmoji` records, from the `emoji` sideload of the
  community reads and `customEmoji` events; `RecordStore.customEmoji(communityId)`, topic
  `emoji:<communityId>`, `useCustomEmoji`), each a name and a picture that is an icon. A
  message's text names one as `<:id>` (`src/features/emoji/customEmoji.ts`, which
  `remarkCustomEmoji` turns into `CustomEmojiGlyph`: the picture at the text's size, named
  `:name:` for assistive technology, or a marked placeholder for an id the community's list
  does not hold); the message box shows `:name:` and `useEmojiCompletion` (`src/features/emoji`)
  completes a colon word with the community's emoji and every unicode emoji by its names (the
  picker's own data in the reader's language, `emojiData.ts`: the catalogue they chose, or for
  `automatic` the first of the browser's languages the picker has, one file loaded on first
  use; the picker takes the same file through `emojiData`, its custom section renamed to the
  app's words), a unicode pick writing the glyph and a custom pick `:name:`, which `encodeCustomEmoji` turns into the reference as the message is sent and
  `decodeCustomEmoji` back for an edit. A reaction's key may be the same reference, so
  `ReactionChips`, the reactions dialog, and the picker (`customEmojis`, with the pictures from
  `useIcons`) take the community, and a DM, which has none, passes `null`. The store drops the
  reactions made with an emoji when its deletion event comes, since the server's cascade
  announces them no further. Community settings' Emoji tab (`EmojiPanel`) lists them and, with
  Manage custom emoji, adds one (`prepareEmojiPicture` scales a still picture to 128 pixels and
  refuses a larger GIF), renames, and removes.
