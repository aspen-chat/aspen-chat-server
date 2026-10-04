import { Navigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { useHomeClient } from "@/api/context";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";

/**
 * A registration link opened while signed in, which is also where creating an account with one
 * lands: a dual invite goes on to its community's invite screen (which opens the community for
 * someone who just joined it, and offers to join to anyone else), and a plain one, or one that no
 * longer works, goes home.
 */
export function RegisterLanding({ invite }: { invite: string | undefined }) {
  const m = useMessages();
  const client = useHomeClient();
  const [communityInvite, setCommunityInvite] = useState<string | null | undefined>(
    invite === undefined ? null : undefined,
  );
  useEffect(() => {
    if (invite === undefined) {
      return;
    }
    let live = true;
    client.registrationInvite(invite).then(
      (read) => {
        if (live) {
          setCommunityInvite(read.data.communityInvite ?? null);
        }
      },
      () => {
        if (live) {
          setCommunityInvite(null);
        }
      },
    );
    return () => {
      live = false;
    };
  }, [client, invite]);
  if (communityInvite === undefined) {
    return (
      <div aria-busy="true" className="flex flex-1 flex-col gap-3 p-6">
        <LoadingLabel text={m.qr.openingInvite} />
        <Skeleton className="h-7 w-48" />
        <Skeleton className="h-24 w-full max-w-sm rounded-lg" />
      </div>
    );
  }
  return communityInvite === null ? (
    <Navigate to="/" replace />
  ) : (
    <Navigate to="/invite/$code" params={{ code: communityInvite }} replace />
  );
}
