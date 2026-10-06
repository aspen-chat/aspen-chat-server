# Configuration

The API server reads `aspen.toml` from its working directory. Any setting can be given in the
environment instead, which overrides the file: `ASPEN_` followed by the key, with `__` between a
section and its keys (`ASPEN_PUBLIC_URL`, `ASPEN_DATABASE_URL`,
`ASPEN_VOICE__IDLE_SESSION_SECONDS`). A setting left out takes the default shown. A value the
server cannot read stops it at startup with a message naming the setting; in `[federation]`,
`[web_client]`, and `[email]`, so does a key it does not know.

A voice server reads `voice_server.toml` the same way, with the prefix `ASPEN_VOICE_SERVER_`;
see [the voice server](#voice_servertoml) at the end.

`aspen.toml` holds what a server needs to start: where its services are, its secrets, its sizes,
and what is bound to its domain. What the deployment's administrators decide (its name, who may
register, second factors, bots, community limits, files in calls, and the federation gates) is
kept in the database instead, so every server follows one answer and a change takes effect at
once without a restart; see [Deployment settings](#deployment-settings). So are the voice servers
calls run on; see [Voice servers](#voice-servers).

## The deployment's address

A deployment is one address: the API, the web client, and the pages the apps open are all served
by each API server, at the same origin.

| Setting | Default | |
| --- | --- | --- |
| `public_url` | required | Your deployment's address, an origin alone, such as `https://chat.example.org`. Everything that names the deployment follows from it: invite links, registration links, the QR codes for invites and for signing in from another device, and the links in mail all point here; passkeys belong to its host; and over `https` its host, with `:port` when not 443, is the deployment's [federation](federation.md) domain. **Other deployments remember the key they find at that domain, so once federating it can never change:** the first server to start with an `https` address records the domain in the database, and a server started with another refuses to start and says which to set. Changing the host also makes every registered passkey useless. Passkeys are offered over `https`, and at `localhost` and names under it; an `http` address on any other host offers none, and an `http` address takes no part in federation. |

### `[web_client]`

| Setting | Default | |
| --- | --- | --- |
| `dir` | `client/packages/app/dist` | The built web client (`pnpm build` in `client/`). The server serves its files, and answers every other path the API does not own with its `index.html`, given link preview tags (Open Graph), so a link to your deployment shared in a chat or a post shows your deployment's name and icon, and an invite link its community's. **The server will not start without it.** Files are read as they are asked for, so a new release is served as soon as it is in place; see [Releasing a new web client](installing.md#releasing-a-new-web-client). |

## The services

| Setting | Default | |
| --- | --- | --- |
| `database_url` | required | PostgreSQL, as `postgres://user:password@host/database`. |
| `database_pool_size` | two per logical CPU | The most PostgreSQL connections this server holds at once. Every write holds one until NATS acknowledges its event, so a busy server may run out of connections before PostgreSQL runs out of CPU; the database connections metric shows requests waiting. Keep the total over every API server below PostgreSQL's `max_connections`. |
| `database_pool_wait_seconds` | `10` | How long a request or background task waits for one of those connections before it is refused with `serverBusy`. A pool that runs dry, because NATS is slow to acknowledge or the server has more work than connections, then refuses work rather than holding it until it frees. |
| `nats_url` | required | NATS with JetStream, as `host:4222`. |
| `nats_auth_token` | | The token NATS was started with, when NATS takes one token from everyone. |
| `[nats] user`, `password` | | The API servers' NATS user, when NATS has users, as it does once each voice server has one of its own ([Installing](installing.md#6-voice-servers)). Give exactly one of this and `nats_auth_token`. |
| `valkey_url` | required | Valkey, as `redis://host:6379`. |

## Event delivery

| Setting | Default | |
| --- | --- | --- |
| `event_queue_size` | `512` | How many events one connection may have waiting to be written. A connection that falls this far behind (a very slow network) is dropped, and its client reconnects and catches up. |
| `event_feed_shards` | one per CPU | How many tasks deliver events to this server's connections. |

## `[connections]`

What the server's listener admits, so clients that open connections and then send nothing, or
send it a byte at a time, cannot hold every socket the server has. A connection over a limit is
closed as soon as it is accepted, and the server logs that it is closing new connections at
most once a minute. An event stream's connection counts for as long as it is open.

| Setting | Default | |
| --- | --- | --- |
| `max` | `100000` | The most connections the server holds open at once. Keep it below the process's open file limit (`LimitNOFILE` under systemd, `--ulimit nofile` in Docker), with room for its connections to PostgreSQL, NATS, Valkey, and storage. |
| `max_per_ip` | `512` | The most one address holds open at once; an IPv6 address counts by its `[rate_limits] ipv6_prefix` network. A browser holds one or two, so this suits a few hundred people behind one address; raise it if a school's or an office's network sends many more. Reverse proxies listed in `[rate_limits] trusted_proxies` count only toward `max`, so when every client arrives through one, limit connections per client there. |
| `handshake_seconds` | `10` | How long a client has to finish its TLS handshake. |
| `header_read_seconds` | `30` | How long an HTTP/1.1 client has to send a request's headers, which is also how long a kept-alive connection may sit idle. An idle HTTP/2 connection is pinged every 30 seconds and closed when a ping goes 20 seconds unanswered. |

## `[media]`

| Setting | Default | |
| --- | --- | --- |
| `max_attachment_bytes` | `268435456` (256 MiB) | The largest file anyone may attach. Apps must declare a file's size when they ask to upload it, and the upload URL is signed for exactly that size, so storage refuses more. Storage must give uploads an `ETag` and honour `x-amz-copy-source-if-match` when copying, as SeaweedFS and S3 do. Icons are held to 8 MiB and custom emoji to 256 KiB whatever this says. |

Attachments are served from `public_base_url` as what they are only when they are pictures,
video, or sound in the formats browsers play, plain text, or PDF, and then as that type alone,
with none of the parameters the sender's app added but a plain text's `charset`. Anything else (an HTML page, an
SVG, XML, a script, an archive) is uploaded and stored as `application/octet-stream`, with
`Content-Disposition: attachment` where the store keeps it, so a browser saves it rather than
running what it holds at your media address; apps still show the type its sender's system gave
it. Icons may only be PNG, JPEG, WebP, or GIF, and the pictures of link previews are kept only
when they are one of those (a page whose picture is an SVG is previewed without it).

The server fetches the pages messages link to, and their pictures, to show previews of them.
It reaches only public addresses, and only their ports 80 and 443, so a link to another port of
a public host (your own server's NATS, PostgreSQL, or metrics, at its public address) is shown
without a preview; keep those services off public interfaces all the same. Each server fetches
at most 64 messages' previews at once and two of one author's; a message past that, or one that
waits more than 30 seconds for its turn, is shown without previews.

## `[media.s3]`

Where attachments, icons, avatars, and link preview images are kept. Clients upload straight to
storage with short-lived URLs the server signs, and download from a public path, so two of these
addresses are the clients' and must be reachable by them. An upload URL writes under `uploads/`,
never where readers fetch from: confirming the upload has the store copy it into place, so a URL
used again later changes nothing anyone reads, and each server deletes what is left under
`uploads/` an hour after its URL expired. A lifecycle rule expiring `uploads/` after a day does
no harm.

| Setting | Default | |
| --- | --- | --- |
| `endpoint` | `http://127.0.0.1:3900` | The S3 API, as this server reaches it. |
| `public_endpoint` | `endpoint` | The S3 API as clients reach it; the upload URLs they are handed name it. It must allow uploads by CORS (`PUT`, with `Content-Type`) from `public_url`'s origin and the apps' (`null` for the desktop app, `capacitor://localhost` and `https://localhost` for the mobile apps), or from any origin. Leaving it out suits only clients on the server's own machine. |
| `public_base_url` | `http://127.0.0.1:3902/aspen-media` | Where clients download objects: a public read path on the bucket, such as a website endpoint or a CDN. The server itself never needs to reach it. |
| `bucket` | `aspen-media` | |
| `region` | `garage` | Whatever your storage expects; many accept any. |
| `access_key`, `secret_key` | development values | A key pair that may read, write, delete, and list in the bucket. |
| `upload_url_ttl_seconds` | `900` | How long an upload URL works. |

## `[media.previews]`

Readers' apps show pictures and videos inline from a smaller copy the server makes: a picture
fitted within 1920 × 960 pixels as WebP, and a video's poster, one frame of it, with the video
itself played only when asked. The originals stay what the gallery shows and what is saved.
A message sent with a picture or video whose preview is still being made may wait for it, up to
twenty seconds from the upload, before it is posted. Every server queues previews; those with
`make` on make them, from the database's queue, so a server that dies leaves its work to the
others. Pictures are made in the server itself. Videos' posters are taken by running `ffmpeg`
and `ffprobe`, which need not be installed: a server without them makes previews of pictures
only, and says so when it starts. Each video is copied whole to a scratch file under `TMPDIR`,
so point that at a disk with room for `max_video_bytes` times `concurrency` (not a RAM-backed
`/tmp`).

| Setting | Default | |
| --- | --- | --- |
| `make` | `true` | Whether this server makes previews. Leave it on somewhere: with no server making them, messages sent with pictures wait their full twenty seconds. |
| `concurrency` | `2` | How many previews this server makes at once. A large picture may take several hundred megabytes while it is made. |
| `max_picture_bytes` | `67108864` (64 MiB) | The largest picture a preview is made of. |
| `max_picture_pixels` | `100000000` | The most pixels a picture may have for a preview to be made of it. |
| `ffmpeg`, `ffprobe` | `"ffmpeg"`, `"ffprobe"` | The programs that take videos' posters, by path or by name on `PATH`. An empty `ffmpeg` takes none. |
| `max_video_bytes` | `2147483648` (2 GiB) | The largest video a poster is taken of. |
| `ffmpeg_memory_mib` | `3072` (3 GiB) | The most memory `ffmpeg` or `ffprobe` may map while taking one poster. A frame of 8K video needs about 2 GiB. |

`ffmpeg` and `ffprobe` run with none of the server's environment, may read only the video they
are given, decode only the codecs phones, cameras, and screen recorders write and frames of at
most `max_picture_pixels`, on one thread, for at most a minute of CPU time, and may write no
file; a frame larger than `max_picture_bytes` is not read. A decoder is still a large body of C
reading files anyone can send, so run the server as a user that cannot read anything it does
not need (not `aspen.toml`'s secrets beyond what the server reads at startup, nor other
services' files), or in a container or sandbox of its own, so a flaw in `ffmpeg` reaches as
little as possible. Leaving `ffmpeg` empty takes no posters at all.

The metrics `aspen_attachment_previews_made_total`, `aspen_attachment_previews_failed_total`,
and `aspen_attachment_preview_duration_seconds` count them, by `kind` (`picture`, `video`).

## `[auth]`

| Setting | Default | |
| --- | --- | --- |
| `reverify_seconds` | `600` | Changing security settings needs a password or code given within this long. |
| `password_hashing_threads` | one per CPU | How many passwords may be hashed or checked at once. Each takes 19 MiB and most of a core for a moment. |
| `password_hashing_wait_seconds` | `10` | How long a sign-in waits for one of those threads before it is refused with `serverBusy`. |

## `[limits]`

| Setting | Default | |
| --- | --- | --- |
| `max_communities_per_user` | `500` | The most communities one account may belong to. It bounds how much each connection reads and how far one profile change spreads. |
| `max_event_streams_per_user` | `20` | The most live connections (event streams) one account may hold open on each API server. One more is closed with code 4429. Each window or device of the app holds one. |
| `max_event_streams_per_address` | `200` | The most live connections one client address may hold open on each API server, counted before they sign in (an IPv6 address by its `[rate_limits] ipv6_prefix` network). One more is closed with code 4429. Raise it when many people reach the deployment from one address, behind one NAT or one proxy you have not listed in `trusted_proxies`. |

## `[presence]`

| Setting | Default | |
| --- | --- | --- |
| `away_after_seconds` | `600` | How long someone connected but not using Aspen stays shown as online before showing as away. Clients report use at most once a minute, so much less than a few minutes makes people flicker. |

## `[voice]`

| Setting | Default | |
| --- | --- | --- |
| `token_secret` | a development value | Signs the tokens that let people into calls; every voice server must have the same. **Set it to a long random string** (`openssl rand -base64 48`): a server whose `public_url` is `https` refuses to start with the development value or with one shorter than 32 bytes, since whoever knows it can let themself into any call. |
| `failure_threshold` | `5` | How many different people failing to reach a voice server, within `failure_window_seconds`, suspend it for `failure_window_seconds`, after which it takes calls again on its own (an administrator enabling it ends the suspension sooner). Only this deployment's people whom a join offer sent to that server within the token's lifetime and a minute count, and not those who joined a call there within the window; bots and people from other deployments never do. The last server taking calls is never suspended. |
| `failure_window_seconds` | `3600` | How long failures are counted, and how long a suspension lasts. |
| `join_token_ttl_seconds` | `60` | How long someone has to reach a voice server after asking to join. |
| `candidate_limit` | `10` | The most voice servers one person is offered to choose the nearest from. |
| `offer_silence_seconds` | `60` | A voice server that has not reported for this long is not offered to people joining. |
| `session_silence_seconds` | `86400` | A voice server that has not reported for this long has its calls ended. Long on purpose: a call is worth more than tidiness after a brief network fault. |
| `idle_session_seconds` | `86400` | A call that never had two people in it at once ends after this long, so a forgotten client cannot hold a place on a voice server. |

## `[plugins]`

How plugins run; which are installed, and their settings, are in the database (see
[Plugins](plugins.md)).

| Setting | Default | |
| --- | --- | --- |
| `intercept_millis` | `25` | How long a plugin has to decide a message about to be saved. Its author waits for it, so keep it short. |
| `observe_millis` | `10000` | How long a plugin has to handle something that happened, such as checking a new message's pictures with another service. |
| `route_millis` | `3000` | How long a plugin has to answer a request to one of its routes. |
| `memory_mib` | `64` | The most memory one call of a plugin may use, all its memories together. |
| `concurrency` | two per logical CPU | The most calls of plugins this server runs at once. A call waits for a place within its own time limit and counts as failed when none comes, so a refusing filter (`failure: closed`) refuses messages while the server is this busy. With `memory_mib` it bounds what plugins can take of the server's memory. |
| `concurrency_per_plugin` | one per logical CPU | The most calls of any one plugin this server runs at once, so one busy plugin leaves room for the rest. |

## `[push]`

| Setting | Default | |
| --- | --- | --- |
| `enabled` | `true` | Whether the Aspen app on phones may ask to be woken when it is not open, for DMs and messages that tag someone. Phones are woken through the relay of whoever published their app (the Aspen Foundation's, for the published apps) or through a UnifiedPush distributor; this server calls them over HTTPS, as it calls other deployments. What it sends them is encrypted to the phone, and says only which channel and message to fetch. |

## `[email]`

Without this section the deployment sends no mail: accounts have no email address, nobody can
reset a forgotten password by email, and the email deployment settings below cannot be turned
on. With it, people may give an address at registration or in their account settings, and the
deployment sends verification codes, password reset codes, the daily digest people ask for, and
the newsletter, when its administrators run one. Mail waits in a queue in the database, password
resets first, and is sent by every API server whose `send` is on: by default all of them, so the
SMTP server must accept connections from each. A large deployment can turn `send` off on most
servers and leave sending, the SMTP credentials, and the work of making digests to a few, which
may be [private workers](installing.md#private-workers) that serve nothing; those few must reach the SMTP server,
and the rest need only `from`. Links in mail, unsubscribing included, go to `public_url`.

| Setting | Default | |
| --- | --- | --- |
| `send` | `true` | Whether this server sends mail and makes daily digests. With it off, the server still takes addresses and queues mail, and tells the senders at once (over NATS) when someone waits for it; at least one server of the deployment must send, or mail waits until one does. |
| `smtp_url` | required where `send` is on | The SMTP server mail is handed to: `smtps://user:password@smtp.example.org` (TLS from the start, port 465), `smtp://user:password@smtp.example.org?tls=required` (STARTTLS, port 587), or `smtp://localhost:1025` for a development mail catcher such as the `mailpit` service in `docker-compose.yaml` (its inbox is at http://localhost:8025). Percent-encode characters in the user and password that a URL reserves. Any provider that takes SMTP works (Amazon SES, Postmark, Mailgun, your own Postfix). |
| `from` | required | Who mail comes from, such as `Example Chat <noreply@chat.example.org>`. Its domain should publish SPF and DKIM records for the SMTP server you use, or mail lands in spam. |
| `max_per_second` | none | The most mail the whole deployment hands to the SMTP server in a second, however many servers send, counted in Valkey; set it under your provider's sending quota (Amazon SES starts accounts at 14 a second). A second's worth may go back to back. Left out, each sending server sends up to eight at once. |

Mail the SMTP server refuses for good (an address that does not exist) is dropped and logged;
mail it cannot take now is tried again, waiting longer each time, for about a day. The metrics
`aspen_emails_sent_total` and `aspen_emails_failed_total` count both.

## `[metrics]`

| Setting | Default | |
| --- | --- | --- |
| `enabled` | `true` | Prometheus metrics at `GET /metrics` on a listener of their own. |
| `listen_addr` | `127.0.0.1:9464` | Keep it off public interfaces: the figures describe the deployment's inside. |

## `[rate_limits]`

Every endpoint is rate limited. The built-in limits, with what each is for, are in
`server/src/rate_limits.toml`, which also explains the format; what you write here is laid over
them, a limit you give replacing the built-in one whole.

| Setting | Default | |
| --- | --- | --- |
| `enabled` | `true` | |
| `trusted_proxies` | `[]` | Reverse proxies, as addresses or networks (`"10.0.0.0/8"`), whose `X-Forwarded-For` names the client: the right-most entry that is not itself a listed proxy, read with or without a port (`192.0.2.1:5678`, `[2001:db8::1]:80`). An entry that is not an address (`unknown`) ends the reading there, and the client counts as the listed proxy that passed it on. **Set it when the server is behind a proxy**, or every client counts as the proxy. |
| `ipv6_prefix` | `64` | IPv6 clients are counted by their network of this many bits, since one household holds a whole /64. |
| `max_suspension_seconds` | `86400` | The longest this server honours a suspension of the limits (`aspen-chat-server limits suspend`), counted from when it began. |
| `fail_closed` | sign-in, password reset, registration, and invite endpoints | Endpoints refused with `serverBusy` while Valkey cannot be reached to count their limits, rather than let through, so an outage does not allow unlimited guessing of passwords, codes, and invites. Every other endpoint is let through during an outage. The per-username sign-in limit always fails closed. A list given here replaces the built-in one (in `rate_limits.toml`) whole. |

Limits for one endpoint go under its method and path:

```toml
[rate_limits.endpoints."POST /channels/{channel}/messages"]
user = { requests = 10, per_seconds = 10, burst = 5 }
```

A limit that names an endpoint, or a path parameter, that does not exist stops the server at
startup with a message saying which.

## `[federation]`

See [Federation](federation.md) for what these mean together. The deployment's domain is
`public_url`'s host.

| Setting | Default | |
| --- | --- | --- |
| `standing_interval_seconds` | `3600` | How often this deployment asks other deployments whether their users here are still in good standing, and reads again the documents of the deployments it federates with, which is how soon it notices one replaced its key. |
| `standing_grace_seconds` | `86400` | How long another deployment may go unreached before its users' sessions here end. |

### `[federation.development]`

For running deployments side by side on one machine (`scripts/dev_federation.py`). Leave it out
of a deployment anyone else uses.

| Setting | Default | |
| --- | --- | --- |
| `extra_root_certificates` | `[]` | PEM files of certificate authorities to trust, besides the system's, when calling other deployments. |
| `allow_private_addresses` | `false` | Lets this server call deployments at private and loopback addresses, which it otherwise refuses so that naming a deployment cannot make it reach inside its own network. |

## Deployment settings

These are kept in the database. Holders of Manage deployment settings change them in the
Administration Dashboard under **Settings**, and holders of Manage federation change the gates
under **Federation**. From the terminal, `aspen-chat-server settings show` lists every one, and
`aspen-chat-server settings set` changes those it is given, such as
`settings set --registration-invite-required true --bots-max-per-user 5`; the terminal needs
NATS, which tells every server at once. An invite-only deployment is made one this way before
anyone has an account.

| Setting | Default | |
| --- | --- | --- |
| `display-name` | none | What the deployment calls itself: on its sign-in screens, as the account its notices come from, and to its users' authenticator apps and passkey prompts, which say "Aspen" without one. The dashboard also sets an icon beside it. An authenticator keeps the name it was given when it was added. |
| `registration-invite-required` | `false` | Creating an account takes a registration invite, made in the dashboard or with `aspen-chat-server invites create`. An invite made in the dashboard may also name a community (a dual invite): each account it makes joins that community as it is made, and someone who already has an account just joins. Revoking a dual invite revokes its community invite too; revoking that community invite from the community leaves a plain registration invite. |
| `require-two-factor` | `false` | Every account must have an authenticator app or a passkey. An account without one can do nothing but add one or sign out; turned on, the apps of those without one are asked to at once. |
| `bots-enabled` | `true` | Whether people may make bots. Bots already made keep working either way. |
| `bots-max-per-user` | `25` | The most bots one person may own. |
| `everyone-mention-limit` | `200` | How many members a community gains before its everyone role loses Mention everyone, so one `@everyone` cannot reach that many people by accident. It happens once per community, and the owner is told why by the deployment's own account (which cannot be signed in to, messaged, or blocked) and may turn it back on. `0` never turns it off. |
| `custom-emoji-limit` | `1000` | The most custom emoji one community may hold. Each is a small picture (PNG, JPEG, WebP, or GIF, at most 256 KiB) in the object storage. |
| `email-required` | `false` | Creating an account takes an email address. Needs `[email]`. It binds registration only: accounts made before it was turned on are not asked for one. |
| `email-verification-required` | `false` | An account that has an email address must verify it, by the code mailed to it, before it can use the deployment; until then it can only verify, change, or resend its address, or sign out. Turned on, the apps of those with an unverified address are asked at once. Accounts without an address are not affected unless `email-required` is on too, and then only once they give one. Needs `[email]`. |
| `newsletter-enabled` | `false` | The deployment has a newsletter: people may subscribe at registration (unticked by default) and in their account settings, and holders of Send newsletters write and send posts under **Newsletter** in the dashboard. Only verified addresses receive it, and each piece carries an unsubscribe link. Needs `[email]`. |
| `file-transfers` | `true` | Whether people may offer files to one another in calls. Transfers go between their devices, or through a voice server's relay (`[transfer]` in `voice_server.toml`); either way this deployment keeps a record of each offer and transfer for its moderators, never the file. `false` turns the whole feature off: no one can offer a file, whatever the channels' permissions, and calls in progress follow at once. |
| `users-emigration`, `bots-emigration` | `closed` | Whether this deployment's accounts may use other deployments: `closed`, `open`, `allowList`, or `blockList`. See [Federation](federation.md). |
| `users-immigration`, `bots-immigration` | `closed` | Whether other deployments' accounts may use this one, the same way. Closing or narrowing it signs out at once those it no longer admits. |
| `users-shared-list`, `bots-shared-list` | `false` | Both directions read one list; both gates must then be the same kind of list. |
| `users-immigration-invite-required`, `bots-immigration-invite-required` | `false` | An account of another deployment arriving for the first time needs a registration invite. |

## Voice servers

The voice servers calls run on are registered in the database. Holders of Manage voice servers
add, change, disable, and remove them in the dashboard. From the terminal:

- `aspen-chat-server voice-servers add NAME --url URL --capacity N` registers one, or gives the
  one already registered by that name this address and capacity, so a deployment script may run
  it every time it deploys. `url` is where clients reach it, as
  `https://voice-1.chat.example.org`; `capacity` is the most people it carries at once, which
  `voice_server estimate-capacity` suggests.
- `voice-servers set NAME [--url URL] [--capacity N] [--enabled true|false]` changes one. A
  disabled server is offered to no one joining a call; calls already on it go on.
- `voice-servers remove NAME` removes one that holds no calls. Disable it first and let its calls
  end, or remove it from the dashboard, which ends them.
- `voice-servers list` lists them.

## `voice_server.toml`

| Setting | Default | |
| --- | --- | --- |
| `id` | required | The server's id in the registry (`SELECT id FROM voice_server WHERE name = '…'` once the API server has registered it). |
| `token_secret` | required | The API servers' `[voice] token_secret`. The server warns at startup when it is the development value or shorter than 32 bytes. |
| `nats_url` | required | The same NATS as the API servers. |
| `[nats] user`, `password` | | This voice server's own NATS user, allowed only its own subjects ([Installing](installing.md#6-voice-servers) gives its permissions). |
| `nats_auth_token` | | The API servers' token instead, which lets this server do anything they can; the server warns at startup. Give exactly one of this and `[nats]`. |
| `listen_addr` | `0.0.0.0:9001` | Where the health check and signalling listen, as plain HTTP; put a TLS proxy in front. |
| `workers` | one per CPU | Media worker processes, each using at most one core. |

### `[rtc]`

| Setting | Default | |
| --- | --- | --- |
| `ip` | `0.0.0.0` | The interface media is received on. |
| `announced_address` | the primary interface's address | The address clients send media to. Set it to the public address when the server is behind NAT. Never a loopback address: the server refuses to start with one. |
| `min_port`, `max_port` | `40000`, `40999` | The media ports, UDP and TCP; open them to clients. The range bounds how many people the server can carry. |

### `[transfer]`

Files people offer each other in calls travel over a connection between their two devices,
encrypted end to end. The voice server answers STUN, so that devices can find the addresses a
direct connection would use, and relays transfers through TURN for people who choose not to
connect directly, or whose connection cannot be made. It relays only between people in its own
calls, never to the rest of the internet, and sees only ciphertext.

| Setting | Default | |
| --- | --- | --- |
| `relay_mbps` | `50` | The most every relayed transfer on this server may carry together, in megabits a second. People are told this limit before they choose the relay. `0` turns relaying off: transfers then go directly between devices or not at all. |
| `port` | `3478` | The UDP port STUN and TURN answer on; open it to clients. |
| `relay_min_port`, `relay_max_port` | `42000`, `42999` | The UDP ports relayed transfers use, two for each (one per side), inside the server only: they need not be open to clients. |

### `[metrics]` and `[rate_limits]`

As the API server's, with `listen_addr` defaulting to `127.0.0.1:9465`. The voice server's
built-in limits are `voice_server/src/limits.toml`, which documents each, including
`max_message_bytes` and `max_pending_sockets_per_ip`, and these, which bound how much of the
media port range one account can hold:

| Setting | Default | |
| --- | --- | --- |
| `max_connections` | `10000` | Connections the listener holds at once; more wait to be accepted. Raise the process's file limit (`ulimit -n`, `LimitNOFILE=`) above it. Every connection must send each request's headers within ten seconds. |
| `max_seats_per_user` | `2` | Calls one account may be in at once on this server. Joining the same call again from another device replaces the first and takes no more. |
| `max_participants_per_call` | `500` | People one call on this server may hold at once. |

A transport that has not connected within thirty seconds of being made is closed, so ports are
not held by transports nobody uses. `createTransport`, `produceRtp`, and `consumeRtp` are limited
per address and for the whole server as well as per account; a load test from a few addresses
suspends the limits (`limits suspend`).
