# Configuration design notes

The reasons behind the settings described in [Configuration](../../operators/configuration/index.md).
For engineers changing `server/app/src/aspen_config.rs` or `voice_server/src/config.rs`.

## File and database

- **What a server needs to start is in `aspen.toml`; what administrators decide is in the
  database.** Every server then follows one answer, and a change takes effect at once without a
  restart. ([Deployment settings](../../operators/configuration/deployment-settings.md))

## Services

- **The pool refuses work after `database_pool_wait_seconds` rather than holding it.** A pool
  that runs dry (NATS slow to acknowledge, or more work than connections) would otherwise hold
  work until a connection frees. ([Services](../../operators/configuration/services.md))

## Connections

- **The listener limits connections per address, per network, and in total, with handshake and
  header timeouts.** Clients that open connections and then send nothing, or send a byte at a
  time, cannot then hold every socket the server has.
  ([Connections](../../operators/configuration/connections.md))
- **`max_per_network` sits alongside `max_per_ip`.** Whoever holds many addresses in one block
  cannot take the server's connections by spreading over them.
- **`ipv6_prefix` defaults to 64.** One household holds a whole /64.
  ([Rate limits](../../operators/configuration/rate-limits.md))

## Accounts

- **`max_communities_per_user` exists to bound work.** It bounds how much each connection reads
  and how far one profile change spreads.
  ([Accounts](../../operators/configuration/accounts.md))

## Media

- **Files outside the inline types are stored as `application/octet-stream` with
  `Content-Disposition: attachment`.** A browser then saves them rather than running what they
  hold at the deployment's media address. ([Media](../../operators/configuration/media.md))
- **Uploads go under `uploads/` and are copied into place on confirmation.** An upload URL used
  again later then changes nothing anyone reads.
- **Link previews fetch only public addresses on ports 80 and 443.** A link cannot make the
  server reach its own NATS, PostgreSQL, or metrics at its public address.

## Previews

- **Previews are made as jobs, by any server with `make` and `[jobs] run` on.** A server that
  dies leaves its work to the others. ([Jobs](../jobs/index.md)) ([Previews](../../operators/configuration/previews.md))
- **`ffmpeg` and `ffprobe` are confined, and operators are told to isolate the server too.** A
  decoder is a large body of C reading files anyone can send, so a flaw in `ffmpeg` should reach
  as little as possible.

## Voice

- **`session_silence_seconds` is long (a day).** A call is worth more than tidiness after a brief
  network fault. ([Voice calls](../../operators/configuration/voice.md))
- **`idle_session_seconds` ends calls that never had two people.** A forgotten client cannot hold
  a place on a voice server.
- **A voice server's `url` must be `https` wherever `public_url` is.** Browsers on an `https`
  page refuse unencrypted WebSockets.
- **Transports that have not connected within thirty seconds are closed.** Ports are not held by
  transports nobody uses. ([Voice server](../../operators/configuration/voice-server.md))

## Plugins

- **`concurrency_per_plugin` caps one plugin below the total.** One busy plugin leaves room for
  the rest. ([Plugin limits](../../operators/configuration/plugins.md))

## Jobs

- **Jobs wait in the database.** None is lost to a restart, every server that runs jobs takes
  its share, and one that stops part way through a job leaves it to another within a minute.
  ([Jobs](../../operators/configuration/jobs.md), [how jobs work](../jobs/index.md))
- **Each class of job keeps one place of its own within `concurrency`.** A busy class never holds
  up the others.

## Email

- **An `smtp://` URL with credentials needs `?tls=required` unless the host is local.**
  `tls=opportunistic` can be stripped by whoever sits between.
  ([Sending mail](../../operators/configuration/email.md))

## Metrics

- **The metrics listener stays off public interfaces.** The figures describe the deployment's
  inside. ([Metrics](../../operators/configuration/metrics.md))

## Rate limits

- **`fail_closed` endpoints are refused while Valkey is down.** An outage then does not allow
  unlimited guessing of passwords, codes, and invites.
  ([Rate limits](../../operators/configuration/rate-limits.md))
- **`username` counts attempts per network.** Guesses from one network cannot lock the name's
  owner out from anywhere else.
- **`username_failures` counts wrong passwords from every network together.** Guessing from many
  networks at once is held too.

## Federation

- **`max_arrivals_per_home_per_day` caps first arrivals from one deployment.** A deployment that
  makes accounts in bulk cannot fill this one with them.
  ([Federation settings](../../operators/configuration/federation.md))
- **Calls to private and loopback addresses are refused unless `allow_private_addresses`.**
  Naming a deployment cannot make the server reach inside its own network.

## Deployment settings

- **`everyone-mention-limit` takes Mention everyone from the everyone role past a size.** One
  `@everyone` cannot reach that many people by accident.
  ([Deployment settings](../../operators/configuration/deployment-settings.md))
