# Identity providers

People sign in through an OpenID Connect provider the deployment's administrators chose: a company's
or a school's Keycloak, Authentik, Kanidm, Entra ID, Google Workspace, or any other that publishes a
discovery document. Aspen is the relying party (the client) only. It issues no tokens to anyone
else.

| Part                                    | Where                                                            |
| --------------------------------------- | ---------------------------------------------------------------- |
| Providers and their administration      | `app::identity_provider`                                         |
| Calling providers, documents, and keys  | `app::identity_provider::discovery`                              |
| Sign-in flows                           | `app::identity_provider::flow`                                   |
| Linked identities                       | `app::identity_provider::identity`                               |
| Back-channel logout                     | `app::identity_provider::logout`, `login::end_provider_sign_ins` |
| HTTP endpoints                          | `api::identity_provider`                                         |
| The callback and back-channel endpoints | `api::identity_provider` (`/auth/oidc/…`, outside `/api/v1`)     |
| Terminal                                | `operator::identity_providers`                                   |
| Stand-in provider for checks            | `scripts/oidc_provider_stand_in.py`                              |

## Providers

A provider is a row of `identity_provider`:

- `name`: what the sign-in screen's button says ("Continue with Example Corp").
- `issuer`: the issuer URL. Its discovery document is `{issuer}/.well-known/openid-configuration`,
  and the document must name the same issuer. Fixed once the provider is added.
- `client_id` and `client_secret`: the client the provider's administrators registered for this
  deployment. The client id is fixed once the provider is added. The secret may be replaced, and is
  never shown again once saved.
- `enabled`: whether it offers sign-in now. A disabled provider keeps its links.
- `skip_invite`: whether a first sign-in makes an account without a registration invite where one is
  required (see [New accounts](#new-accounts)).
- `enforces_two_factor`: whether every sign-in there counts as two factors whatever its `amr` claim
  says (see [Strength](#strength)).
- `trust_email`: whether an address it reports as verified becomes the account's verified address.

The client is registered at the provider with one redirect URI, `{public_url}/auth/oidc/callback`,
and, for [back-channel logout](#back-channel-logout), `{public_url}/auth/oidc/back-channel-logout`.
The dashboard shows both.

A deployment has at most `MAX_PROVIDERS` (20). Holders of the deployment permission Manage identity
providers add, change, and remove them in the dashboard (`/admin/identity-providers`), and the
terminal does the same (`aspen-chat-server identity-providers list|add|set|remove`). Adding one
fetches its discovery document first, so a mistyped issuer is refused with what went wrong. A
provider's name, secret, and choices may change later; its issuer and client id may not. A `sub` is
unique only within one issuer, and a provider may give each client its own subjects, so an issuer or
client changed under existing links would sign whoever holds those subjects at the new one into the
linked accounts. Moving to another issuer or client is adding a provider and having its people link
it.

### Removing a provider

Removing marks the row removed (`removed_at`) and saves a job (`forgetIdentityProvider`) deleting
its links a batch at a time. The row stays, so the sign-ins it made keep naming it. Sign-ins it
already made last until they end in any of the usual ways. Accounts whose only way in was that
provider can no longer sign in, so the dashboard asks for confirmation, naming how many accounts are
linked (counted up to 1000).

### Discovery, keys, and outbound calls

Each API server caches each provider's discovery document and keys for an hour
(`discovery::document`). A token signed with a key the cache lacks (its `kid` is not among them, or
it names none and none verifies it) fetches them again, so a provider's key rotation needs no
restart: at most once a minute per issuer on each server, the one fetch shared by every token that
arrives while it is made (`discovery::document_for_unknown_key`). Anyone may post a logout token
naming any key, and without that bound each would cost the provider two calls. A token whose `kid`
is among the cached keys and does not verify is refused without fetching anything.

Calls to providers are HTTPS only, follow no redirects, take at most ten seconds each, and read
answers of at most 1 MiB. Why one failed is classified (`aspen_outbound::classify`, as calls to
other deployments are) into a message the administrator can act on. An issuer that resolves to an
address inside a network (a LAN Keycloak) is refused unless
`[identity_providers] allow_private_addresses` is set in `aspen.toml`: otherwise anyone holding
Manage identity providers could make the server call services only it can reach. Names are checked
as they resolve (`PublicResolver`); a URL naming an address outright is never resolved, so every
call (`discovery::call`, for the issuer and for each endpoint its document names: keys, token)
checks it first, refusing one inside a network unless private addresses are allowed, and a loopback
or unspecified one (this machine) always. `[identity_providers] ca_file` adds authorities to trust,
for a provider whose certificate an organisation issued itself.

## Flows

Every exchange with a provider is a flow: the authorization code flow with PKCE (RFC 7636) and a
nonce, the provider's code exchanged with the client secret. A flow has a purpose:

| Purpose          | Needs                                   | Takes effect                                                        |
| ---------------- | --------------------------------------- | ------------------------------------------------------------------- |
| `signIn`         | No session                              | Signs into the linked account, or offers to make one                |
| `link`           | A recently verified session of a person | Links the provider's identity to the caller's account               |
| `reauthenticate` | A session of a person                   | Marks the sign-in verified, when the provider signed them in afresh |

1. The client starts a flow: `POST /auth/identity-provider-flows` with the provider, the purpose, a
   PKCE `S256` challenge of its own, and for the desktop and mobile apps a return address. It
   receives the flow's id and the provider's authorization URL.
2. The client opens that URL: the web client in its own tab, the apps in the system browser.
3. The provider sends the browser to `GET /auth/oidc/callback` with `state` (the flow's id) and a
   code. The server exchanges the code with the provider (with its own PKCE verifier, which the
   client never sees), verifies the ID token (signature, issuer, audience, expiry, nonce), and keeps
   the outcome, or the reason it failed.
4. The server sends the browser to the return address with a one-time return code: the app's
   (loopback or `aspen:`, as for [passkeys](passkeys.md#hand-off-to-the-system-browser)), or the web
   client's page `/sign-in/provider`, with the flow and the code in the URL fragment.
5. The client claims the outcome: `POST /auth/identity-provider-flows/{flow}/claim` with its PKCE
   verifier and the return code.

The flow is kept in Valkey for ten minutes, and its outcome five minutes more for the claim. The
callback takes the flow once, so a second callback for it finds nothing. A callback without a flow
to return to shows a page saying the sign-in expired.

**Why the claim needs both secrets:** whoever starts a flow can send its authorization URL to
someone else. The verifier alone would let them claim what that person's provider said: a session in
that person's account. The return code goes only to the return address, on the device whose browser
ran the flow (the app's own, or this deployment's own page in that browser). A flow is therefore
claimed only where it was run, by whoever started it.

**Why `state` may be the flow's id:** it passes through the provider and may reach its logs. Knowing
it gives nothing: the claim needs the client's verifier and the return code, and the provider's code
is exchanged with a PKCE verifier only the server holds, so a code someone else obtained cannot be
spent on this flow.

### Signing in

The claim of a `signIn` flow:

- **A linked account:** the identity (`user_identity`, by provider, issuer, and `sub`) names it. The
  account is signed in. If the sign-in counts as one factor and the account holds a second factor,
  it is asked for, as after a password (`secondFactorRequired`, with a ticket). The sign-in counts
  as verified (`verified_at`, see [Reverification](reverification.md#recent-verification)) when the
  ID token's `auth_time` says the person last signed in at the provider, or never when it says
  nothing. A provider signs someone in silently from a session it has held for days, so whoever
  finds that session open in a browser gets a sign-in that must still verify again (a
  `reauthenticate` flow, which makes the provider ask) before any security change. A second factor
  asked for here is proved now, and the sign-in it finishes is verified now.
- **No linked account:** the answer is `accountRequired`, with a suggested username (the provider's
  `preferred_username`, when it is one an account could take), its display name, whether an invite
  is needed, and the address the account will have or whether one must be given. The person chooses
  a username, and the client makes the account with
  `POST /auth/identity-provider-flows/{flow}/account` and the same verifier. A name someone holds
  (`usernameTaken`) leaves the offer standing for another try. A flow makes at most one account, and
  only within the five minutes after its claim.

An identity is matched by provider, issuer, and `sub`, never by email address. Each link keeps the
issuer it was made at (`user_identity.issuer`), and every lookup (signing in, verifying again,
back-channel logout, the security settings' list, `has_way_in`, the dashboard's count) asks for the
provider's current issuer, so a provider row that came to name another issuer by any route finds
none of the accounts linked before. A flow's outcome records the issuer its ID token was verified
against, and a claim made after the provider's issuer changed is refused as expired. An address can
be reused, renamed, or asserted by a provider that does not own it, so matching by it would let one
person's provider sign in as someone else.

### New accounts

A new account:

- is made as registration makes one (`app::user::create_account`), with the same rules for its name
  and the same rate limits as `POST /users`;
- needs a registration invite where `registration_invite_required` is set, unless the provider has
  `skip_invite`;
- has no password. Its way in is its provider. It may set a password later from its security
  settings, after verifying again;
- gets the provider's verified address as its verified address, when the provider has `trust_email`,
  reports `email_verified`, and the deployment takes email. Otherwise it follows the usual rules
  (`email_required`);
- gets the provider's `name` as its display name, when it is one a display name may be.

### Linking and unlinking

A person adds a provider to an existing account from their security settings with a `link` flow. It
needs a recent verification, and is claimed only by the sign-in that started it. An account holds at
most one identity of each provider, and an identity belongs to one account; linking one another
account holds is refused (`identityTaken`).

The security settings (`GET /users/{user}/security`) list the caller's links as `identities`, and
say whether the account has a password (`hasPassword`). `DELETE /users/{user}/identities/{identity}`
removes one. It needs a recent verification, and is refused when it would leave the account no way
in: no password, no passkey, and no other linked provider (`lastSignInMethod`). Removing a passkey
is refused the same way (`identity_provider::has_way_in`, checked with the account's row held).

An account without a password sets its first with `PUT /users/{user}/password`, leaving out
`oldPassword`, with a recently verified session. Password sign-in checks an account without one
against a hash no password matches, so it takes as long as a wrong password.

### Verifying again

An account may verify again through any provider it is linked to with a `reauthenticate` flow. It
asks the provider to sign the person in afresh (`prompt=login`, `max_age=0`), and counts only when
the ID token's `auth_time` is no earlier than the flow's start. A provider that sends no `auth_time`
cannot verify anyone again. An account holding a second factor verifies again this way only when the
provider's sign-in [counts as two factors](#strength), as a password alone does not do for it.

## Strength

A provider's sign-in counts as two factors (`SignInMethod::IdentityProviderTwoFactor`) when the
provider has `enforces_two_factor`, or its ID token's `amr` claim names a second factor (`mfa`,
`otp`, `hwk`, `swk` with another method, `sms`, or `fpt`, `face`, `iris`, `retina`, or `vbm` with
another method; RFC 8176). Otherwise it counts as one (`SignInMethod::IdentityProvider`):

- `require_two_factor` holds it like a password sign-in: such a session must add a second factor of
  Aspen's own before anything else.
- An account holding a second factor is asked for it.

Signing in abroad carries the strength rather than the provider: an assertion names a one-factor
provider sign-in `password` and a two-factor one `secondFactor`, so other deployments read it as
they read any other (see [Signing in abroad](../federation/signing-in-abroad.md)).

## Back-channel logout

A provider that supports OpenID Connect Back-Channel Logout posts a logout token to
`POST /auth/oidc/back-channel-logout` when someone signs out there or is removed. The server
verifies it (a `typ` of `logout+jwt` when it has one, an asymmetric signature by the one provider of
its issuer, issuer, audience, `iat`, the logout event, no `nonce`, used once by `jti`) and ends:

- the sign-ins made through that provider with its session id (`sid`), when the token names one;
- otherwise every sign-in made through that provider by the account its `sub` is linked to.

Each sign-in records the provider and the provider's `sid` (`refresh_token.identity_provider`,
`identity_provider_session`). Sign-ins made another way (a password, a passkey, a device link from a
password sign-in) are untouched. A device link copies the provider and session id of the sign-in
that gave it, so it ends with it.

Without back-channel logout, a sign-in made through a provider lasts as any other does, however the
person's standing there changes.

## Rate limits

All in `rate_limits.toml`:

| Endpoint                                     | Limits                                                                            |
| -------------------------------------------- | --------------------------------------------------------------------------------- |
| `POST /auth/identity-provider-flows`         | The `sign_in` group, and by everyone's starts together, as passkey ceremonies are |
| `POST …/claim`                               | The `sign_in` group; wrong verifiers and codes count as failures                  |
| `POST …/account`                             | As `POST /users`, and sharing its global bucket (`registration`)                  |
| `GET /auth/oidc/callback`                    | Per address, in the `sign_in` bucket. Device-exempt: the browser opens it         |
| `POST /auth/oidc/back-channel-logout`        | Per address, and refused tokens per address. Device-exempt: the provider posts it |
| `DELETE /users/{user}/identities/{identity}` | The `security` group                                                              |
| `/admin/identity-providers`                  | The `admin` group                                                                 |

Starting a flow, claiming it, and making its account are in `fail_closed`.

## When access is given or taken away

1. **Who can observe a flow?** Whoever holds its verifier and return code: the device that started
   and ran it. Its id alone reads nothing. A link is read only by its account (its security
   settings). Providers' settings are read only by holders of Manage identity providers; the secret
   by no one. `GET /auth/methods` names the enabled providers to everyone.
2. **What decides it?** For a sign-in, the provider's ID token, and then everything `issue_session`
   checks (deletion, a ban from the deployment). For a link or a reauthentication, the sign-in that
   started it (`login::sign_in_id`), which must still be live when the claim is made.
3. **When access is lost:**
   - Disabling a provider stops new flows and fails flows waiting at the callback. Removing it also
     deletes its links (a job). Sign-ins it made last until they end.
   - Back-channel logout ends the sign-ins it names (see above), announced as `signInsEnded`.
   - Unlinking stops new sign-ins through that identity; sign-ins it made stay.
   - Signing out, a password change, a ban, or deletion end a provider's sign-ins as they end any
     other. Deleting an account deletes its links, so the identity can make a new account.
   - Losing Manage identity providers takes away the dashboard's provider pages at the next request.
4. **When it is gained:** the claim's answer tells the client. A provider added or enabled appears
   on sign-in screens when they next read `GET /auth/methods`.
5. **Does every path announce it?** Ending sign-ins publishes `signInsEnded` inside the transaction,
   from the dashboard, the terminal, back-channel logout, and the removal job alike. Links and
   providers have no events; the security settings and the dashboard read them when they open.
6. **Is it published inside the transaction?** Yes, `signInsEnded` is published inside the
   transaction that ends the sign-ins.

`scripts/check_permissions.py` (`identity_providers`) checks these against a stand-in provider
(`scripts/oidc_provider_stand_in.py`): signing in and making an account, back-channel logout by
`sid` and by `sub` and what it leaves, logout tokens used again, from an unknown issuer, carrying a
nonce, or changed after signing, unlinking the last way in, verifying again (and not as someone
else), unlinking, a link whose starting sign-in ended, a disabled provider, and a removed one's
links going.
