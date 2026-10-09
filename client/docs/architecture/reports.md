# Reports, warnings, and message links

## Where it lives

| Part | Where |
| --- | --- |
| Report buttons | `ReportMessageButton`, `ReportProfileButton`, `ReportNicknameButton` (`src/features/users/Nickname.tsx`) |
| The report dialog | `ReportModal` (`src/features/reports/ReportDialog.tsx`) |
| Profile aspects | `PROFILE_ASPECTS` (`src/features/users/profileAspects.ts`) |
| Message links | `messageUrl`, `parseSelfLink` (`links.test.ts`), `SelfLink` |
| Embedded messages | `LinkedMessages`, `EmbeddedMessage` (`src/features/messages/EmbeddedMessage.tsx`) |
| Warnings | `WarningBody` (`src/features/messages/WarningBody.tsx`), `ProfileSnapshotCard` |
| Review | `src/features/admin/Reports.tsx`, `ResolveDialog`, `NicknameCase` |
| Categories | `src/features/admin/ReportCategories.tsx` |
| Bans from the deployment | `UserBanDialog`, `BanFields` |

## Reporting

### Who is offered Report

| What | Button | Offered to | Not offered for |
| --- | --- | --- | --- |
| A message | Report (`ReportMessageButton`, or the `report` sheet on a touch screen), in the message's actions | Anyone but its author | The system account's notices and a call's record |
| A profile | Report profile (`ReportProfileButton`), on the profile card | Anyone but the person themselves | The system account |
| A nickname | Report nickname (`ReportNicknameButton`), on the profile card opened in a community where they have a nickname | | |

### The dialog

Each opens `ReportModal`:

1. A profile report first asks which aspects are wrong (`PROFILE_ASPECTS`). A nickname is
   reported whole, as the server keeps it with its community.
2. The category, from `GET /report-categories` as the server offers them in the reader's
   language, each with what it covers.
3. Anything to add. The Other category needs it.

- Sent, a toast says so.
- A second report of the same thing is the server's `alreadyReported`, shown as its detail.
- A report goes to the deployment whose message, profile, or community it is, through that
  scope's `AspenSync`.

## Message links

### Copying

Copy link (`MessageActions`) copies `messageUrl`: the message's route on its deployment's own
address (`AspenClient.baseUrl`). The route is one of:

- `/communities/{c}/channels/{ch}/messages/{m}`
- `/dms/{ch}/messages/{m}`

The server reads either as a link to the message in any message. `parseSelfLink` reads the same
two routes back (`links.test.ts`).

### In a message's text

| Link to | Drawn as |
| --- | --- |
| The home deployment, or another the user uses | A chip naming the message's place and author that opens it here (`SelfLink`, see [message-rendering](message-rendering/self-links.md)), with the message itself beneath |
| Any other host | An ordinary link |

### Embedded messages

A message's `linkedMessages` show beneath it as `LinkedMessages`. Each is an `EmbeddedMessage`:

- the author;
- the time, as the way to jump to it;
- its body drawn `still`: no poll to vote in, no actions, and no links of its own followed;
- or a line saying it was deleted or cannot be viewed.

What the reader finds at each comes from the store's `linkedMessage` (topic `link:<id>`):

- History reads fill it with `include=linked`.
- For a message that arrived or was edited since, `AspenSync.loadLinks` fills it: one read per
  linking message, however many embeds ask.
- A delete event for a linked message marks its link deleted.

## Warnings

A message of kind `warning` is drawn by `WarningBody` in place of its body:

1. labelled a moderator's warning;
2. what it is about, one of:
   - the reported message, from the store's `warnedMessage` (topic `warned:<id>`). History reads
     fill it with `include=warnings`, deleted or not. It is marked deleted when it is;
   - the profile as the reports found it (`ProfileSnapshotCard`), the reported aspects marked;
   - the nickname, with the community it was chosen in, as the warning names it, so it reads
     the same after the person leaves;
3. the moderator's words.

## Review

The dashboard's Reports tab (`src/features/admin/Reports.tsx`, permission Review reports).

### The lists

- It lists the open, resolved, and dismissed cases.
- The open ones are counted on the tab and on the rail's dashboard button (`useOpenReports`,
  `RecordStore.openReports`). The count is read from `GET /admin/report-counts` when first asked
  and kept by `reportsChanged` events.
- It reads the page again on each such event (`useReportsChanges`).

### A case

A case shows every report of it, and what was reported:

| Reported | Shown |
| --- | --- |
| A message | The message, deleted or not, where it was posted, and the conversation around it on request. A page at a time either way, the reported message outlined |
| A profile | The profile as reported beside how it is now |
| A nickname | The nickname as reported, in which community, and what it is now when the reviewer may read that community's members (`NicknameCase`) |

The people and files the dashboard's reads name are put in the cache, so messages there draw as
everywhere else. Channels and communities are not, since they would join the reviewer's own
lists.

### Acting on a case

Take action opens `ResolveDialog`, offering only what the reviewer may do:

| Action | Permission |
| --- | --- |
| Warn, with their words. The system account sends it without their name | Every reviewer |
| Delete the message | Remove content |
| Clear the nickname | Remove content |
| Reset chosen aspects of a profile of this deployment | Remove content |
| Ban from the server, through the same `BanFields` as a community's ban | Ban users |
| Delete their recent messages, with the ban | Remove content |

- Dismiss sets a case aside. The Dismissed list restores it.
- A case the reviewer may not act on says another moderator must. That is a case about them, or
  about someone not below their rank.

## Categories

The Report categories tab (`src/features/admin/ReportCategories.tsx`, permission Manage report
categories):

- hides and shows categories, Other excepted;
- adds the deployment's own;
- renames and describes them;
- moves them up and down among themselves.

## Bans from the deployment

The user directory:

- filters to banned users (Banned only);
- shows each standing ban and its end;
- for holders of Ban users, offers Ban or Lift ban, for anyone but the system account and the
  caller.

Ban opens `UserBanDialog`, with the shared `BanFields`:

- with the deletion window, for holders of Moderate any community;
- for a bot, its owner too.

When a user is banned:

1. Their event stream closes with `4410` after an `accountBanned` event.
2. `EventStream` treats it as a rejected token, so the refresh that follows fails and the app
   signs out.
3. Signing in shows the server's `deploymentBanned` detail with the reason and the end.
