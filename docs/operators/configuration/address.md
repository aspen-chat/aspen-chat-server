# Address and web client

A deployment is one address. The API, the web client, and the pages the apps open are all
served by each API server, at the same origin.

## The deployment's address

| Setting | Default | |
| --- | --- | --- |
| `public_url` | required | Your deployment's address, an origin alone, such as `https://chat.example.org`. **Once the deployment federates, it can never change**; see [below](#what-follows-from-public_url). |

### What follows from `public_url`

Everything that names the deployment follows from it:

- Invite links, registration links, and the links in mail all point here.
- So do the QR codes for invites and for signing in from another device.
- Passkeys belong to its host.
- Over `https`, its host, with `:port` when not 443, is the deployment's
  [federation](../federation/index.md) domain.

### Changing it

**Other deployments remember the key they find at that domain, so once federating the domain can
never change.**

- The first server to start with an `https` address records the domain in the database.
- A server started with another refuses to start, and says which to set.
- **Changing the host also makes every registered passkey useless.**

### `http` and passkeys

- Passkeys are offered over `https`, and at `localhost` and names under it.
- An `http` address on any other host offers no passkeys.
- An `http` address takes no part in federation.

## `[web_client]`

| Setting | Default | |
| --- | --- | --- |
| `dir` | `client/packages/app/dist` | The built web client (`pnpm build` in `client/`). **The server will not start without it.** |

What the server does with it:

- It serves the web client's files.
- It answers every other path the API does not own with the client's `index.html`.
- That page is given link preview tags (Open Graph). A link to your deployment shared in a chat
  or a post shows your deployment's name and icon, and an invite link shows its community's.
- Files are read as they are asked for, so a new release is served as soon as it is in place.
  See [Releasing a new web client](../installing/5-web-client.md#releasing-a-new-web-client).
