import { Navigate } from "@tanstack/react-router";
import { Button } from "react-aria-components";
import { useCommunities, useSyncStatus } from "@/api/hooks";
import { primaryButtonClass } from "@/features/auth/styles";
import { AddCommunityDialog } from "@/features/communities/AddCommunityDialog";
import { JoinForm } from "@/features/invites/JoinForm";
import { communityLink, dmsLink, useDomain } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { ChannelSkeleton } from "@/features/layout/ScreenSkeletons";

/** `/`: opens the first community, or explains that there is none to open. */
export function Home() {
  const m = useMessages();
  const communities = useCommunities();
  const status = useSyncStatus();
  const first = communities[0];
  if (first !== undefined) {
    return <Navigate to="/communities/$communityId" params={{ communityId: first.id }} replace />;
  }
  if (status === "bootstrapping") {
    return <ChannelSkeleton />;
  }
  return (
    <main className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
      <h1 className="text-xl font-semibold">{m.noCommunitiesHeading}</h1>
      <p className="max-w-md text-ink-muted">{m.noCommunitiesHint}</p>
      <AddCommunityDialog
        initialStep="create"
        trigger={<Button className={primaryButtonClass + " mt-2"}>{m.createCommunity}</Button>}
      />
      <p className="mt-4 text-sm text-ink-faint">{m.orJoin}</p>
      <div className="w-full max-w-sm text-start">
        <JoinForm />
      </div>
    </main>
  );
}

/**
 * `/at/{domain}`: another deployment's first community, or its DMs when the user belongs to
 * none of its communities yet.
 */
export function ForeignIndex() {
  const domain = useDomain();
  const communities = useCommunities();
  const status = useSyncStatus();
  const first = communities[0];
  if (first !== undefined) {
    return <Navigate {...communityLink(domain, first.id)} replace />;
  }
  if (status === "bootstrapping") {
    return <ChannelSkeleton />;
  }
  return <Navigate {...dmsLink(domain)} replace />;
}
