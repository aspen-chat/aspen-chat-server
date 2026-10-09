# Troubleshooting: design notes

Why some answers in the operator's troubleshooting guide
([`docs/operators/troubleshooting/`](../../operators/troubleshooting/index.md)) are what they are.

## Federation errors

- **Someone signing in from another deployment, or a deployment sending a notice, is told only
  that a deployment could not be reached.** Anyone can make this server try any domain that way,
  so the specific cause (no address, private addresses only, a refused connection, a certificate
  problem) goes to this server's log and to the dashboard's Federation tab instead. See
  [`deploymentUnreachable`](../../operators/troubleshooting/federation.md#deploymentunreachable).

## Usernames

- **Usernames are unique regardless of case**, so people can sign in however they type theirs.
  The migration that makes them so names the accounts that clash. See [The server](../../operators/troubleshooting/server.md#aspen-migrate-up-stops-at-usernames-that-differ-only-by-case).
