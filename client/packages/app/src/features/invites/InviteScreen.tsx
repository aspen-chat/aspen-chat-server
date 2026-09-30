import { ApiProblemError, type InviteLookup } from "@aspen/protocol";
import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useSync, useSyncStatus } from "@/api/hooks";
import { useHomeDomainState } from "@/api/identity";
import { primaryButtonClass } from "@/features/auth/styles";
import { Avatar } from "@/features/communities/Avatar";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useDomain, communityLink, inviteLink } from "@/features/messages/links";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";

type Lookup =
  | { state: "loading" }
  | { state: "failed"; message: string; notFound: boolean }
  | ({ state: "found" } & InviteLookup);

/**
 * `/invite/{code}`: what an invite link opens. Shows which community the invite is for and
 * joins it on request; if the user already belongs, it just opens the community. A link whose
 * `?at=` names a deployment other than the user's home goes on to that deployment's invite
 * route, which signs in there first.
 */
export function InviteScreen() {
  const m = useMessages();
  // Rendered only under an invite route, at home or on another deployment.
  const code = useParams({ strict: false }).code ?? "";
  const sync = useSync();
  const status = useSyncStatus();
  const navigate = useNavigate();
  const domain = useDomain();
  const { at }: { at?: unknown } = useSearch({ strict: false });
  const home = useHomeDomainState();
  // Where the invite belongs: `null` for here, the domain to go on to, or `undefined` while the
  // home's own domain is not yet known.
  const named = domain === null && typeof at === "string" ? at.toLowerCase() : null;
  const elsewhere =
    named === null ? null : home === undefined ? undefined : named === home ? null : named;
  const [lookup, setLookup] = useState<Lookup>({ state: "loading" });
  const [joining, setJoining] = useState(false);
  const [joinError, setJoinError] = useState<string | null>(null);

  useEffect(() => {
    if (typeof elsewhere === "string") {
      void navigate({ ...inviteLink(elsewhere, code), replace: true });
    }
  }, [elsewhere, code, navigate]);

  // The membership check needs the bootstrap, so wait until the cache is loaded; an invite of
  // another deployment is looked up there.
  const ready = status !== "bootstrapping" && status !== "stopped" && elsewhere === null;
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
        {lookup.state === "loading" && (
          <div aria-busy="true" className="flex w-full flex-col items-center gap-4">
            <LoadingLabel text={m.lookingUpInvite} />
            <Skeleton className="h-12 w-12 rounded-full" />
            <Skeleton className="h-6 w-48" />
            <Skeleton className="h-4 w-36" />
            <Skeleton className="h-10 w-full" />
          </div>
        )}
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
