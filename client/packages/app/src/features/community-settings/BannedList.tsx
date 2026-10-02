import { ApiProblemError } from "@aspen/protocol";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useBans, useSync } from "@/api/hooks";
import { alertClass, hintClass } from "@/features/auth/styles";
import { planeClass, planeSurfaceClass, secondaryButtonClass } from "@/features/invites/dialog";
import { RowsSkeleton } from "@/features/layout/ScreenSkeletons";
import { PersonAvatar, PersonName } from "@/features/users/PersonName";
import { useDateFormat } from "@/i18n/format";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const WHEN: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/**
 * The community's standing bans, newest first, each with its reason and end, and a button
 * that lifts it. Shown to holders of Ban members, whose events keep it current.
 */
export function BannedList({ communityId }: { communityId: string }) {
  const m = useMessages();
  const bans = useBans(communityId);
  return (
    <section className={planeClass} aria-labelledby="banned-heading">
      <h3 id="banned-heading" className="font-medium">
        {m.members.bannedHeading}
      </h3>
      {bans === undefined ? (
        <RowsSkeleton count={2} />
      ) : bans.length === 0 ? (
        <p className={hintClass}>{m.members.bannedNone}</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {bans.map((ban) => (
            <BannedRow
              key={ban.user}
              communityId={communityId}
              userId={ban.user}
              reason={ban.reason ?? null}
              until={ban.until ?? null}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function BannedRow({
  communityId,
  userId,
  reason,
  until,
}: {
  communityId: string;
  userId: string;
  reason: string | null;
  until: string | null;
}) {
  const m = useMessages();
  const sync = useSync();
  const when = useDateFormat(WHEN);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function lift() {
    setBusy(true);
    setError(null);
    try {
      await sync.liftBan(communityId, userId);
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
      setBusy(false);
    }
  }

  return (
    <li className={planeSurfaceClass + " flex flex-wrap items-center gap-3 px-3 py-2"}>
      <PersonAvatar id={userId} size="sm" />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-medium">
          <PersonName id={userId} />
        </span>
        <span className="text-xs text-ink-muted">
          {until === null
            ? m.members.bannedForever
            : format(m.members.bannedUntil, { when: when.format(new Date(until)) })}
          {" · "}
          {reason === null ? m.members.bannedNoReason : format(m.members.bannedReason, { reason })}
        </span>
      </span>
      <Button
        className={secondaryButtonClass}
        isDisabled={busy}
        onPress={() => {
          void lift();
        }}
      >
        {busy ? m.members.lifting : m.members.liftBan}
      </Button>
      {error !== null && (
        <p role="alert" className={alertClass + " basis-full"}>
          {error}
        </p>
      )}
    </li>
  );
}
