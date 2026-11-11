import type { Channel } from "@aspen/protocol";
import { Link } from "@tanstack/react-router";
import { Button } from "react-aria-components";
import { type LogPage, useCursorLog } from "@/features/admin/useCursorLog";
import { useDmTitle } from "@/features/dms/useDmTitle";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";

/** The widths of the names standing in for DMs on their way, one after another. */
const TITLE_WIDTHS = ["w-40", "w-28", "w-48", "w-32"];

/**
 * For a moderator: someone's DMs, most recently active first, each a link to read it, a page
 * at a time. `load` reads the page a cursor asks for, or the first, and must keep its identity
 * between renders for as long as it reads the same person's DMs.
 */
export function UserDmList({ load }: { load: (cursor?: string) => Promise<LogPage<Channel>> }) {
  const m = useMessages();
  const { entries, loading, error, more, showMore } = useCursorLog(load);
  return (
    <>
      {entries.length > 0 && (
        <ul className="flex flex-col gap-1">
          {entries.map((dm) => (
            <li key={dm.id} className="flex flex-col">
              <DmLink dm={dm} />
            </li>
          ))}
        </ul>
      )}
      {loading && (
        <div aria-busy="true" className="flex flex-col gap-1">
          <LoadingLabel />
          {TITLE_WIDTHS.slice(0, entries.length === 0 ? 4 : 2).map((width) => (
            <div key={width} className="px-2 py-1">
              <Skeleton className={"h-5 " + width} />
            </div>
          ))}
        </div>
      )}
      {error !== null && (
        <p role="alert" className="px-1 text-xs text-danger">
          {error}
        </p>
      )}
      {!loading && error === null && entries.length === 0 && (
        <p className="px-1 text-sm text-ink-muted">{m.admin.noDms}</p>
      )}
      {!loading && more && (
        <Button onPress={showMore} className={secondaryButtonClass + " self-start"}>
          {m.admin.showMore}
        </Button>
      )}
    </>
  );
}

function DmLink({ dm }: { dm: Channel }) {
  const title = useDmTitle(dm);
  return (
    <Link
      to="/dms/$channelId"
      params={{ channelId: dm.id }}
      className="truncate rounded px-2 py-1 text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      {title}
    </Link>
  );
}
