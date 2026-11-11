import type { ReportCaseList } from "@aspen/protocol";

/** Two pages of cases as one, the later's after the earlier's, continuing where it does. */
function joined(a: ReportCaseList, b: ReportCaseList): ReportCaseList {
  return {
    nextCursor: b.nextCursor,
    cases: [...a.cases, ...b.cases],
    users: [...a.users, ...b.users],
    messages: [...a.messages, ...b.messages],
    channels: [...a.channels, ...b.channels],
    communities: [...a.communities, ...b.communities],
    categories: [...a.categories, ...b.categories],
    attachments: [...a.attachments, ...b.attachments],
  };
}

/**
 * The first `pages` pages of a list of cases as one, or as many as it has: each after the
 * first is asked of `read` by the cursor the one before gave, while it gave one. The result's
 * `nextCursor` says whether a page follows them, which how many cases a page held never does.
 */
export async function readCasePages(
  read: (cursor?: string) => Promise<ReportCaseList>,
  pages: number,
): Promise<ReportCaseList> {
  let list = await read();
  for (let page = 1; page < pages && list.nextCursor !== null; page++) {
    list = joined(list, await read(list.nextCursor));
  }
  return list;
}
