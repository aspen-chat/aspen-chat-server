# Addresses

An account of this deployment holds at most one address, a row of `user_email` keyed by the user. The row holds the address, when it was verified, and what the account receives there.

## Rules

- Addresses are checked by `app::email::parse_address`: trimmed, at most 254 characters, a mailbox `lettre` accepts.
- Addresses are not unique. Two accounts may share one, and nothing tells anyone whether an address is in use. [Why](design-notes.md#addresses-are-not-unique)
- Bots, the system account, and users of other deployments have none (`Caller::ensure_person`). Their mail is their home's to send.

## Endpoints

| Endpoint | What it does |
| --- | --- |
| `POST /users` with `email` | Gives an address at registration. `newsletter` subscribes, which only counts where the deployment has a newsletter. |
| `PUT /users/{user}/email/address` | Gives or changes the address. |
| `DELETE /users/{user}/email/address` | Removes the address. Refused with `forbidden` while `email_required` is on. |
| `GET /users/{user}/email` | Reads the address and what it receives. |
| `PATCH /users/{user}/email` | Changes the preferences (below). |

## Giving, changing, or removing an address

- It is a change to security settings, since a verified address can reset the password.
- It needs a sign-in verified within `[auth] reverify_seconds` (`reauthenticationRequired`), except at registration.
- A new address starts unverified, with new unsubscribe links.
- When the address it replaced was verified, that address is told (`addressChanged`, naming the new address masked).
- Removing an address deletes the mail waiting to go to it.

Every address given is mailed a code; see [Verification](verification.md).

## Preferences

`PATCH /users/{user}/email` changes these, with merge-patch semantics:

| Field | Values |
| --- | --- |
| `shown` | Whether the profile shows the address (below). |
| `newsletter` | Refused where the deployment has no newsletter. |
| `digest` | Whether the [daily digest](digest.md) is sent. |
| `digestTimeZone` | An IANA name `chrono-tz` knows. |
| `digestHour` | 0 to 23. |

Each change is announced to the user's own subject as `emailAccountChanged`. It names only the user and whether the account now holds an unverified address. Their other devices read the settings again on it.

## The shown address

An account may show its address on its profile.

- `user.public_email` holds the address exactly while it is verified and `shown`. Every read of a user carries it without a join, as `User.publicEmail`.
- `app::email::set_public_email` writes it, inside the transaction that changes either. It publishes a `user` update with `publicEmail` when it changes.
- A foreign user's profile carries the address their home shows: `Profile.public_email` in the sign-in assertion, read and written as the rest of the profile is at each sign-in (see [Federation](../federation/index.md)).
