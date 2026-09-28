import { Link } from "@tanstack/react-router";
import { ArrowLeftIcon, ArrowSquareLeftIcon, ArrowSquareRightIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { Button } from "react-aria-components";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMembersPanel } from "@/features/members/membersPanel";
import { useMessages } from "@/i18n/context";
import { useOnePane } from "@/features/layout/useMediaQuery";

/**
 * The bar above a channel: a way back to the channel list on small screens, the channel's
 * name behind a glyph for its kind, anything the screen adds, and the members toggle.
 */
export function ChannelHeader({
  communityId,
  glyph,
  name,
  children,
}: {
  communityId: string;
  glyph: ReactNode;
  name: string;
  children?: ReactNode;
}) {
  const onePane = useOnePane();
  const Heading = onePane ? "h1" : "h2";
  const m = useMessages();
  const membersPanel = useMembersPanel();
  return (
    <header className="flex items-center gap-2 border-b border-line px-4 py-3">
      <Link
        to="/communities/$communityId"
        params={{ communityId }}
        aria-label={m.backToChannels}
        className="tap-target rounded-md p-1 text-ink-muted outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50 md:hidden"
      >
        <ArrowLeftIcon size={18} aria-hidden="true" />
      </Link>
      <Heading className="flex min-w-0 flex-1 items-center gap-1.5 truncate font-semibold">
        <span aria-hidden="true" className="text-ink-faint">
          {glyph}
        </span>
        {name}
      </Heading>
      {children}
      <Tooltip text={membersPanel.open ? m.hideMembers : m.showMembers}>
        <Button
          onPress={membersPanel.toggle}
          aria-label={membersPanel.open ? m.hideMembers : m.showMembers}
          className="hidden rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 lg:block"
        >
          {membersPanel.open ? (
            <ArrowSquareRightIcon size={20} aria-hidden="true" />
          ) : (
            <ArrowSquareLeftIcon size={20} aria-hidden="true" />
          )}
        </Button>
      </Tooltip>
    </header>
  );
}
