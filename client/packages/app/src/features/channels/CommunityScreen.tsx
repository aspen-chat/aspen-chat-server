import { Navigate, Outlet, useParams } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { useChannels, useCommunity, useDeploymentCan, useSync, useSyncStatus } from "@/api/hooks";
import { ChannelSidebar } from "@/features/channels/ChannelSidebar";
import { MemberList } from "@/features/members/MemberList";
import { MEDIUM_SCREEN, useMediaQuery } from "@/features/layout/useMediaQuery";
import { MembersPanelContext } from "@/features/members/membersPanel";
import { useMessages } from "@/i18n/context";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { useDomain, channelLink } from "@/features/messages/links";

/**
 * `/communities/{community}`: the channel sidebar beside the route's content, with the member
 * list on the right. On narrow screens only one of the sidebar and the content is shown: the
 * sidebar at the community index, the content once a channel is chosen. The member list is
 * hidden below the large breakpoint and can be toggled from a channel's header. A moderator of
 * the server may open a community they are not in, which is read here on arrival.
 */
export function ChannelSidebarLayout() {
  const onePane = useOnePane();
  const m = useMessages();
  // Rendered only under a community route, at home or on another deployment.
  const communityId = useParams({ strict: false }).communityId ?? "";
  const { channelId } = useParams({ strict: false });
  const community = useCommunity(communityId);
  const sync = useSync();
  const live = useSyncStatus() === "live";
  const moderator = useDeploymentCan("moderateCommunities");
  const [membersOpen, setMembersOpen] = useState(true);
  const [missingId, setMissingId] = useState<string | null>(null);
  const held = community !== undefined;
  useEffect(() => {
    if (!live || held || !moderator) {
      return;
    }
    sync.loadCommunity(communityId).catch(() => {
      setMissingId(communityId);
    });
  }, [sync, communityId, live, held, moderator]);
  if (community === undefined) {
    return (
      <main className="flex flex-1 items-center justify-center p-6 text-ink-muted">
        {moderator && missingId !== communityId ? m.loading : m.communityNotFound}
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
      <div
        role={onePane && !showingChannel ? "main" : undefined}
        className={`${showingChannel ? "hidden md:flex" : "flex"} w-full flex-col md:w-64`}
      >
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

/**
 * The community's index: on wide screens, open its first text channel. On narrow ones the
 * index is where the channel list shows, and it is what a channel's back link leads to, so it
 * stays put.
 */
export function CommunityIndex() {
  const m = useMessages();
  // Rendered only under a community route, at home or on another deployment.
  const communityId = useParams({ strict: false }).communityId ?? "";
  const channels = useChannels(communityId);
  const wide = useMediaQuery(MEDIUM_SCREEN);
  const domain = useDomain();
  if (!wide) {
    return null;
  }
  const first = channels.find((c) => c.ty === "text");
  if (first !== undefined) {
    return <Navigate {...channelLink({ domain, community: communityId }, first.id)} replace />;
  }
  return (
    <main className="flex flex-1 items-center justify-center p-6 text-ink-muted">
      {m.noChannels}
    </main>
  );
}
