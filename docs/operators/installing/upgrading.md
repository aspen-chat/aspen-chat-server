# Upgrading

## Steps

1. Build the new binaries and web client.
2. Check that the database is still a PostgreSQL the release supports ([Step 1](1-services.md)), and
   upgrade it first when it is not: a migration fails on what an older version lacks.
3. Run `aspen-migrate up`.
4. Restart the API servers, then the voice servers. Calls on a voice server that restarts end, and
   their clients rejoin on their own.
5. Copy the new web client over the old on each API server (see
   [Releasing a new web client](5-web-client.md#releasing-a-new-web-client)).

People's event streams reconnect by themselves when an API server restarts, and pick up exactly
where they left off.
