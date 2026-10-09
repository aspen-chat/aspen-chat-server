# Evidence

| Part | Where |
| --- | --- |
| Deleting a message | `app::message::soft_delete` |
| Marking attachments as evidence | `app::attachment::evidence` (`keep_deleted`, `keep_removed`) |
| Moving evidence in storage | `moveEvidence` job (`evidence::move_step`), `media_store::EVIDENCE_PREFIX` |
| Reading evidence for review | `evidence::read_for_review`, `api::attachment::attachment_for_review`, `evidence::URL_LIFETIME` |
| Purging after retention | `purgeEvidence` job (`evidence::purge_step`, `evidence::HELD_SQL`), `link_preview::purge_kept_pictures` |
| Purging at once | `aspen-chat-server attachments purge`, `moderation_log::log_operator_moderation` |

## What deleting a message keeps

Deleting a message hides it rather than erasing it (`app::message::soft_delete`). Its text, attachments, and link previews stay, so the reviews and warnings that show it still can. Every other read treats it as gone.

## Attachments that become evidence

These attachments become evidence, in the transaction that makes the change:

- A deleted message's attachments that no standing message holds.
- Any attachment taken off a message (by an edit or `DELETE /messages/{message}/attachments/{attachment}`) and in no other message.

`attachment.evidence_at` marks them. From then on:

- No read but a review's returns them. `read_attachments`, `read_attachment`, and plugins' `read-attachment` leave them out.
- No message may take them up again.
- No preview is made of them.

## The mover

Within about five seconds, the recurring `moveEvidence` job (`evidence::move_step`; see [Jobs](../jobs/kinds.md#moveevidence)) moves the original and its preview from the anonymous read path to `evidence/` (`media_store::EVIDENCE_PREFIX`), and records the new keys.

The move is a copy and a deletion. The new keys are recorded only where the recorded keys are still the old ones. So servers moving at once do no harm.

**Warning:** the anonymous read path must never serve `evidence/` (see [The storage's read path](../../operators/installing/storage-read-path.md)).

## Reading evidence

- A case's messages name the attachments taken off them (`ReviewedMessage.removedAttachments`).
- A review's `attachments` give evidence at URLs on the S3 API signed for ten minutes (`evidence::URL_LIFETIME`, `api::attachment::attachment_for_review`).
- The warned person's copy of the warned message names none.

## Purging evidence

Nothing over the API deletes evidence.

### After the retention period

The recurring job `purgeEvidence` (`evidence::purge_step`, daily; see [Jobs](../jobs/index.md)) deletes evidence once the deployment setting `evidence_retention_days` has passed since it became evidence.

| Setting | Default | Meaning |
| --- | --- | --- |
| `evidence_retention_days` | 365 | Days evidence is kept. 0 keeps it for good. |

- It deletes oldest first, through the index `attachment_evidence_at`.
- It keeps evidence whose message a report case is about while that case is open, or closed within the retention period (`evidence::HELD_SQL`, through `report_case_message`).
- On the same terms it deletes the pictures of deleted messages' link previews, keeping the previews' text (`link_preview::purge_kept_pictures`). It finds them through `message_link_preview.message_deleted_at`, which deleting a message writes. Until then those pictures stay on the public read path.
- It writes nothing to the moderation log. **Why:** see [design notes](design-notes.md#evidence).

### At once, from the terminal

An operator deletes evidence at once with:

- `aspen-chat-server attachments purge --message <id>`: every piece of a message's evidence.
- `aspen-chat-server attachments purge --attachment <id>`: one attachment.

The purge deletes the rows and every object at the keys they may be at. It writes each to the moderation log as `purgeAttachment` with no actor (`moderation_log::log_operator_moderation`). An attachment a standing message holds is refused.

## When access is given or taken away

1. **Who can observe it, and by which routes?** Holders of Review reports, through the cases (`GET /admin/reports`, `GET /admin/reports/{case}`) at signed URLs. Nobody else, by REST, the event stream, search, plugins, or the anonymous read path once moved. Whoever saved a URL before the deletion reads it until the move, about five seconds.
2. **What decides it, and where is that checked?** `attachment.evidence_at`, set by `evidence::keep_deleted` and `keep_removed` under a lock on the attachment rows. So two deletions of messages sharing one cannot both miss that the other left it in none. Every reader of attachments but `evidence::read_for_review` leaves it out, and that one is reached only from the review endpoints, which take Review reports.
3. **When the deciding permission is lost, what happens to what is already open?**
   - A reviewer who loses Review reports reads no more cases. A signed URL already given works out its ten minutes.
   - The uploader loses the attachment with the deletion or removal.
   - A client's cache keeps the record until the message's deletion or update prunes it.
4. **When it is gained, how does a client already open find out without a reload?** A reviewer reads cases afresh. Evidence is never announced.
5. **Does every path that changes it announce it?** Deletion and removal announce the message's deletion or update as they always do. Becoming evidence, the move, and the retention purge are not announced, since only reviewers read evidence and they read it by request. The terminal's purge is written to the moderation log.
6. **Is it published inside the transaction that makes the change?** `evidence_at` is set in the deletion's or removal's own transaction, so a rollback leaves the attachment as it was.
