import { ApiProblemError, type InviteLookup } from "@aspen/protocol";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useSync, useSyncStatus } from "@/api/hooks";
import { primaryButtonClass } from "@/features/auth/styles";
import { Avatar } from "@/features/communities/Avatar";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useDomain, communityLink } from "@/features/messages/links";

type Lookup =
  | { state: "loading" }
  | { state: "failed"; message: string; notFound: boolean }
  | ({ state: "found" } & InviteLookup);

/**
 * `/invite/{code}`: what an invite link opens. Shows which community the invite is for and
 * joins it on request; if the user already belongs, it just opens the community.
 */
export function InviteScreen() {
  const m = useMessages();
  // Rendered only under an invite route, at home or on another deployment.
  const code = useParams({ strict: false }).code ?? "";
  const sync = useSync();
  const status = useSyncStatus();
  const navigate = useNavigate();
  const domain = useDomain();
  const [lookup, setLookup] = useState<Lookup>({ state: "loading" });
  const [joining, setJoining] = useState(false);
  const [joinError, setJoinError] = useState<string | null>(null);

  // The membership check needs the bootstrap, so wait until the cache is loaded.
  const ready = status !== "bootstrapping" && status !== "stopped";
  useEffect(() => {
    if (!ready) {
      return;
    }
    let cancelled = false;
    sync.lookupInvite(code).then(
      (found) => {
        if (!cancelled) {
          setLookup({ state: "found", ...found });
        }
      },
      (error: unknown) => {
        if (!cancelled) {
          const problem = error instanceof ApiProblemError ? error : null;
          setLookup({
            state: "failed",
            notFound: problem?.status === 404,
            message: problem?.message ?? String(error),
          });
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [sync, code, ready]);

  async function join(communityId: string) {
    setJoining(true);
    setJoinError(null);
    try {
      await sync.joinCommunity(communityId, code);
      await navigate({ ...communityLink(domain, communityId), replace: true });
    } catch (e) {
      setJoinError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setJoining(false);
    }
  }

  return (
    <main className="flex flex-1 items-center justify-center p-6">
      <div className="flex w-full max-w-sm flex-col items-center gap-4 rounded-lg border border-line bg-surface-raised p-6 text-center">
        {lookup.state === "loading" && <p className="text-ink-muted">{m.lookingUpInvite}</p>}
        {lookup.state === "failed" && (
          <>
            <h1 className="text-xl font-semibold">{m.inviteUnusableHeading}</h1>
            <p className="text-ink-muted">{lookup.notFound ? m.inviteNotFound : lookup.message}</p>
          </>
        )}
        {lookup.state === "found" && (
          <>
            <Avatar name={lookup.community.name} iconId={lookup.community.icon} size="lg" />
            <h1 className="text-xl font-semibold">
              {lookup.member
                ? format(m.alreadyMember, { community: lookup.community.name })
                : lookup.expired
                  ? m.inviteExpiredHeading
                  : format(m.invitedTo, { community: lookup.community.name })}
            </h1>
            {lookup.expired && !lookup.member && (
              <p className="text-ink-muted">{m.inviteExpiredHint}</p>
            )}
            {joinError !== null && (
              <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
                {joinError}
              </p>
            )}
            {lookup.member ? (
              <Button
                onPress={() => {
                  void navigate(communityLink(domain, lookup.community.id));
                }}
                className={primaryButtonClass}
              >
                {m.openCommunity}
              </Button>
            ) : (
              !lookup.expired && (
                <Button
                  isDisabled={joining}
                  onPress={() => {
                    void join(lookup.community.id);
                  }}
                  className={primaryButtonClass}
                >
                  {joining ? m.joining : m.join}
                </Button>
              )
            )}
          </>
        )}
      </div>
    </main>
  );
}
