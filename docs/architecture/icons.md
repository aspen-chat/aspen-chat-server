# Icons

An icon is a picture something is shown by: a user's profile, a bot's, a community, a custom emoji,
the deployment's profile, and a copy of a foreign user's avatar. Each is a row of `icon` and one
object in storage, under the public prefix `icons/` (`media_store::PublicPrefix::Icons`).

| Part           | Where                                                                    |
| -------------- | ------------------------------------------------------------------------ |
| Logic          | `app::icon`                                                              |
| Handlers       | `api::icon`                                                              |
| How it is kept | `icon::as_stored`, `aspen_previews::picture::fitted`                     |
| Old icons      | the [`shrinkIcons`](jobs/kinds.md#shrinkicons) job (`icon::shrink_step`) |
| Forgetting     | the [`forgetIcon`](jobs/kinds.md#forgeticon) job (`icon::forget_step`)   |

## Uploading

1. `POST /icons` reserves a row with `ready_at` empty and answers a presigned `PUT` URL for exactly
   the declared size, at most 8 MiB (`icon::MAX_BYTES`), of PNG, JPEG, WebP, or GIF
   (`icon::IMAGE_TYPES`).
2. The client uploads to the staging key the URL names.
3. `POST /icons/{icon}/confirm`, by the uploader alone, promotes the upload to the icon's own key,
   refuses one larger than the limit or whose header gives no size or more than 4096 by 4096 pixels'
   worth (`icon::MAX_PIXELS`), stores it as below, and marks it ready.

Only the uploader may then give it to something (`icon::require_own`).

## How it is stored

An icon is stored as it is shown, never larger (`icon::as_stored`):

- A picture larger than 512 by 512 (`icon::STORED_BOX`), or not WebP, is brought into sRGB, fitted
  within that box, and encoded as WebP, as attachments' previews are (see
  [Pictures](attachment-previews/pictures.md)).
- The WebP takes the picture's place when it is at least a tenth smaller
  (`attachment::preview::worth_keeping`). Otherwise the picture is kept as it was uploaded.
- A WebP that already fits is never encoded again.
- A picture that moves (an animated GIF, APNG, or WebP), or whose colours cannot be brought into
  sRGB, is kept as it was uploaded.
- The WebP is stored over the upload under the icon's own key, and the row's type becomes
  `image/webp`, before the icon is ready. Nobody ever reads the upload itself.
- The original is not kept.
- Each server fits at most half its logical CPUs' worth of pictures at once, on blocking threads,
  holding no database connection while it does.

Copies of foreign users' avatars are stored the same way (`federation::abroad`). Icons confirmed
before icons were stored this way are brought to it by the `shrinkIcons` job, which a migration
queues once.

512 pixels is twice the largest an icon is shown, the sign-in screen's 256 CSS pixels, for screens
of two device pixels to one, and the size the web client crops a picture to before uploading it
(`ICON_MAX_SIZE`).

## Forgetting

An icon nothing uses is deleted with its picture by `forgetIcon`, which the database queues a day
after an upload is confirmed and when what used it lets it go (see
[Kinds](jobs/kinds.md#forgeticon)).

## When access is given or taken away

1. **Who can observe it, and by which routes?** Anyone holding its `downloadUrl`: icons are public
   objects under `public_base_url`, with no credentials. `GET /icons/{icon}` gives the record to a
   signed-in user, and the records of profiles, communities, emoji, and the deployment carry the
   icon's id or record. `GET /federation/icons/{icon}` serves the picture to other deployments.
2. **What decides it, and where is that checked?** Nothing beyond holding the link: what a profile
   or community shows is public to whoever sees that profile or community. Giving an icon to
   something is checked by `icon::require_own`.
3. **When the deciding permission is lost, what happens to what is already open?** Nothing is
   withdrawn; a picture someone has seen they keep. An icon let go is deleted by `forgetIcon` once
   nothing uses it, and its link then answers nothing.
4. **When it is gained, how does a client already open find out?** Through the event of whatever
   took the icon up (a `User`, `Community`, or `CustomEmoji` update), which carries its id.
5. **Does every path that changes it announce it?** An `Icon` record has no events: its picture
   never changes but once, when `shrinkIcons` stores an older icon fitted under the same key and
   changes its type. Clients show the picture as storage serves it, by its own type, and read
   `mimeType` for nothing, so a record read before then is stale only in a field no client uses. The
   deployment's icon record, which every server holds a copy of, is read again on every server
   (`deployment_settings::icon_stored_anew`).
6. **Is it published inside the transaction that makes the change?** The deployment's revision is
   rung in the transaction that writes the type; nothing else is published.
