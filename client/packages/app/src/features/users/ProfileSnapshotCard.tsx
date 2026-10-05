import type { ProfileAspect, ProfileSnapshot } from "@aspen/protocol";
import { useAspectList } from "@/features/users/profileAspects";
import { Avatar } from "@/features/communities/Avatar";
import { displayNameOf, statusLine } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A profile as a report found it, for reference: its picture, names, pronouns, status, and bio,
 * with the aspects the reports named marked and listed.
 */
export function ProfileSnapshotCard({
  snapshot,
  aspects,
}: {
  snapshot: ProfileSnapshot;
  aspects: readonly ProfileAspect[];
}) {
  const m = useMessages();
  const list = useAspectList();
  const name = displayNameOf(snapshot);
  const marked = (aspect: ProfileAspect) =>
    aspects.includes(aspect) ? " rounded bg-danger-soft px-0.5" : "";
  return (
    <div className="flex max-w-lg flex-col gap-1 rounded-md border border-line bg-surface-raised p-2 text-sm">
      <div className="flex items-center gap-2">
        <span
          className={
            aspects.includes("picture")
              ? "forced-outline rounded-full ring-2 ring-danger ring-offset-1"
              : ""
          }
        >
          <Avatar name={name} iconId={snapshot.icon ?? null} />
        </span>
        <div className="min-w-0">
          <div className={"truncate font-semibold" + marked("displayName")}>
            {snapshot.displayName ?? snapshot.name}
          </div>
          <div className="truncate text-ink-muted">
            <span className={marked("username")}>@{snapshot.name}</span>
            {snapshot.pronouns != null && (
              <span>
                {" · "}
                <span className={marked("pronouns")}>{snapshot.pronouns}</span>
              </span>
            )}
          </div>
        </div>
      </div>
      {snapshot.status != null && (
        <p className={"break-words" + marked("status")}>{statusLine(snapshot.status)}</p>
      )}
      {snapshot.bio != null && (
        <p className={"break-words whitespace-pre-wrap" + marked("bio")}>{snapshot.bio}</p>
      )}
      {aspects.length > 0 && (
        <p className="text-xs font-medium text-danger">
          {format(m.reports.reportedAspects, { aspects: list(aspects) })}
        </p>
      )}
    </div>
  );
}
