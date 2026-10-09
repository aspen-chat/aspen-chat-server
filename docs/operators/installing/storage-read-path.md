# The storage's read path

Everyone's apps fetch `public_base_url` (in `[media.s3]`) without credentials.

## What it must allow

It must allow exactly one thing: reading an object by its name (S3's `GetObject`), and only
under the four prefixes clients read:

- `attachments/`
- `attachment-previews/`
- `icons/`
- `link-preview-images/`

**It must not:**

- list the bucket, which would hand anyone every attachment ever posted;
- take writes or deletions;
- serve `evidence/`, where the files of deleted messages and of attachments taken off their
  messages are kept for reviewing reports (reviewers read them through links the server signs
  for ten minutes);
- serve `uploads/`, where uploads wait to be confirmed.

## Setting it up

| Storage | How |
| --- | --- |
| AWS S3 (or anything taking its policies) | A bucket policy allowing `s3:GetObject` to `*` on `arn:aws:s3:::BUCKET/attachments/*`, `arn:aws:s3:::BUCKET/attachment-previews/*`, `arn:aws:s3:::BUCKET/icons/*`, and `arn:aws:s3:::BUCKET/link-preview-images/*`, and nothing else. Not `s3:ListBucket`, and not `BUCKET/*`. |
| MinIO | `mc anonymous set download` grants listing and the whole bucket, so do not use it. Set a policy of your own with `mc anonymous set-json`, holding only the statement above. |
| Garage | Its website endpoint (`s3_web`, `garage bucket website --allow BUCKET`) lists nothing but serves every object. Put a reverse proxy or CDN before it that passes only paths under the four prefixes, and refuses the rest. |
| SeaweedFS | Give the S3 gateway an anonymous identity allowed only `Read` on the bucket (`"actions": ["Read:BUCKET"]` in its S3 configuration). That reaches every object, so serve `public_base_url` through a reverse proxy or CDN that passes only paths under the four prefixes. |

**SeaweedFS: never expose the filer** (port 8888). It lists directories and takes uploads and
deletions from anyone.

### A proxy rule for the prefixes

In Caddy (`handle` blocks are tried in order):

```
media.chat.example.org {
    @public path_regexp ^/aspen-media/(attachments|attachment-previews|icons|link-preview-images)/[^/]+$
    handle @public {
        header X-Content-Type-Options nosniff
        reverse_proxy 127.0.0.1:3902
    }
    respond 404
}
```

### Its origin and headers

- Serve `public_base_url` from an origin of its own (`https://media.chat.example.org`), never
  under `public_url`'s.
- Have it (or the CDN before it) send `X-Content-Type-Options: nosniff`.

## Checking it

Run these from a machine outside your network, with:

- `OBJECT`: the address of any picture someone posted (copy it from the app),
- `BASE`: `public_base_url`,
- `S3`: `public_endpoint` with the bucket (`https://s3.chat.example.org/aspen-media`).

```
curl -s -o /dev/null -w '%{http_code}\n' "$OBJECT"                     # 200
curl -sI "$OBJECT" | grep -i x-content-type-options                    # nosniff
curl -s -o /dev/null -w '%{http_code}\n' "$BASE/"                      # 403 or 404, never 200
curl -s -o /dev/null -w '%{http_code}\n' "$BASE/?list-type=2"          # 403 or 404, never 200
curl -s -o /dev/null -w '%{http_code}\n' "$S3?list-type=2"             # 403
curl -s -o /dev/null -w '%{http_code}\n' -X PUT --data x "$BASE/write-check"  # 403 or 405
curl -s -o /dev/null -w '%{http_code}\n' -X PUT --data x "$S3/write-check"    # 403
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE "$OBJECT"           # 403 or 405
```

Then check that the read path keeps `evidence/` to itself, with the AWS command line and the
deployment's storage key pair (`ENDPOINT` is `[media.s3] endpoint`):

```
echo check | aws --endpoint-url "$ENDPOINT" s3 cp - s3://aspen-media/evidence/read-check
curl -s -o /dev/null -w '%{http_code}\n' "$BASE/evidence/read-check"  # 403 or 404, never 200
aws --endpoint-url "$ENDPOINT" s3 rm s3://aspen-media/evidence/read-check
```

| A `200` for | Means |
| --- | --- |
| A listing | Anyone can see the bucket's contents. |
| A write | Anyone can put files at your media address. |
| `evidence/` | Anyone holding an old link can read what was deleted. |

---

Next: [Upgrading](upgrading.md)
