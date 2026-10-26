# The Privacy Policy and Terms of Use

A deployment may publish a Privacy Policy and Terms of Use for anyone to read, signed in or not. Its
administrators write each in Markdown. Aspen gives no default for either.

## Where it lives

| Part           | Code                                                                      |
| -------------- | ------------------------------------------------------------------------- |
| The documents  | `app::deployment_document` (`read`, `publish_as`, `withdraw_as`)          |
| Endpoints      | `api::deployment_document`                                                |
| Terminal       | `aspen-chat-server documents show\|set\|withdraw` (`operator::documents`) |
| Rows           | `deployment_document`, keyed by `kind`                                    |
| Client screens | `client/docs/architecture/documents.md`                                   |

## The documents

- There are two kinds, `privacyPolicy` and `termsOfUse`. Each has one row once published, and none
  before or once withdrawn.
- A body is not blank, and is at most 65,536 characters (`MAX_CHARS`, and the column's check), in
  any script.
- Publishing replaces the body and moves `updated_at`, which the page shows as when it was last
  updated.
- There is no default text. A deployment without one shows "This deployment has not published a
  Privacy Policy." or "This deployment has not published Terms of Use." where it would be.

## Endpoints

| Endpoint                                  | What it does                                                 | Permission                 |
| ----------------------------------------- | ------------------------------------------------------------ | -------------------------- |
| `GET /deployment/documents/{document}`    | The document, with `body` and `updatedAt` `null` without one | None                       |
| `PUT /deployment/documents/{document}`    | Publishes `body` in place of the one before                  | Manage deployment settings |
| `DELETE /deployment/documents/{document}` | Withdraws it; withdrawing one never published does nothing   | Manage deployment settings |

The read asks a request without a session for a device credential, as `GET /deployment` does. The
two writes count toward the `admin` rate limits.

## Who can observe them, and what decides it

- Anyone may read either document, by the endpoint alone. No event carries them, and nothing caches
  them: each page reads the row when it opens.
- Changing one takes Manage deployment settings, checked in `publish_as` and `withdraw_as` against
  the caller's deployment roles as the request arrives.
- Losing Manage deployment settings leaves nothing open to act on: the next write is refused, and
  the dashboard's editor shows the refusal.
- The terminal changes them without an account and announces nothing, since there is no one to tell:
  a reader sees the change the next time a page opens.

## Checks

- `app::deployment_document`'s test publishes, replaces, refuses a blank and an overlong body, and
  withdraws.
- `scripts/check_permissions.py` checks that writing a document takes Manage deployment settings,
  and that taking the role away refuses the next write.
