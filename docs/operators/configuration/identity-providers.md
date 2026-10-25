# Identity providers

People can sign in through an OpenID Connect provider you choose: your organisation's Keycloak,
Authentik, Kanidm, Entra ID, Google Workspace, or any provider that publishes a discovery document.
Aspen is only the client. How it works is in
[Identity providers](../../architecture/sign-in-and-security/identity-providers.md).

## Adding a provider

1. Register a client for the deployment at the provider:
   - a confidential client using the authorization code flow, with PKCE allowed;
   - the redirect URI `{public_url}/auth/oidc/callback`;
   - where the provider offers OpenID Connect Back-Channel Logout, the logout URI
     `{public_url}/auth/oidc/back-channel-logout`, with its session id (`sid`) required, so that
     signing out there, or disabling someone, ends their Aspen sign-ins too;
   - the scopes `openid`, `email`, and `profile`.
2. Add it, from the dashboard (holders of Manage identity providers) or the terminal:

   ```
   aspen-chat-server identity-providers add "Example Corp" \
       --issuer https://id.example.org/realms/staff --client-id aspen --client-secret-file secret.txt
   ```

   The secret is read from the file, or from standard input with `-`, never from the command line.
   Adding it fetches the issuer's discovery document, and refuses one it cannot read, saying why.

3. `identity-providers list` shows each provider, how many accounts are linked to it, and the two
   URIs to register.

### Options

| Flag (terminal)         | Dashboard              | Off by default | What it does                                                                                                                                             |
| ----------------------- | ---------------------- | -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--skip-invite`         | Skips invites          | yes            | A first sign-in makes an account without a registration invite where `registration-invite-required` is on. Use it when the provider decides who belongs. |
| `--enforces-two-factor` | Enforces two factors   | yes            | Every sign-in there counts as two factors. Without it, one counts as two only when the provider's `amr` claim names a second factor.                     |
| `--distrust-email`      | Trusts email addresses | no (trusted)   | Without it, an address the provider reports as verified becomes a new account's verified address.                                                        |
| `--disabled`            | Enabled                | no (enabled)   | A disabled provider offers no sign-in, and keeps its links.                                                                                              |

`identity-providers set <name or id>` changes any of them, `--name` renames it, and
`--client-secret-file` replaces the secret. A provider's issuer and client ID are fixed once it is
added. To move to another issuer or client, add a new provider, have its people link it from their
security settings, and then remove the old one.

### Removing a provider

`identity-providers remove <name or id>`, or the dashboard, removes it. Its links are deleted in the
background, and the sign-ins it made last until they end. **Accounts whose only way in was that
provider can no longer sign in**: ask their people to set a password or add a passkey first.

## `[identity_providers]`

| Setting                   | Default |                                                                                                                                        |
| ------------------------- | ------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `allow_private_addresses` | `false` | Whether a provider may be on a private or local network. Off, the server calls providers only on the public internet.                  |
| `ca_file`                 | none    | Certificate authorities, in PEM, trusted for providers besides the system's: your organisation's own, if it issued the provider's one. |

Turn `allow_private_addresses` on for a provider on your LAN. While it is on, anyone holding Manage
identity providers can make the server fetch a discovery document from any address it can reach, so
grant that permission accordingly.

## What must be reachable

- People's browsers reach `{public_url}/auth/oidc/callback`, and the provider's server reaches
  `{public_url}/auth/oidc/back-channel-logout`. A reverse proxy in front of the API server must pass
  both through, as it passes everything else under `public_url`.
- The API servers reach the provider's issuer, token endpoint, and keys over HTTPS.
