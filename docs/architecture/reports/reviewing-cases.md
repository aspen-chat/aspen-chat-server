# Reviewing cases

Review takes the deployment permission Review reports.

## Endpoints

| Endpoint | Does |
| --- | --- |
| `GET /admin/reports?filter[status]=open\|resolved\|dismissed` | Pages the cases with `before` (the last case of the previous page) and `limit` (15 by default). Open and dismissed cases are ordered by their latest report, through `report_case_by_status`; resolved ones by when they were resolved, through `report_case_resolved` |
| `GET /admin/reports/{case}` | Reads one case |
| `GET /admin/report-counts` | Counts the open and dismissed cases, each up to `MAX_COUNTED_CASES` (1000, shown as "999+") |
| `GET /admin/reports/{case}/context` | Reads the reported message's surroundings (below) |

The list and the single read give each case with:

- Its latest `SHOWN_REPORTS` (50) reports, and `reportCount`, how many it has in all.
- The reported messages, whether deleted or not.
- The categories used.
- The people, channels, communities (a nickname case's among them), and attachments they name.

## A message's context

`GET /admin/reports/{case}/context` reads the reported message's channel, thread, or DM around it:

- `CONTEXT_MESSAGES` (25) either side.
- Deleted messages included, and marked.
- Paged with `before` and `after`, no further than `CONTEXT_REACH` (100) messages from the reported one either way.

Each page read of a DM the reviewer is not in is written to the moderation log as `readReportContext`, naming the reported message.

## Who may review a case

Nobody reviews a case about:

- Themselves.
- Their own bot.
- Someone whose highest deployment role is not below theirs. A bot ranks as the higher of itself and its owner, since it acts for them.

This is `may_act` and `Standing`. Such cases are:

- Left out of the list, the counts, and `reportsChanged`.
- Not found by `GET /admin/reports/{case}` and its context.
- Refused by the actions with `403` `reportConflictOfInterest`.

So a case's `mayAct` is true wherever it is read.

Who may see a case is decided when it is read. A reviewer who loses rank, or gains it, finds the list changed at the next read. The dashboard reads it again on each `reportsChanged`.
