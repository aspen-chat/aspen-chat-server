# Media storage

Attachments, what types they are served as, link previews, and the object storage they are kept
in. Previews of pictures and videos are on [Previews](previews.md).

## `[media]`

| Setting | Default | |
| --- | --- | --- |
| `max_attachment_bytes` | `268435456` (256 MiB) | The largest file anyone may attach. Icons are held to 8 MiB and custom emoji to 256 KiB whatever this says. |

### What storage must support

- Apps declare a file's size when they ask to upload it. The upload URL is signed for exactly
  that size, so storage refuses more.
- Storage must give uploads an `ETag`.
- Storage must honour `x-amz-copy-source-if-match` when copying.
- SeaweedFS and S3 both do.

### What types files are served as

Attachments are served from `public_base_url` as what they are only when they are one of these:

- pictures, video, or sound in the formats browsers play
- plain text
- PDF

They are then served as that type alone, with none of the parameters the sender's app added,
except a plain text's `charset`.

Anything else (an HTML page, an SVG, XML, a script, an archive):

- is uploaded and stored as `application/octet-stream`
- gets `Content-Disposition: attachment` where the store keeps it, so a browser saves it rather
  than opening it at your media address
- still shows in apps with the type its sender's system gave it.

Icons may only be PNG, JPEG, WebP, or GIF. The pictures of link previews are kept only when they
are one of those; a page whose picture is an SVG is previewed without it.

### Link previews

The server fetches the pages messages link to, and their pictures, to show previews of them.

- It reaches only public addresses, and only their ports 80 and 443.
- So a link to another port of a public host (your own server's NATS, PostgreSQL, or metrics, at
  its public address) is shown without a preview. Keep those services off public interfaces all
  the same.
- Each server fetches at most 64 messages' previews at once, and two of one author's.
- A message past that, or one that waits more than 30 seconds for its turn, is shown without
  previews.

## `[media.s3]`

Where attachments, icons, avatars, and link preview images are kept. Clients upload straight to
storage with short-lived URLs the server signs, and download from a public path. Two of these
addresses (`public_endpoint` and `public_base_url`) are the clients' and must be reachable by
them.

| Setting | Default | |
| --- | --- | --- |
| `endpoint` | `http://127.0.0.1:3900` | The S3 API, as this server reaches it. |
| `public_endpoint` | `endpoint` | The S3 API as clients reach it; the upload URLs they are handed name it. It must allow uploads by [CORS](#cors-for-uploads). |
| `public_base_url` | `http://127.0.0.1:3902/aspen-media` | Where clients download objects. It must be a [read-only path](#the-read-path) at an origin of its own. |
| `bucket` | `aspen-media` | |
| `region` | `garage` | Whatever your storage expects; many accept any. |
| `access_key`, `secret_key` | development values | A key pair that may read, write, delete, and list in the bucket. A server at a public `https` address refuses the development values. |
| `upload_url_ttl_seconds` | `900` | How long an upload URL works. |
| `[media.s3.tls] ca_file` | | PEM authorities this server trusts besides the system's, for an `https` `endpoint`. See [`[media.s3.tls]`](#medias3tls). |

### `[media.s3.tls]`

- `ca_file` is for storage with a certificate from your own authority, such as storage on a
  private network.
- It applies only to `endpoint`. Clients reach `public_endpoint` and `public_base_url` with their
  own trust, so give those certificates clients trust.
- The S3 client presents no client certificate.

### CORS for uploads

`public_endpoint` must allow `PUT`, with `Content-Type`, by CORS from:

- `public_url`'s origin
- `null`, for the desktop app
- `capacitor://localhost` and `https://localhost`, for the mobile apps

Allowing any origin also works. Leaving `public_endpoint` out suits only clients on the server's
own machine.

### The read path

`public_base_url` is an anonymous read path on the bucket, such as a website endpoint or a CDN.

- It allows reading objects and nothing else: no listing, no writes.
- **Never a SeaweedFS filer.**
- It is at an origin of its own.
- It sends `X-Content-Type-Options: nosniff`.
- The server itself never needs to reach it.

[The storage's read path](../installing/storage-read-path.md) says how to set it up, and how to
check it.

### The `uploads/` prefix

1. An upload URL writes under `uploads/`, never where readers fetch from.
2. Confirming the upload has the store copy the file into place.
3. Each server deletes what is left under `uploads/` an hour after its URL expired.

A lifecycle rule on your storage expiring `uploads/` after a day does no harm.
