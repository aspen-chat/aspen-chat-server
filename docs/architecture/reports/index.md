# Reports

Anyone may report a message they can read, a person's profile, or a fellow member's nickname to the deployment's moderators. Reports gather in cases, which holders of Review reports resolve with actions such as a warning, a ban, or deleting the message.

## Pages

- [Filing reports](filing-reports.md): the three report endpoints, what each keeps, and the report categories.
- [Cases](cases.md): how reports gather into cases, case statuses, and the `reportsChanged` event.
- [Reviewing cases](reviewing-cases.md): the review endpoints, a message's context, and who may review which case.
- [Resolving cases](resolving-cases.md): the resolution actions (`warn`, `ban`, `deleteMessage`, `clearNickname`, `reset`), dismissal, and restoring.
- [Evidence](evidence.md): what deleting a message keeps, attachments kept as evidence, the mover, purging after the retention period or at once, and the revocation checklist.

Why things are the way they are: [design notes](design-notes.md).

## Key files

| Part | Where |
| --- | --- |
| Reports, cases, review, resolution | `app::report`, `api::report` |
| Warnings | `app::system_account::post` |
| Deleting messages | `app::message::soft_delete` |
| Evidence | `app::attachment::evidence`, `media_store::EVIDENCE_PREFIX` |
| Purging evidence | `purgeEvidence` job (`evidence::purge_step`), `aspen-chat-server attachments purge`, `moderation_log::log_operator_moderation` |
