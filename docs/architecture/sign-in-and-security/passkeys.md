# Passkeys

Passkeys are WebAuthn through `webauthn-rs`.

| Part | Where |
| --- | --- |
| Ceremonies | `app::passkey` |
| Hand-off page at `/auth/passkey` (outside `/api/v1`) | `api::passkey_page` |
| The sign-in a ceremony acts on | `login::sign_in_id` |
| A second factor's ticket key | `login::TicketKey` |

## What a passkey does

- It signs its user in on its own.
- It also serves as the second factor after a password.
- Registration requires a discoverable credential, so sign-in needs no username.
- An account holds at most 50 passkeys (`passkey::MAX_PASSKEYS`). The cap is checked as a `register` ceremony starts, and again, with the account's row held, as it completes.

## Ceremonies

Every exchange with an authenticator is a ceremony:

1. `POST /auth/passkey-ceremonies` with a purpose (`signIn`, `register`, `reauthenticate`) returns the options for `navigator.credentials`.
2. The verifier's state waits in Valkey for five minutes.
3. `POST /auth/passkey-ceremonies/{id}/credential` completes it once, whether or not it verifies.

## Hand-off to the system browser

A browser offers a passkey only to pages under `rp_id`. The desktop and mobile apps are not such pages, so they hand the ceremony to the page this server serves at `/auth/passkey`, in the system browser:

1. The app starts the ceremony with a `handoff` naming a return address and a PKCE `S256` challenge. The return address is a loopback port or the `aspen:` scheme, never a web page.
2. The page completes the ceremony. This only checks the credential.
3. The page sends the browser back to the return address with a one-time return `code` added.
4. The app claims the outcome with `POST …/claim`, its verifier, and that code.

Only the claim gives the ceremony its effect: a session issued, a passkey added, or a sign-in marked verified.

Rules of the hand-off:

- The ceremony id travels in the page's URL fragment, so it reaches no server log.
- The claim needs the return code. An app that sends none is told to update.
- The page tells whoever opens it to continue only for a request they made in the app on that device.
- The hand-off page runs only handed-off ceremonies.

**Why the return code:** whoever starts a ceremony can send its link to someone else. The code makes sure a ceremony is claimed only on the device where it was run. See [design notes](design-notes.md#the-return-code).

## Who may complete a ceremony

- A `register` or `reauthenticate` ceremony acts on the sign-in that started it (`login::sign_in_id`). Only that sign-in completes it in its own page, or claims it after a hand-off. Anyone else gets `forbidden`.
- A ceremony in Valkey keeps that sign-in id, never the sign-in's tokens.
- A `signIn` ceremony that is a password sign-in's second factor keeps the ticket's key (`login::TicketKey`, holding its digest), never the ticket.
- Re-verifying marks the sign-in by its id, and refuses one that has ended.

Starting a ceremony is rate limited per address and by everyone's starts together, as starting a device link is (see [Device links](device-links.md#rate-limits)).
