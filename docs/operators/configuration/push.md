# Waking phones

## `[push]`

| Setting | Default | |
| --- | --- | --- |
| `enabled` | `true` | Whether the Aspen app on phones may ask to be woken when it is not open, for DMs and messages that tag someone. |

How phones are woken:

- Through the relay of whoever published their app (the Aspen Foundation's, for the published
  apps), or through a UnifiedPush distributor.
- This server calls them over HTTPS, as it calls other deployments.
- What it sends is encrypted to the phone, and says only which channel and message to fetch.
