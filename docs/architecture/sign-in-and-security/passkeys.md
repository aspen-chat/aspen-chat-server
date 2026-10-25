# Passkeys

Passkeys are WebAuthn through `webauthn-rs`.

| Part                                                 | Where                                       |
| ---------------------------------------------------- | ------------------------------------------- |
| Ceremonies                                           | `app::passkey`                              |
| Hand-off page at `/auth/passkey` (outside `/api/v1`) | `api::passkey_page`                         |
| The sign-in a ceremony acts on                       | `login::sign_in_id`                         |
| A second factor's ticket key                         | `login::TicketKey`                          |
| Android apps that run ceremonies themselves          | `passkey::android_apps`, `api::asset_links` |

## What a passkey does

- It signs its user in on its own.
- It also serves as the second factor after a password.
- Registration requires a discoverable credential, so sign-in needs no username.
- An account holds at most 50 passkeys (`passkey::MAX_PASSKEYS`). The cap is checked as a `register`
  ceremony starts, and again, with the account's row held, as it completes.

## Ceremonies

Every exchange with an authenticator is a ceremony:

1. `POST /auth/passkey-ceremonies` with a purpose (`signIn`, `register`, `reauthenticate`) returns
   the options for `navigator.credentials`.
2. The verifier's state waits in Valkey for five minutes.
3. `POST /auth/passkey-ceremonies/{id}/credential` completes it once, whether or not it verifies.

## Hand-off to the system browser

A browser offers a passkey only to pages under `rp_id`. The desktop and mobile apps are not such
pages, so they hand the ceremony to the page this server serves at `/auth/passkey`, in the system
browser:

1. The app starts the ceremony with a `handoff` naming a return address and a PKCE `S256` challenge.
   The return address is a loopback port or the `aspen:` scheme, never a web page.
2. The page completes the ceremony. This only checks the credential.
3. The page sends the browser back to the return address with a one-time return `code` added.
4. The app claims the outcome with `POST …/claim`, its verifier, and that code.

Only the claim gives the ceremony its effect: a session issued, a passkey added, or a sign-in marked
verified.

Rules of the hand-off:

- The ceremony id travels in the page's URL fragment, so it reaches no server log.
- The claim needs the return code. An app that sends none is told to update.
- The page tells whoever opens it to continue only for a request they made in the app on that
  device.
- The hand-off page runs only handed-off ceremonies.

**Why the return code:** whoever starts a ceremony can send its link to someone else. The code makes
sure a ceremony is claimed only on the device where it was run. See
[design notes](design-notes.md#the-return-code).

## Who may complete a ceremony

- A `register` or `reauthenticate` ceremony acts on the sign-in that started it
  (`login::sign_in_id`). Only that sign-in completes it in its own page, or claims it after a
  hand-off. Anyone else gets `forbidden`.
- A ceremony in Valkey keeps that sign-in id, never the sign-in's tokens.
- A `signIn` ceremony that is a password sign-in's second factor keeps the ticket's key
  (`login::TicketKey`, holding its digest), never the ticket.
- Re-verifying marks the sign-in by its id, and refuses one that has ended.

Starting a ceremony is rate limited per address and by everyone's starts together, as starting a
device link is (see [Device links](device-links.md#rate-limits)).

## Telling passkey managers what the server holds

Browsers' passkey managers keep passkeys the server has forgotten, and names it has changed, unless
told. WebAuthn's Signals API tells them. Only a page under `rp_id` may send a signal, so the web
client sends them, and the hand-off page sends the one it can. Each is feature-detected and dropped
quietly where the browser lacks it or refuses it.

| Signal                         | Sent                                                                                                           | From                                                 |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| `signalUnknownCredential`      | A sign-in names a credential no account holds (`passkeyUnknown`)                                               | The web client's ceremonies, and the hand-off page's |
| `signalAllAcceptedCredentials` | Every time Sign-in and security reads its settings: opening, and after a passkey is added, renamed, or removed | The web client                                       |
| `signalCurrentUserDetails`     | After the display name (or username) changes, for an account with passkeys                                     | The web client                                       |

What the server gives them:

- `passkeyUnknown`: a `signIn` ceremony whose credential the user handle's account does not hold,
  removed or never registered, or whose account is deleted. It is told apart before the ticket of a
  second factor is checked, so another account's passkey is never reported unknown.
- `Passkey.credentialId`: the WebAuthn credential id, base64url.
- `SecuritySettings.passkeyUserHandle`: the user handle every passkey of the account holds,
  `passkey::user_handle`, the account id's 16 bytes in base64url.

The hand-off page learns neither the account's list nor its names, so it sends only
`signalUnknownCredential`. The Android app's own ceremonies send none.

### Who may read them

1. **Who observes it.** The account's own sessions, through `GET /users/{user}/security` (the user
   themself only, `403` for anyone else) and the `Passkey` records its ceremonies and renames
   return. No event carries them, nothing is pushed, and no sideload includes them. The user handle
   is the account's id, which is no secret.
2. **What decides it.** The session alone (`EnrollingSessionUser`), checked in `api::security`.
3. **When it is lost.** A session that ends (signing out, a password change, signing out everywhere
   else, a ban, deletion) reads nothing more: `401`. The page signals only what it last read, and
   the server keeps nothing on the client's behalf.
4. **When it is gained.** A new passkey is in the next read, which the panel makes after every
   change.
5. **Every path that changes it.** Adding, renaming, and removing a passkey are the account's own
   requests, after which the panel reads again. An account deleted holds no passkeys
   (`passkey::credentials` skips it), and a sign-in with one is `passkeyUnknown`.
6. Nothing is published.

`scripts/check_permissions.py`'s `passkeys` scenario checks these with a passkey held in software
(`scripts/software_passkey.py`), and that an Android app's origin is accepted only for the apps
named.

## Autofill in the web client

The web client's sign-in form offers passkeys among the username field's suggestions
(`autocomplete="username webauthn"`, WebAuthn's conditional mediation), in a page under `rp_id`
whose browser has it (`isConditionalMediationAvailable`).

1. As the form opens, a `signIn` ceremony starts (`useConditionalPasskey`,
   `AspenClient.runConditionalPasskeySignIn`), and the browser waits with it.
2. Picking a passkey completes it as the button's would.
3. Signing in another way aborts it first: a password, the passkey button, or leaving the form.
4. It ends 15 seconds before the ceremony expires, so a pick never reaches the server after.

Starting a ceremony counts toward the `sign_in` limits of the device and the address, and toward
everyone's (`POST /auth/passkey-ceremonies`). So starts follow what the person does, never a timer:
the form starts one as it opens, and another only when the person returns to the username field with
none waiting. A page left open starts one and no more. A start the server refuses is silent, and the
button stays.

## Android apps

Android lets an app use a domain's passkeys itself, through Credential Manager, once the domain
names it at `/.well-known/assetlinks.json` (Digital Asset Links). The authenticator then signs the
app's own origin rather than a page's, `android:apk-key-hash:` and the base64url SHA-256 of its
signing certificate.

| Part                                  | Where                                                                               |
| ------------------------------------- | ----------------------------------------------------------------------------------- |
| The apps named                        | `passkey::android_apps`: `OFFICIAL_ANDROID_FINGERPRINTS`, and `[auth] android_apps` |
| Their origins, accepted in ceremonies | `passkey::relying_party`                                                            |
| The statements                        | `api::asset_links` at `GET /.well-known/assetlinks.json`                            |
| What clients read                     | `AuthMethods.passkeys.androidApps`                                                  |

- Each app is a package and a certificate's fingerprint, one statement per package with the relation
  `delegate_permission/common.get_login_creds`, which Credential Manager needs to offer a site's
  passkeys to an app.
- The official app's release certificates are built in. None exists yet; each debug build has its
  builder's own key, which an operator names in `[auth] android_apps`.
- Without `rp_id` no app is named, and the statements are an empty list.
- The page is rate limited per address (`GET /.well-known/assetlinks.json` in `rate_limits.toml`)
  and needs no device credential: Google's servers read it.

The app's side is in the client's
[sign-in page](../../../client/docs/architecture/sign-in.md#passkeys). Its ceremonies start and
complete as a page's do, by the sign-in that started them, with no hand-off.

The app's origin is the same for every deployment that names it, so this server cannot tell an
assertion the app made for it from one made for options another deployment relayed from one of its
ceremonies. The app holds the line a browser holds for a page: it runs a ceremony natively only when
the options' relying party (`rpId`, or `rp.id` to register) is the deployment's own host or a parent
of it, and otherwise hands it to that deployment's page in the system browser. Shipping a release
certificate in `OFFICIAL_ANDROID_FINGERPRINTS` relies on that check.
