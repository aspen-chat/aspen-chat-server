import type { Message } from "@aspen/protocol";
import { ShieldWarningIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { useChannel, useMe, useSync, useUser, useWarnedMessage } from "@/api/hooks";
import { EmbedNotice, EmbedSkeleton, EmbeddedMessage } from "@/features/messages/EmbeddedMessage";
import type { ChannelHome } from "@/features/messages/links";
import { MessageBody } from "@/features/messages/MessageBody";
import { ProfileSnapshotCard } from "@/features/users/ProfileSnapshotCard";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A moderator's warning, as its DM shows it: labelled as one, then what it is about (the
 * message reported, shown even once deleted, or the profile as the reports found it), then the
 * moderator's own words.
 */
export function WarningBody({ message, home }: { message: Message; home: ChannelHome }) {
  const m = useMessages();
  const me = useMe();
  const warning = message.warning;
  const subject = useUser(warning?.subject);
  const mine = me !== null && warning?.subject === me.id;
  const name = subject === undefined ? m.unknownUser : displayNameOf(subject);
  const about =
    warning?.message != null
      ? mine
        ? m.reports.warningAbout
        : format(m.reports.warningAboutTheirs, { name })
      : mine
        ? m.reports.warningAboutProfile
        : format(m.reports.warningAboutTheirProfile, { name });
  return (
    <div className="mt-1 flex max-w-xl flex-col gap-2 rounded-md border border-danger/40 bg-danger-soft/40 p-2">
      <p className="flex items-center gap-1.5 text-sm font-semibold text-danger">
        <ShieldWarningIcon size={16} weight="fill" aria-hidden="true" />
        {m.reports.warningLabel}
      </p>
      {warning != null && (
        <div className="flex flex-col gap-1">
          <p className="text-xs text-ink-muted">{about}</p>
          {warning.message != null ? (
            <WarnedMessage id={warning.message} from={message.id} />
          ) : warning.profile != null ? (
            <ProfileSnapshotCard snapshot={warning.profile} aspects={warning.aspects ?? []} />
          ) : null}
        </div>
      )}
      <MessageBody message={message} home={home} />
    </div>
  );
}

/** The message a warning is about, read with the warning, deleted or not. */
function WarnedMessage({ id, from }: { id: string; from: string }) {
  const m = useMessages();
  const sync = useSync();
  const kept = useWarnedMessage(id);
  const channel = useChannel(kept?.message.channelId ?? "");
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (kept === undefined && !failed) {
      sync.loadLinks(from).catch(() => {
        setFailed(true);
      });
    }
  }, [sync, kept, from, failed]);
  if (kept === undefined) {
    return failed ? <EmbedNotice text={m.reports.embedUnavailable} /> : <EmbedSkeleton />;
  }
  return (
    <EmbeddedMessage
      message={kept.message}
      community={channel === undefined ? undefined : (channel.community ?? null)}
      deletedAt={kept.deletedAt ?? null}
    />
  );
}
