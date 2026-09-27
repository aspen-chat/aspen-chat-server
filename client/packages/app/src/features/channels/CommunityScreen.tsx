import { Navigate, Outlet, useParams } from "@tanstack/react-router";
import { useState } from "react";
import { useChannels, useCommunity } from "@/api/hooks";
import { ChannelSidebar } from "@/features/channels/ChannelSidebar";
import { MemberList } from "@/features/members/MemberList";
import { MembersPanelContext } from "@/features/members/membersPanel";
import { useMessages } from "@/i18n/context";
import { communityRoute } from "@/router";

/**
 * `/communities/{community}`: the channel sidebar beside the route's content, with the member
 * list on the right. On narrow screens only one of the sidebar and the content is shown: the
 * sidebar at the community index, the content once a channel is chosen. The member list is
 * hidden below the large breakpoint and can be toggled from a channel's header.
 */
export function ChannelSidebarLayout() {
  const m = useMessages();
  const { communityId } = useParams({ from: communityRoute.id });
  const { channelId } = useParams({ strict: false });
  const community = useCommunity(communityId);
  const [membersOpen, setMembersOpen] = useState(true);
  if (community === undefined) {
    return (
      <main className="flex flex-1 items-center justify-center p-6 text-ink-muted">
        {m.communityNotFound}
      </main>
    );
  }
  const showingChannel = channelId !== undefined;
  return (
    <MembersPanelContext.Provider
      value={{
        open: membersOpen,
        toggle: () => {
          setMembersOpen((open) => !open);
        },
      }}
    >
      <div className={`${showingChannel ? "hidden md:flex" : "flex"} w-full flex-col md:w-64`}>
        <ChannelSidebar community={community} />
      </div>
      <div className={`${showingChannel ? "flex" : "hidden md:flex"} min-w-0 flex-1 flex-col`}>
        <Outlet />
      </div>
      {membersOpen && (
        <div className="hidden lg:flex">
          <MemberList communityId={communityId} />
        </div>
      )}
    </MembersPanelContext.Provider>
  );
}

/** The community's index: on wide screens, open its first text channel. */
export function CommunityIndex() {
  const m = useMessages();
  const { communityId } = useParams({ from: communityRoute.id });
  const channels = useChannels(communityId);
  const first = channels.find((c) => c.ty === "text");
  if (first !== undefined) {
    return (
      <Navigate
        to="/communities/$communityId/channels/$channelId"
        params={{ communityId, channelId: first.id }}
        replace
      />
    );
  }
  return (
    <main className="flex flex-1 items-center justify-center p-6 text-ink-muted">
      {m.noChannels}
    </main>
  );
}
