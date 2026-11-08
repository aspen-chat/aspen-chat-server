import type { OfferState, TransferState } from "@aspen/protocol";
import { ArrowDownIcon, ArrowUpIcon, FileArrowUpIcon, FolderOpenIcon } from "@phosphor-icons/react";
import { useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Modal,
  ModalOverlay,
  useLocale,
} from "react-aria-components";
import { useSync, useUser, useVoiceCall } from "@/api/hooks";
import { outlineButtonClass } from "@/features/auth/styles";
import {
  dialogClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { Tooltip } from "@/features/layout/Tooltip";
import { MentionBadge } from "@/features/mentions/MentionBadge";
import { callBarButtonClass } from "@/features/voice/callBarButton";
import { hasFiles, useIncomingOffers } from "@/features/voice/callFiles";
import { handleOf } from "@/features/users/profile";
import { OfferFileDialog, ReceiveFileDialog } from "@/features/voice/FileDialogs";
import { formatSize, formatTimeLeft } from "@/features/voice/files";
import { saveFile } from "@/features/layout/saveFile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { PersonName } from "@/features/users/PersonName";
import { formatNodes } from "@/i18n/formatNodes";

const smallButtonClass = secondaryButtonClass + " px-2 py-1 text-xs";

/**
 * The files of the call the user is in: offering one, what others offer them (not those they
 * blocked), their own offers with the time each has left, and their transfers with progress, a
 * way to cancel each, and, once a file has arrived, a way to save it. `titled` heads it with its
 * own title, which a dialog holding it gives instead.
 */
export function FilesPanel({ titled = true }: { titled?: boolean }) {
  const m = useMessages();
  const call = useVoiceCall();
  const files = call.files;
  const [offering, setOffering] = useState(false);
  const [receiving, setReceiving] = useState<string | null>(null);
  const { incoming, now } = useIncomingOffers();
  const own = files.offers.filter((offer) => offer.own && offer.expiresAt > now);
  const accepting = incoming.find((offer) => offer.id === receiving);
  if (!hasFiles(call.canTransfer, incoming.length, files.transfers.length)) {
    return null;
  }
  return (
    <section
      aria-labelledby={titled ? "call-files" : undefined}
      aria-label={titled ? undefined : m.files.heading}
      className="flex flex-col gap-3"
    >
      <div className={"flex items-center gap-2 " + (titled ? "justify-between" : "justify-end")}>
        {titled && (
          <h2 id="call-files" className="text-sm font-semibold text-ink-muted">
            {m.files.heading}
          </h2>
        )}
        {call.canTransfer && (
          <Button
            onPress={() => {
              setOffering(true);
            }}
            className={outlineButtonClass + " flex items-center gap-2"}
          >
            <FileArrowUpIcon size={16} aria-hidden="true" />
            {m.files.offer}
          </Button>
        )}
      </div>
      {incoming.length === 0 && own.length === 0 && files.transfers.length === 0 && (
        <p className="text-sm text-ink-muted">{m.files.nothing}</p>
      )}
      {incoming.length > 0 && (
        <OfferList label={m.files.offeredToYou}>
          {incoming.map((offer) => (
            <IncomingOffer
              key={offer.id}
              offer={offer}
              now={now}
              onReceive={() => {
                setReceiving(offer.id);
              }}
            />
          ))}
        </OfferList>
      )}
      {own.length > 0 && (
        <OfferList label={m.files.yourOffers}>
          {own.map((offer) => (
            <OwnOffer key={offer.id} offer={offer} now={now} />
          ))}
        </OfferList>
      )}
      {files.transfers.length > 0 && (
        <OfferList label={m.files.transfers}>
          {files.transfers.map((transfer) => (
            <Transfer key={`${transfer.offer}:${transfer.peer}`} transfer={transfer} />
          ))}
        </OfferList>
      )}
      {offering && (
        <OfferFileDialog
          relayMbps={files.relayMbps}
          onClose={() => {
            setOffering(false);
          }}
        />
      )}
      {accepting !== undefined && (
        <ReceiveFileDialog
          offer={accepting}
          relayMbps={files.relayMbps}
          onClose={() => {
            setReceiving(null);
          }}
        />
      )}
    </section>
  );
}

function OfferList({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <h3 className="text-xs font-semibold text-ink-faint uppercase">{label}</h3>
      <ul aria-label={label} className="flex flex-col gap-1">
        {children}
      </ul>
    </div>
  );
}

const rowClass = "flex items-center gap-3 rounded-md bg-surface-raised px-3 py-2 text-sm";

function IncomingOffer({
  offer,
  now,
  onReceive,
}: {
  offer: OfferState;
  now: number;
  onReceive: () => void;
}) {
  const m = useMessages();
  const { locale } = useLocale();
  return (
    <li className={rowClass}>
      <ArrowDownIcon size={16} aria-hidden="true" className="shrink-0 text-ink-muted" />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-medium">{offer.name}</span>
        <span className="text-xs text-ink-muted">
          {formatNodes(m.files.offeredBy, {
            size: formatSize(offer.size, locale),
            handle: <PersonName id={offer.from} handle />,
          })}
          {" · "}
          {format(m.files.timeLeft, { time: formatTimeLeft(offer.expiresAt - now) })}
        </span>
      </span>
      <Button onPress={onReceive} className={smallButtonClass}>
        {m.files.receiveButton}
      </Button>
    </li>
  );
}

function OwnOffer({ offer, now }: { offer: OfferState; now: number }) {
  const m = useMessages();
  const sync = useSync();
  const { locale } = useLocale();
  return (
    <li className={rowClass}>
      <ArrowUpIcon size={16} aria-hidden="true" className="shrink-0 text-ink-muted" />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-medium">{offer.name}</span>
        <span className="text-xs text-ink-muted">
          {formatSize(offer.size, locale)}
          {" · "}
          {format(m.files.timeLeft, { time: formatTimeLeft(offer.expiresAt - now) })}
        </span>
      </span>
      <Button
        onPress={() => {
          sync.voice.withdrawOffer(offer.id);
        }}
        className={smallButtonClass}
      >
        {m.files.withdraw}
      </Button>
    </li>
  );
}

function Transfer({ transfer }: { transfer: TransferState }) {
  const m = useMessages();
  const sync = useSync();
  const { locale } = useLocale();
  const peer = useUser(transfer.peer);
  const handle = peer === undefined ? m.unknownUser : handleOf(peer);
  const sending = transfer.role === "sender";
  const live = transfer.status === "connecting" || transfer.status === "moving";
  const percent = transfer.size === 0 ? 100 : Math.round((transfer.bytes / transfer.size) * 100);
  const status = (() => {
    switch (transfer.status) {
      case "connecting":
        return m.files.connecting;
      case "moving":
        return format(m.files.progress, {
          done: formatSize(transfer.bytes, locale),
          total: formatSize(transfer.size, locale),
        });
      case "completed":
        return transfer.toDisk ? m.files.saved : m.files.completed;
      case "cancelled":
        return transfer.endedBy === "self"
          ? m.files.cancelledBySelf
          : format(m.files.cancelledByPeer, { handle });
      case "failed":
        return m.files.failed;
      case "left":
        return format(m.files.left, { handle });
      case "notPermitted":
        return sending ? m.files.notPermittedSelf : format(m.files.notPermittedPeer, { handle });
    }
  })();
  return (
    <li className={rowClass}>
      {sending ? (
        <ArrowUpIcon size={16} aria-hidden="true" className="shrink-0 text-ink-muted" />
      ) : (
        <ArrowDownIcon size={16} aria-hidden="true" className="shrink-0 text-ink-muted" />
      )}
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="truncate font-medium">
          {format(sending ? m.files.sendingTo : m.files.receivingFrom, {
            name: transfer.name,
            handle,
          })}
        </span>
        {live && (
          <progress
            value={transfer.bytes}
            max={Math.max(transfer.size, 1)}
            aria-label={`${String(percent)}%`}
            className="h-1.5 w-full accent-accent"
          />
        )}
        <span className="text-xs text-ink-muted">{status}</span>
        <span className="text-xs text-ink-muted">
          {format(m.files.preferred, {
            mode: transfer.mode === "relayOnly" ? m.files.modeRelay : m.files.modeDirect,
          })}
          {" · "}
          {format(m.files.actual, {
            route:
              transfer.route === null
                ? m.files.routePending
                : transfer.route === "relayed"
                  ? m.files.routeRelayed
                  : m.files.routeDirect,
          })}
        </span>
      </span>
      {live ? (
        <Button
          onPress={() => {
            sync.voice.cancelTransfer(transfer.offer, transfer.peer);
          }}
          className={smallButtonClass}
        >
          {m.files.cancelTransfer}
        </Button>
      ) : (
        <span className="flex gap-1">
          {transfer.file !== null && (
            <Button
              onPress={() => {
                if (transfer.file !== null) {
                  void saveFile(transfer.file, transfer.name);
                }
              }}
              className={smallButtonClass}
            >
              {m.files.save}
            </Button>
          )}
          <Button
            onPress={() => {
              sync.voice.dismissTransfer(transfer.offer, transfer.peer);
            }}
            className={smallButtonClass}
          >
            {m.files.dismiss}
          </Button>
        </span>
      )}
    </li>
  );
}

/**
 * The call bar's way to the call's files, where the call screen does not show them (a phone or
 * tablet): a button opening them in a dialog, counting the offers waiting for the user, so one
 * is not missed while the files are out of sight.
 */
export function FilesButton() {
  const m = useMessages();
  const call = useVoiceCall();
  const { incoming } = useIncomingOffers();
  if (!hasFiles(call.canTransfer, incoming.length, call.files.transfers.length)) {
    return null;
  }
  const label =
    incoming.length === 0
      ? m.files.heading
      : format(m.files.headingWithOffers, { count: incoming.length });
  return (
    <DialogTrigger>
      <Tooltip text={label}>
        <Button aria-label={label} className={callBarButtonClass + " relative"}>
          <FolderOpenIcon size={24} aria-hidden="true" />
          <MentionBadge
            count={incoming.length}
            title=""
            className="absolute top-0.5 start-1/2 ms-2"
          />
        </Button>
      </Tooltip>
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>{m.files.heading}</DialogHeading>
            <FilesPanel titled={false} />
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}
