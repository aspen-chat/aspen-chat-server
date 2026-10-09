# Message links

A message whose text links to messages of this deployment shows each beneath it, as a link preview shows a page.

| Part | Where |
| --- | --- |
| Logic | `app::message_link` |
| Finding links | `app::link_preview::extract_urls`, the same walk over the Markdown as link previews, skipping code |
| Stored | `message.linked_messages`; on the record, `linkedMessages` |
| Sideload | `include=linked`, as `included.linkedMessages` |

## What counts as a link

A link is any URL under `/communities/` or `/dms/` ending in `messages/{id}`, the routes the clients give messages:

- `https://{deployment}/communities/{c}/channels/{ch}/messages/{m}`
- `/dms/{ch}/messages/{m}`

Rules:

- Its host is not checked, since a message id names one message anywhere.
- A link under `/at/{domain}` is to another deployment's message by its own route, and is left alone.
- Link previews leave message links out, so the server never fetches its own web client.

## Recording links

Posting or editing a message records the ids it links to:

- in order, each once;
- at most `MAX_LINKS` (5);
- never itself.

An edit that changes the text announces the new set in its `update` event.

- A command links nowhere.
- A warning shows what it is about through its own `warning` field (see [Reports](reports/index.md)).

## Reading links

Who may see what is decided when the links are read, for each reader. Message reads sideload `include=linked` as `included.linkedMessages`: one record per message linked, with its `state`.

| `state` | When |
| --- | --- |
| `available` | The reader may read the message. In a community, the record also names the community its route names |
| `deleted` | The message is deleted, and the reader may read its channel |
| `unavailable` | The reader may not read the message |

- The available messages themselves go among `included.messages`. Their authors and attachments come with the read's own, when those are asked for.
- A deleted message is `deleted` only where the reader may read its channel, so its having existed is no one else's to learn.
- Links are recorded, never followed: the messages a linked message links to are not read with it.
