import { Link } from "@tanstack/react-router";
import { ArrowLeftIcon, ArrowSquareLeftIcon, ArrowSquareRightIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { Button } from "react-aria-components";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMembersPanel } from "@/features/members/membersPanel";
import { useChannelOnline } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { useNumberFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { useDomain, communityLink } from "@/features/messages/links";

/**
 * The bar above a channel: a way back to the channel list on small screens, the channel's
 * name behind a glyph for its kind and how many who may view it are online, anything the
 * screen adds, and the members toggle.
 */
export function ChannelHeader({
  communityId,
  channelId,
  glyph,
  name,
  children,
}: {
  communityId: string;
  channelId: string;
  glyph: ReactNode;
  name: string;
  children?: ReactNode;
}) {
  const onePane = useOnePane();
  const Heading = onePane ? "h1" : "h2";
  const m = useMessages();
  const domain = useDomain();
  const membersPanel = useMembersPanel();
  return (
    <header className="flex items-center gap-2 border-b border-line px-4 py-3">
      <Link
        {...communityLink(domain, communityId)}
        aria-label={m.backToChannels}
        className="tap-target rounded-md p-1 text-ink-muted outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50 md:hidden"
      >
        <ArrowLeftIcon size={18} aria-hidden="true" className="rtl:-scale-x-100" />
      </Link>
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <Heading className="flex min-w-0 items-center gap-1.5 truncate font-semibold">
          <span aria-hidden="true" className="text-ink-faint">
            {glyph}
          </span>
          {name}
        </Heading>
        <OnlineCount channelId={channelId} />
      </div>
      {children}
      <Tooltip text={membersPanel.open ? m.hideMembers : m.showMembers}>
        <Button
          onPress={membersPanel.toggle}
          aria-label={membersPanel.open ? m.hideMembers : m.showMembers}
          className="rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {membersPanel.open ? (
            <ArrowSquareRightIcon size={20} aria-hidden="true" className="rtl:-scale-x-100" />
          ) : (
            <ArrowSquareLeftIcon size={20} aria-hidden="true" className="rtl:-scale-x-100" />
          )}
        </Button>
      </Tooltip>
    </header>
  );
}

/** How many who may view the channel are online, behind the online status's dot. */
function OnlineCount({ channelId }: { channelId: string }) {
  const m = useMessages();
  const numbers = useNumberFormat();
  const online = useChannelOnline(channelId);
  if (online === undefined) {
    return null;
  }
  return (
    <span className="flex shrink-0 items-center gap-1.5 text-sm text-ink-muted">
      <span aria-hidden="true" className="h-2.5 w-2.5 rounded-full bg-online" />
      {format(m.channelOnline, { count: numbers.format(online) })}
    </span>
  );
}
