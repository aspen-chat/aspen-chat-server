import type { CommunityGrowth } from "@aspen/protocol";
import { UsersThreeIcon } from "@phosphor-icons/react";
import { useCommunity } from "@/api/hooks";
import { growthText } from "@/features/messages/growth";
import { useMessages } from "@/i18n/context";

/**
 * The system message a community's default channel shows when the community becomes large. It
 * tags everyone, so it notifies everyone who can see the channel.
 */
export function CommunityGrewNotice({
  communityId,
  growth,
}: {
  communityId: string | null;
  growth: CommunityGrowth | null | undefined;
}) {
  const m = useMessages();
  const community = useCommunity(communityId ?? "");
  return (
    <div className="flex items-start gap-2 text-sm text-ink-muted">
      <UsersThreeIcon size={18} aria-hidden="true" className="mt-0.5 shrink-0" />
      <span>{growthText(m, growth, community?.name ?? m.communityGrew.thisCommunity)}</span>
    </div>
  );
}
