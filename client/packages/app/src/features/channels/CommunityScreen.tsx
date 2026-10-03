import { Navigate, Outlet, useParams } from "@tanstack/react-router";
import { ResizablePane } from "@/features/layout/ResizablePane";
import { CHANNEL_LIST, MEMBER_LIST } from "@/features/layout/paneSizes";
import { useEffect, useRef, useState } from "react";
import { useChannels, useCommunity, useDeploymentCan, useSync, useSyncStatus } from "@/api/hooks";
import { ChannelSidebar } from "@/features/channels/ChannelSidebar";
import { MemberGroups, MemberList } from "@/features/members/MemberList";
import { Drawer } from "@/features/layout/Drawer";
import { LARGE_SCREEN, MEDIUM_SCREEN, useMediaQuery } from "@/features/layout/useMediaQuery";
import { MembersPanelContext } from "@/features/members/membersPanel";
import { useMessages } from "@/i18n/context";
import { useOnePane } from "@/features/layout/useMediaQuery";
import { useDomain, channelLink } from "@/features/messages/links";
import { ChannelListSkeleton, ChannelSkeleton } from "@/features/layout/ScreenSkeletons";

/**
 * `/communities/{community}`: the channel sidebar beside the route's content, with the member
 * list on the right. On narrow screens only one of the sidebar and the content is shown: the
 * sidebar at the community index, the content once a channel is chosen. From the large
 * breakpoint the member list is a pane, which a channel's header shows and hides; below it the
 * list is a drawer over the channel, which the header's button opens, or a swipe across the
 * channel toward the inline start draws out. A moderator of the server may open a community
 * they are not in, which is read here on arrival.
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
  const large = useMediaQuery(LARGE_SCREEN);
  const [membersOpen, setMembersOpen] = useState(true);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const channelArea = useRef<HTMLDivElement>(null);
  const showingChannel = channelId !== undefined;
  const drawerEnabled = !large && showingChannel && community !== undefined;
  if (!drawerEnabled && drawerOpen) {
    setDrawerOpen(false);
  }
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
  // Until the sync is live, or while a moderator's read of it is on its way, the community is
  // not missing but coming: its screen stands in skeleton.
  if (community === undefined && (!live || (moderator && missingId !== communityId))) {
    return (
      <>
        <ResizablePane
          sizing={CHANNEL_LIST}
          edge="end"
          label={m.layout.channelList}
          className={`${channelId !== undefined ? "hidden md:flex" : "flex"} w-full flex-col`}
        >
          <ChannelListSkeleton />
        </ResizablePane>
        {channelId !== undefined || !onePane ? <ChannelSkeleton /> : null}
      </>
    );
  }
  if (community === undefined) {
    return (
      <main className="flex flex-1 items-center justify-center p-6 text-ink-muted">
        {m.communityNotFound}
      </main>
    );
  }
  return (
    <MembersPanelContext.Provider
      value={
        large
          ? {
              open: membersOpen,
              toggle: () => {
                setMembersOpen((open) => !open);
              },
            }
          : {
              open: drawerOpen,
              toggle: () => {
                setDrawerOpen((open) => !open);
              },
            }
      }
    >
      <ResizablePane
        sizing={CHANNEL_LIST}
        edge="end"
        label={m.layout.channelList}
        role={onePane && !showingChannel ? "main" : undefined}
        className={`${showingChannel ? "hidden md:flex" : "flex"} w-full flex-col`}
      >
        <ChannelSidebar community={community} />
      </ResizablePane>
      <div
        ref={channelArea}
        className={`${showingChannel ? "flex" : "hidden md:flex"} min-w-0 flex-1 flex-col`}
      >
        <Outlet />
      </div>
      {large ? (
        membersOpen && (
          <ResizablePane
            sizing={MEMBER_LIST}
            edge="start"
            label={m.layout.memberList}
            className="flex"
          >
            <MemberList communityId={communityId} />
          </ResizablePane>
        )
      ) : (
        <Drawer
          swipeFrom={channelArea}
          enabled={drawerEnabled}
          isOpen={drawerOpen}
          onOpenChange={setDrawerOpen}
          title={m.membersLabel}
        >
          <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-3">
            <MemberGroups communityId={communityId} headingLevel={3} />
          </div>
        </Drawer>
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
