# Upgrading

## Steps

1. Build the new binaries and web client.
2. Run `aspen-migrate up`.
3. Restart the API servers, then the voice servers. Calls on a voice server that restarts end, and
   their clients rejoin on their own.
4. Copy the new web client over the old on each API server (see
   [Releasing a new web client](5-web-client.md#releasing-a-new-web-client)).

People's event streams reconnect by themselves when an API server restarts, and pick up exactly
where they left off.
