# Upgrading

## Steps

1. Build the new binaries and web client.
2. Run `aspen-migrate up`.
3. Restart the API servers, then the voice servers. Calls on a voice server that restarts end,
   and their clients rejoin on their own.
4. Copy the new web client over the old on each API server (see
   [Releasing a new web client](5-web-client.md#releasing-a-new-web-client)).

People's event streams reconnect by themselves when an API server restarts, and pick up exactly
where they left off.

## From shared-secret join tokens

Deployments whose `aspen.toml` has a `[voice] token_secret` signed join tokens with that secret.
API servers of this release sign them with a key of their own, and ignore the setting.
Upgrade the voice servers first, while they still have the secret, then the API servers.

Before step 1, add `aspen.voice.token-key` to each voice server's NATS permissions (see
[Step 6: Voice servers](6-voice-servers.md#give-it-a-nats-user)).

1. Run `aspen-migrate up`. Then restart each voice server on the new build, with its
   `token_secret` still in `voice_server.toml`. It takes both kinds of token, and warns that the
   secret is set.
2. Restart the API servers on the new build. The first to start makes the key. Each answers the
   voice servers asking for it, and every join token from then on is signed with it.
3. Take `token_secret` out of every `voice_server.toml` and out of `aspen.toml`, and restart the
   voice servers. A voice server without it refuses tokens of the old kind.

Calls go on throughout, apart from those on each voice server as it restarts, whose clients
rejoin on their own.
