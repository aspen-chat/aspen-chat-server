import { ApiProblemError, SAVED_PAGE, type SavedMessage } from "@aspen/protocol";
import { XIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import { Button } from "react-aria-components";
import { SourceScope } from "@/api/deployments";
import { useEverywhere, type Source } from "@/api/everywhere";
import { useMessageOnDemand, useSync } from "@/api/hooks";
import { PersonalPage } from "@/features/activity/PersonalPage";
import { Tooltip } from "@/features/layout/Tooltip";
import { ListedMessage, listedActionClass } from "@/features/messages/ListedMessage";
import { useMessagePlace } from "@/features/messages/place";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** How far one deployment's saved messages have been read. */
interface Read {
  /** How many of its saves, newest first, have had their messages read. */
  readonly count: number;
  /** The message of the last save read, which the next page starts after. */
  readonly last: string | undefined;
  readonly exhausted: boolean;
  readonly problem: string | null;
}

const UNREAD: Read = { count: 0, last: undefined, exhausted: false, problem: null };

/**
 * Saved messages, `/saved`: every message the reader saved on every deployment they use,
 * newest save first, each with a way to go to it and to remove it. The saves themselves are
 * known from the start (`RecordStore.saves`); their messages are read a page at a time per
 * deployment as the list shows more of them.
 */
export function SavedScreen() {
  const m = useMessages();
  const saves = useEverywhere(["saved"], (sources) =>
    sources
      .flatMap((source) => source.sync.store.saves().map((save) => ({ source, save })))
      .sort((a, b) => b.save.id.localeCompare(a.save.id)),
  );
  const [shown, setShown] = useState(SAVED_PAGE);
  const [reads, setReads] = useState<ReadonlyMap<Source, Read>>(new Map());
  // The deployments a page is on its way from.
  const reading = useRef(new Set<Source>());
  const listed = saves.slice(0, shown);

  // Each deployment's messages are read as far as its saves among those shown.
  useEffect(() => {
    const needed = new Map<Source, number>();
    for (const { source } of listed) {
      needed.set(source, (needed.get(source) ?? 0) + 1);
    }
    for (const [source, count] of needed) {
      const read = reads.get(source) ?? UNREAD;
      if (count <= read.count || read.exhausted || reading.current.has(source)) {
        continue;
      }
      reading.current.add(source);
      void source.sync
        .loadSavedMessages(read.last)
        .then(
          (found): Read => ({
            count: read.count + found.length,
            last: found.at(-1)?.id ?? read.last,
            exhausted: found.length < SAVED_PAGE,
            problem: null,
          }),
          (error: unknown): Read => ({
            ...read,
            exhausted: true,
            problem: error instanceof ApiProblemError ? error.message : String(error),
          }),
        )
        .then((next) => {
          reading.current.delete(source);
          setReads((current) => new Map(current).set(source, next));
        });
    }
  }, [listed, reads]);

  // Where each save stands among its deployment's, which says whether its page has been read.
  const placeIn = new Map<Source, number>();
  return (
    <PersonalPage current="saved">
      {[...reads].map(
        ([source, read]) =>
          read.problem !== null && (
            <p key={source.domain ?? ""} role="alert" className="px-2 text-sm text-danger">
              {format(m.saved.failedOn, {
                domain: source.domain ?? m.activity.thisServer,
                problem: read.problem,
              })}
            </p>
          ),
      )}
      {saves.length === 0 ? (
        <p className="px-2 text-sm text-ink-muted">{m.saved.none}</p>
      ) : (
        <ul aria-label={m.saved.heading} className="flex flex-col gap-1">
          {listed.map(({ source, save }) => {
            const index = placeIn.get(source) ?? 0;
            placeIn.set(source, index + 1);
            const read = reads.get(source) ?? UNREAD;
            return (
              <SourceScope key={`${source.domain ?? ""}/${save.id}`} source={source}>
                <SavedItem
                  save={save}
                  domain={source.domain}
                  paged={index < read.count || read.exhausted}
                />
              </SourceScope>
            );
          })}
        </ul>
      )}
      {saves.length > shown && (
        <Button
          onPress={() => {
            setShown((count) => count + SAVED_PAGE);
          }}
          className="self-center rounded-md px-3 py-1.5 text-sm text-accent outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {m.saved.more}
        </Button>
      )}
    </PersonalPage>
  );
}

/**
 * One saved message, in its deployment's scope: where it was said, a way to go to it, and a way
 * to remove it from the saved. Its message is read with its page (`paged`), or on its own when
 * the page did not bring it; one the reader may not read now is left out.
 */
function SavedItem({
  save,
  domain,
  paged,
}: {
  save: SavedMessage;
  domain: string | null;
  paged: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const { message, missing } = useMessageOnDemand(save.message, paged);
  const { home, link, where } = useMessagePlace(message, domain);
  if (missing) {
    return null;
  }
  return (
    <ListedMessage
      message={message}
      home={home}
      link={link}
      where={domain === null ? where : `${where} · ${format(m.search.onDomain, { domain })}`}
      actions={
        <Tooltip text={m.saved.remove}>
          <Button
            aria-label={m.saved.remove}
            onPress={() => {
              void sync.setSaved(save.message, false).catch(() => undefined);
            }}
            className={listedActionClass}
          >
            <XIcon size={16} aria-hidden="true" />
          </Button>
        </Tooltip>
      }
    />
  );
}
