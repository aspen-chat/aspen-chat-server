import { ApiProblemError } from "@aspen/protocol";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useSync, useVoiceMutes } from "@/api/hooks";
import { ShowMore } from "@/features/layout/ShowMore";
import { alertClass, hintClass } from "@/features/auth/styles";
import { planeClass, planeSurfaceClass, secondaryButtonClass } from "@/features/invites/dialog";
import { RowsSkeleton } from "@/features/layout/ScreenSkeletons";
import { PersonAvatar, PersonName } from "@/features/users/PersonName";
import { useDateFormat } from "@/i18n/format";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const WHEN: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/**
 * The people a moderator muted in the community's calls, newest first, a page at a time, each
 * with a button that lifts the mute, whether or not they are in a call. Shown to holders of
 * Manage calls, whose events keep it current.
 */
export function VoiceMutedList({ communityId }: { communityId: string }) {
  const m = useMessages();
  const sync = useSync();
  const mutes = useVoiceMutes(communityId);
  return (
    <section className={planeClass} aria-labelledby="voice-muted-heading">
      <h3 id="voice-muted-heading" className="font-medium">
        {m.members.voiceMutedHeading}
      </h3>
      {mutes === undefined ? (
        <RowsSkeleton count={2} />
      ) : mutes.length === 0 ? (
        <p className={hintClass}>{m.members.voiceMutedNone}</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {mutes.map((mute) => (
            <VoiceMutedRow
              key={mute.user}
              communityId={communityId}
              userId={mute.user}
              mutedAt={mute.mutedAt}
            />
          ))}
        </ul>
      )}
      <ShowMore
        topic={`voiceMutes:${communityId}`}
        load={() => sync.loadVoiceMutes(communityId, true)}
      />
    </section>
  );
}

function VoiceMutedRow({
  communityId,
  userId,
  mutedAt,
}: {
  communityId: string;
  userId: string;
  mutedAt: string;
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
      await sync.liftVoiceMute(communityId, userId);
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
          {format(m.members.voiceMutedSince, { when: when.format(new Date(mutedAt)) })}
        </span>
      </span>
      <Button
        className={secondaryButtonClass}
        isDisabled={busy}
        onPress={() => {
          void lift();
        }}
      >
        {busy ? m.members.liftingVoiceMute : m.members.liftVoiceMute}
      </Button>
      {error !== null && (
        <p role="alert" className={alertClass + " basis-full"}>
          {error}
        </p>
      )}
    </li>
  );
}
