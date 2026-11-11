import { plainFileName, type FileOfferEntry } from "@aspen/protocol";
import { useCallback, useState } from "react";
import { Button, useLocale } from "react-aria-components";
import { useSync, useIdWizard } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { AdminUserPicker } from "@/features/admin/AdminUserPicker";
import { Cell, Table } from "@/features/admin/FleetHealth";
import { useCursorLog } from "@/features/admin/useCursorLog";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { formatSize } from "@/features/voice/files";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { PersonName } from "@/features/users/PersonName";
import { formatNodes } from "@/i18n/formatNodes";
import { CopyIdButton } from "@/features/layout/CopyId";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/**
 * The record of files offered in calls, newest first, a page at a time: who offered what, by
 * name and size, and who received it, how, and how it ended, of everyone or only of the person
 * chosen, as sender or receiver. The files never reach the server.
 */
export function FileTransferLog() {
  const timeFormat = useDateFormat(TIME);
  const { locale } = useLocale();
  const m = useMessages();
  const sync = useSync();
  const wizard = useIdWizard();
  const [person, setPerson] = useState<string | undefined>(undefined);
  const load = useCallback(
    (cursor?: string) => sync.admin.fileTransferLog(cursor, person),
    [sync, person],
  );
  const { entries, loading, error, more, showMore, reload } = useCursorLog(load);

  return (
    <Section
      id="admin-file-transfers"
      title={m.admin.fileTransfers}
      hint={m.admin.fileTransfersHint}
    >
      <div className="max-w-xs">
        <AdminUserPicker
          label={m.admin.fileTransfersOf}
          anyoneLabel={m.admin.fileTransfersOfAnyone}
          onChange={(user) => {
            setPerson(user?.id);
          }}
        />
      </div>
      {error !== null && <ReadFailed error={error} onRetry={reload} />}
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
            ...(wizard ? [{ content: m.bots.idColumn }] : []),
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
                <span className="break-all">{plainFileName(entry.fileName)}</span>
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
              {wizard && (
                <Cell>
                  <CopyIdButton id={entry.id} thing="fileOffer" />
                </Cell>
              )}
            </tr>
          ))}
        </Table>
      )}
      {more && (
        <Button
          isDisabled={loading}
          onPress={showMore}
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
