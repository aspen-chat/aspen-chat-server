# Signing in abroad

A signed-in user gets a signed assertion from their home and hands it to another deployment, which signs them in as a foreign user. This is phase two; the client holding several sessions is phase three.

## Where it lives

| Part | Code |
| --- | --- |
| Assertions and arrivals | `app::federation::abroad` (`count_arrival`) |
| JWS | `app::federation::jws` |
| Verifying statements | `app::federation::received::receive` |
| Avatars | `app::icon` (`IMAGE_TYPES`, `delete_if_unused`), `GET /federation/icons/{icon}` |
| Record of deployments used | `user_foreign_deployment` |

## The flow

1. A signed-in user asks their home for an assertion naming one other deployment: `POST /auth/assertions` with `audience`.
2. The home signs it only when its emigration gate admits that deployment for their kind of account.
3. The home records it in `user_foreign_deployment`. `GET /users/@me/foreign-deployments` reads it, for their other devices. It is not shown to moderators.
4. The user hands the assertion to the other deployment: `POST /auth/federated-sign-in`, unauthenticated.
5. The other deployment verifies it (below) and signs the user in with an ordinary session, owing it no second factor.
6. The first time, it makes their [foreign user](#the-foreign-user).

## The assertion

An assertion is a compact JWS with EdDSA (`app::federation::jws`, RFC 7515 and 8037). Each kind of statement has its own `typ`. It holds:

- the home;
- the audience;
- the user's id at home;
- two minutes of life;
- a single-use id;
- how the sign-in proved who they are (`SignInMethod`: `password`, `secondFactor`, `passkey`, or a bot's `token`), recorded on every `refresh_token`;
- their profile.

## Verifying it

The receiving deployment, in order:

1. reads only the issuer before verifying;
2. contacts that issuer only if an immigration gate might admit it;
3. verifies against the pinned key, contacting again once when that fails, to follow a handover;
4. checks the audience, the lifetime, and the gate for the account's kind;
5. refuses a password-only sign-in where it requires two factors (`strongerSignInRequired`);
6. refuses an assertion that calls a bot a person or a person a bot (`statementInconsistent`), whether by its sign-in method or against the foreign user it already made;
7. remembers the id in Valkey until it expires, so it is used once.

Step 6 matters because the gate it checks is the one for the kind the assertion claims.

## The foreign user

The first sign-in makes a `user` row with `home_domain` and `home_id`, its own id, and no password.

- A registration invite is redeemed when `immigration_invite_required` is set for their kind of account. A dual invite joins its community too (see [Registration invites](../administration/registration-invites.md#dual-invites)).
- At most `[federation] max_arrivals_per_home_per_day` (five hundred) of one home's users and bots arrive for the first time in a UTC day. This is counted in Valkey across every server (`abroad::count_arrival`). The rest are refused (`federationRefused`) until the next day.
- Usernames are unique among a deployment's own users only (`user_name_key` is partial).
- Password sign-in and the terminal look only at the deployment's own users.
- Records carry `homeDomain`.

## The profile

The profile is the home's: its name, display name, pronouns, bio, avatar, and the email address its user shows, if any (see [Email](../email/index.md)).

- Each sign-in writes it and announces changes.
- The user cannot edit it abroad, only their status.

### The name

The name is taken only when it is one an account made here could take (`user::validate_new_username`).

- A first arrival whose name is not is refused with `assertionInvalid` saying why. Names that look like the system account's are among those refused.
- A later name that is not leaves the name held here. So a rule this deployment tightened never signs anyone out, and never stands in the way of retiring or revoking them.

### The avatar

The avatar is copied into the host's storage.

- `home_icon` names the home's original.
- A sign-in naming the original already copied copies nothing.
- The copy a new one replaces is deleted once nothing uses it (`app::icon::delete_if_unused`). So a home cannot fill the host's storage by changing it.
- It is fetched from the home itself at `GET /federation/icons/{icon}`.
- The host takes only PNG, JPEG, WebP, or GIF from it.

`GET /federation/icons/{icon}` on the home:

- serves only its own users' avatars;
- serves only those that are PNG, JPEG, WebP, or GIF (`app::icon::IMAGE_TYPES`, the only kinds an icon may be uploaded as);
- sends `X-Content-Type-Options: nosniff` and `Content-Security-Policy: default-src 'none'; sandbox`, since it answers on the web client's origin.

## What a foreign user may not do

Abroad, a foreign user:

- changes no sign-in security;
- owns no bots;
- signs in nowhere else.

Sessions last as native ones do.

## Refusals

The flow's refusals are `federationRefused` (a closed gate, federation off, or an account that is not the home's), `assertionInvalid`, and `deploymentUnreachable`. Every refusal on this page:

| Code | When |
| --- | --- |
| `federationRefused` | A closed gate, federation off, an account that is not the home's, too many first arrivals today, no common protocol version, or a [ban](standing.md#bans) |
| `assertionInvalid` | An assertion that fails its checks, and a first arrival whose name is not allowed |
| `strongerSignInRequired` | A password-only sign-in where two factors are required |
| `statementInconsistent` | A bot called a person, or a person a bot |
| `deploymentUnreachable` | The home could not be reached. Its detail is always the `statementSenderUnreachable` message; see [The directory](directory.md#failures) |

## The client (phase three)

The client keeps a session on every deployment the home lists, one sync each, and shows them together (`Deployments`; see `client/docs/architecture/deployments.md`).

- One community rail, ordered by the account preference `rail.order` at home.
- One DM list.
- Another deployment's routes are under `/at/{domain}/…`.
- A user stops using a deployment with `DELETE /users/@me/foreign-deployments/{domain}`, which their other devices follow.
- Signing out at home signs out everywhere.
- `GET /auth/methods` names the deployment's `federationDomain`, which a client needs to offer signing in elsewhere.

Browsers call other deployments' APIs directly, so every deployment allows every CORS origin (see [Gates and lists](gates-and-lists.md#cors)).
