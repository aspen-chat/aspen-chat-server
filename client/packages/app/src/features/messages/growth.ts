import type { CommunityGrowth } from "@aspen/protocol";
import { format, type Messages } from "@/i18n/messages";

/**
 * What changed when a community became large, in the reader's language: how many members it
 * has, that channels without a setting of their own now notify of tags alone, and, when it
 * happened, that its everyone role lost Mention everyone.
 */
export function growthText(
  m: Messages,
  growth: CommunityGrowth | null | undefined,
  community: string,
) {
  const parts = [
    format(m.communityGrew.members, { community, members: growth?.members ?? 0 }),
    m.communityGrew.notifications,
  ];
  if (growth?.mentionEveryoneTaken === true) {
    parts.push(m.communityGrew.mentionEveryone);
  }
  return parts.join(" ");
}
