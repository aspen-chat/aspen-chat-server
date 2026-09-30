import { ApiProblemError, type FileOfferEntry } from "@aspen/protocol";
import { useCallback, useEffect, useState } from "react";
import { Button, useLocale } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { Cell, Table } from "@/features/admin/FleetHealth";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { formatSize } from "@/features/voice/files";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { PersonName } from "@/features/users/PersonName";
import { formatNodes } from "@/i18n/formatNodes";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/** A page of the record is what the server gives by default. */
const PAGE = 50;

/**
 * The record of files offered in calls, newest first, a page at a time: who offered what, by
 * name and size, and who received it, how, and how it ended. The files never reach the server.
 */
export function FileTransferLog() {
  const timeFormat = useDateFormat(TIME);
  const { locale } = useLocale();
  const m = useMessages();
  const sync = useSync();
  const [entries, setEntries] = useState<readonly FileOfferEntry[]>([]);
  const [complete, setComplete] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(
    (before: string | undefined, isCurrent: () => boolean) => {
      sync.fileTransferLog(before).then(
        (page) => {
          if (!isCurrent()) {
            return;
          }
          setEntries((held) => (before === undefined ? page : [...held, ...page]));
          setComplete(page.length < PAGE);
          setError(null);
          setLoading(false);
        },
        (e: unknown) => {
          if (isCurrent()) {
            setError(e instanceof ApiProblemError ? e.message : String(e));
            setLoading(false);
          }
        },
      );
    },
    [sync],
  );

  useEffect(() => {
    let current = true;
    read(undefined, () => current);
    return () => {
      current = false;
    };
  }, [read]);

  return (
    <Section
      id="admin-file-transfers"
      title={m.admin.fileTransfers}
      hint={m.admin.fileTransfersHint}
    >
      {error !== null && (
        <ReadFailed
          error={error}
          onRetry={() => {
            setLoading(true);
            read(undefined, () => true);
          }}
        />
      )}
      {!loading && entries.length === 0 && error === null ? (
        <p className="text-sm text-ink-muted">{m.admin.noFileTransfers}</p>
      ) : (
        <Table
          label={m.admin.fileTransfers}
          headings={[
            { content: m.admin.logWhen },
            { content: m.admin.logWho },
            { content: m.admin.logFile },
            { content: m.admin.logReceivers },
          ]}
          numeric={[]}
          dimmed={loading && entries.length > 0}
          skeletonRows={loading && entries.length === 0 ? 5 : 0}
        >
          {entries.map((entry) => (
            <tr key={entry.id}>
              <Cell>{timeFormat.format(new Date(entry.offeredAt))}</Cell>
              <Cell>
                <Handle userId={entry.sender ?? undefined} />
              </Cell>
              <Cell>
                <span className="break-all">{entry.fileName}</span>
                <span className="text-ink-muted"> · {formatSize(entry.fileSize, locale)}</span>
              </Cell>
              <Cell>
                {entry.transfers.length === 0 ? (
                  <span className="text-ink-muted">{m.admin.noReceivers}</span>
                ) : (
                  <ul className="flex flex-col gap-0.5">
                    {entry.transfers.map((transfer) => (
                      <li key={transfer.startedAt + (transfer.receiver ?? "")}>
                        <ReceiverLine transfer={transfer} />
                      </li>
                    ))}
                  </ul>
                )}
              </Cell>
            </tr>
          ))}
        </Table>
      )}
      {!complete && entries.length > 0 && (
        <Button
          isDisabled={loading}
          onPress={() => {
            setLoading(true);
            read(entries.at(-1)?.id, () => true);
          }}
          className={secondaryButtonClass + " self-start"}
        >
          {m.admin.showMore}
        </Button>
      )}
    </Section>
  );
}

function Handle({ userId }: { userId: string | undefined }) {
  return <PersonName id={userId} handle />;
}

function ReceiverLine({ transfer }: { transfer: FileOfferEntry["transfers"][number] }) {
  const m = useMessages();
  return (
    <>
      {formatNodes(m.admin.receiverLine, {
        name: <PersonName id={transfer.receiver ?? undefined} handle />,
        mode: m.admin.transferModes[transfer.mode],
        outcome:
          transfer.outcome == null
            ? m.admin.transferOutcomes.underway
            : m.admin.transferOutcomes[transfer.outcome],
      })}
    </>
  );
}
