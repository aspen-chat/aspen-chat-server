# Custom emoji

A community keeps its own emoji: a name and a picture. Each is identified by its community and its own UUIDv7, and the UI calls it by its name alone.

| Part | Where |
| --- | --- |
| Logic | `app::custom_emoji` |
| Handlers | `api::custom_emoji` |
| Table | `custom_emoji` |
| Reactions | `app::react::stored_key`, column `react.custom_emoji` |
| Sideload | `include=emoji` on community reads, as `included.customEmoji` |
| Events | Each change is an event on the community's subject |

## The picture

The picture is an icon the one adding it uploaded first through the icon flow: `POST /icons` and its confirmation. Both refuse anyone else's upload (`app::icon::require_own`).

| Limit | Value | Where checked |
| --- | --- | --- |
| Formats | PNG, JPEG, WebP, or GIF | |
| Emoji picture size | At most 256 KiB (`custom_emoji::MAX_BYTES`) | The server, against the stored picture, when the emoji is added |
| Any icon's size | At most 8 MiB (`icon::MAX_BYTES`) | |
| Any icon's pixels | 4096 by 4096 pixels' worth (`icon::MAX_PIXELS`) | Read from its header when it is confirmed. Link previews' pictures and copied foreign avatars also meet this cap |
| Emoji display size | 128 by 128 | The client scales a still picture down to it, and refuses a larger GIF |
| Emoji pixels | At most 512 by 512 pixels' worth (`custom_emoji::MAX_PIXELS`) | The server |

- The picture must be in use nowhere else when the emoji is added (`customEmojiIconUsed`), so an emoji never takes a profile's or a report's picture with it.
- Deleting the emoji removes its picture, unless something has taken that picture up since (`icon::delete_if_unused`, from the [`purgeCustomEmoji`](jobs/kinds.md#purgecustomemoji) job).

## The name

- 2 to 32 characters, of any script.
- No whitespace or colons.
- Unique within the community, ignoring case (`customEmojiNameTaken`).

## Endpoints

| Endpoint | Does | Permission |
| --- | --- | --- |
| `GET /communities/{community}/emoji` | Lists them | |
| `POST /communities/{community}/emoji` | Adds one | Manage custom emoji |
| `PATCH /emoji/{emoji}` | Renames one | Manage custom emoji |
| `DELETE /emoji/{emoji}` | Removes it with its picture | Manage custom emoji |

Manage custom emoji is held by the Moderator and Admin templates, not by the everyone role. The migration gave it to every role holding Manage messages.

## Limit per community

A community holds at most the deployment setting `custom_emoji_limit` (1000; see [Administration](administration/index.md)). Lowered, it keeps what a community already holds.

## In messages

- A message's text names one as `<:id>`, by the id alone, so a rename changes no message.
- A reader resolves it from the community's list, rendering an unknown id as a marked placeholder.
- The message box shows `:name:` and encodes it as it sends.
- The server never parses `<:id>` in text. An id from elsewhere resolves to nothing where it is read, which is all the rule needs.

## In reactions

A reaction's emoji may be that same reference (`app::react::stored_key`), stored with `react.custom_emoji` naming the emoji.

| Where the message is | Result |
| --- | --- |
| A channel of the emoji's community | Allowed |
| A DM (which has no community) | `customEmojiNotHere` |
| Another community's channel | `customEmojiUnknown` |

Deleting an emoji takes its reactions with it by cascade. Clients apply that on the emoji's deletion event.
