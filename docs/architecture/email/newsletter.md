# The newsletter and unsubscribing

Where the deployment setting `newsletter_enabled` is on, accounts may subscribe to a newsletter, and its writers send posts (`app::email::newsletter`).

## Subscribing

- At registration (the box starts unticked).
- In the account's settings (`newsletter` in the [preferences](addresses.md#preferences)).

## Writing posts

- Holders of Send newsletters write posts at `/admin/newsletter/posts`.
- The permission is `DeploymentPermission::SendNewsletters`, bit 14. The migration gave it to every role holding Manage deployment settings.

A post is Markdown. It is sent:

- as HTML made by `pulldown-cmark` (`newsletter::html`), which shows HTML written into the Markdown as text and drops links that are not `http`, `https`, or `mailto`;
- and as the Markdown itself.

## A post's life

| Action | Endpoint or effect |
| --- | --- |
| Edit, delete | Drafts only. |
| Preview | The record carries its `html`. |
| Send a test | `POST …/test`, to its sender's verified address. |
| Send | `POST …/sending` fixes the post and marks it sent, saving a `queueNewsletter` job (bulk) beside it. |

After sending:

1. The `queueNewsletter` job (see [Jobs](../jobs/index.md)) queues its mail a thousand subscribers at a time, in order of their ids (`newsletter::queue_step`, `queued_through`, `recipients`). Each batch's mail is saved in one statement. So a newsletter to a million subscribers is never one transaction.
2. Sent posts stay, as the archive.

## Unsubscribing

Each digest and newsletter carries:

- `List-Unsubscribe` and `List-Unsubscribe-Post: List-Unsubscribe=One-Click` (RFC 8058);
- a link at its foot.

All go to `/email/unsubscribe?list={newsletter|digest}&token={token}` on `public_url`.

- The token is random per address (`user_email.unsubscribe_token`), replaced when the address changes.
- A `GET` there, which mail scanners follow, shows a page asking to confirm (`api::email::unsubscribe_page`).
- A `POST` (the page's button, or a mail program's one-click unsubscribe) unsubscribes and says so. It announces `emailAccountChanged`.
- Both are served outside `/api/v1` and limited like the API.
