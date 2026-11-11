# Custom emoji

Custom emoji are a community's own. Each is a name and a picture that is an icon. They appear in
message text and in reactions.

## Where it lives

| Part                           | Code                                                                                     |
| ------------------------------ | ---------------------------------------------------------------------------------------- |
| Records                        | `CustomEmoji`, from the `emoji` sideload of the community reads and `customEmoji` events |
| Store                          | `RecordStore.customEmoji(communityId)`, topic `emoji:<communityId>`, `useCustomEmoji`    |
| Reference parsing and encoding | `src/features/emoji/customEmoji.ts`                                                      |
| Drawing                        | `remarkCustomEmoji`, `CustomEmojiGlyph`                                                  |
| Completion                     | `useEmojiCompletion` (`src/features/emoji`)                                              |
| Keyboard                       | `EmojiKeyboard`, below the message box and the editor                                    |
| Picker data                    | `emojiData.ts`                                                                           |
| Settings tab                   | `EmojiPanel`                                                                             |

A read that sideloads `emoji` lists them whole for the communities it was of (the bootstrap's listed
communities, a community read's own), so the store drops what it held for those and the read lacks,
a community whose last emoji was deleted among them, and drops a community's emoji with the
community.

## In message text

A message's text names a custom emoji as `<:id>`.

`remarkCustomEmoji` turns the reference into `CustomEmojiGlyph`:

- the picture, at the text's size
- named `:name:` for assistive technology
- a marked placeholder for an id the community's list does not hold

## In the message box

The message box shows a custom emoji as `:name:`.

- One picked from the completion or the keyboard is a pick ([Picks](tagging.md#picks)): it is sent
  as the reference of the emoji that was picked, whatever else shares or resembles its name.
- One typed by hand is looked up by name on send (`encodeCustomEmoji`): as written first, then
  ignoring case when exactly one emoji has the name so.
- For an edit, `decodePicked` turns each reference back into `:name:`, a pick of the same emoji.
- `decodeCustomEmoji` reads references as names where a line of a message is shown.

### Completion

`useEmojiCompletion` completes a colon word with:

- the community's emoji
- every unicode emoji, by its names

A unicode pick writes the glyph. A custom pick writes `:name:` and remembers which emoji it is.

### The keyboard

The smiley left of Send docks the picker below the box (`EmojiKeyboard`; see
[The emoji keyboard](composer-and-drafts.md#the-emoji-keyboard)). The community's emoji are its own
section. A pick is written as the completion writes one, through `useEmojiCompletion`'s `insert`,
which turns the picker's reference into `:name:`, a pick of that emoji.

### Emoji data

Unicode names come from the picker's own data in the reader's language (`emojiData.ts`):

- the catalogue the reader chose, or
- for `automatic`, the first of the browser's languages the picker has

One file is loaded, on first use. The picker takes the same file through `emojiData`, with its
custom section renamed to the app's words.

## In reactions

A reaction's key may be the same `<:id>` reference. So these take the community:

- `ReactionChips`
- the reactions dialog
- the picker (`customEmojis`, with the pictures from `useIcons`)

A DM has no community, so it passes `null`.

When an emoji's deletion event comes, the store drops the reactions made with it. **Why:** the
server's cascade announces them no further.

## Managing them

Community settings' Emoji tab (`EmojiPanel`) lists them. With Manage custom emoji, it:

- adds one. `prepareEmojiPicture` scales a still picture to 128 pixels and refuses a larger GIF.
- renames one
- removes one
