# Filing reports

Code: `app::report`, `api::report`.

## The report endpoints

| Endpoint | Reports | Keeps |
| --- | --- | --- |
| `POST /messages/{message}/reports` | A message the reporter can read. An echo is reported as the reply it shows | The message |
| `POST /users/{user}/reports` | A person's profile, naming the aspects found objectionable: `displayName`, `username`, `picture`, `status`, `bio`, `pronouns` | The whole profile as it stood (`ProfileSnapshot`, picture id included) |
| `POST /communities/{community}/members/{user}/nickname/reports` | The nickname a member chose in a community the reporter belongs to (see [Roles and permissions](../roles-and-permissions/index.md)) | The nickname as it stood (`report.nickname`) |

All three:

- Answer `201` with a receipt.
- Share the rate limit bucket `reports`: ten back to back, fifty a day.
- Name a category from `GET /report-categories`.
- May explain themselves in up to `EXPLANATION_MAX_CHARS` (1000) characters. The Other category needs the explanation.

## Refusals

- Nobody reports their own message, profile, or nickname.
- Nobody reports the system account's.
- Reporting a member with no nickname is refused with `reportNoNickname`.
- Someone outside the community finds neither the member nor the nickname.

## Icons kept by reports

An icon a report or warning keeps cannot be deleted through `DELETE /icons/{icon}`. That endpoint takes only its uploader's icons that nothing uses (`app::icon::delete_own_icon`). Nor does the `forgetIcon` job delete it: that job deletes an icon only when nothing uses it, reports and warnings included.

## Categories

The categories are the rows of `report_category`. Categories are never deleted, so reports keep theirs.

| Kind | Named by | Shown | Managed by |
| --- | --- | --- | --- |
| Built-in | `builtin`: `spam`, `harassment`, `hateSpeech`, `violence`, `selfHarm`, `illegalContent`, `impersonation`, `other`. Seeded by the migration | In the reader's language, with a description | Fixed |
| The deployment's own | A name and description in one language | As written | Holders of Manage report categories: `POST`/`PATCH /admin/report-categories`, ordered by `PUT /admin/report-category-order` |

They are offered in this order:

1. The built-in ones, in their order.
2. The deployment's own, by `position`.
3. Other.

Any category but Other may be hidden, which stops it being offered.
