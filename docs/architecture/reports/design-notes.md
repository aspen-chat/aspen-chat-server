# Reports: design notes

Why reports work as they do. The how is in the [main pages](index.md).

## Filing

See [Filing reports](filing-reports.md).

- **A profile report keeps the whole profile as it stood (`ProfileSnapshot`).** So a later change hides nothing.
- **Categories are never deleted, only hidden.** So reports keep their category.

## Cases

See [Cases](cases.md).

- **One unresolved case per subject, enforced by partial unique indexes, with `ON CONFLICT DO NOTHING` and a locked re-read.** So concurrent first reports make one case.

## Reviewing

See [Reviewing cases](reviewing-cases.md).

- **A bot ranks as the higher of itself and its owner.** It acts for them.
- **Who may see a case is decided when it is read.** A reviewer who loses or gains rank finds the list changed at the next read.

## Resolving

See [Resolving cases](resolving-cases.md).

- **The case is marked resolved before its actions run, under its row lock.** So two reviewers cannot both act.
- **Review reports alone allows `warn`.** So every reviewer can act on a case.
- **Warnings come from the system account, without the reviewer's name.** They are sent for the deployment's moderators. The system account reaches anyone, and nobody answers or blocks it.
- **A warning keeps a snapshot of what it is about.** So it reads the same after the subject leaves or the thing is deleted.
- **`clearNickname` ignores the community's ranks.** The reviewer's deployment rank has already been checked. It reaches foreign users too, since the nickname is this deployment's.

## Evidence

See [Evidence](evidence.md).

- **Deleting a message hides it rather than erasing it.** So the reviews and warnings that show it still can.
- **`evidence_at` is set under a lock on the attachment rows.** So two deletions of messages sharing one attachment cannot both miss that the other left it in none.
- **Records of moved keys are written only where they are still the old ones.** So servers moving at once do no harm.
- **Becoming evidence and the move are not announced.** Only reviewers read evidence, and they read it by request.
- **Nothing over the API deletes evidence.** It goes when the deployment's retention period passes (`purgeEvidence`), or at once by an operator at the terminal.
- **The retention purge writes nothing to the moderation log.** It carries out the deployment's policy rather than anyone's act. The terminal's purge is someone's act, so it is logged.
