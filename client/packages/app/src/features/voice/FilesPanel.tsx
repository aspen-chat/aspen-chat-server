import type { OfferState, TransferState } from "@aspen/protocol";
import { ArrowDownIcon, ArrowUpIcon, FileArrowUpIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { Button, useLocale } from "react-aria-components";
import { useSilenced, useSync, useUser, useVoiceCall } from "@/api/hooks";
import { outlineButtonClass } from "@/features/auth/styles";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { handleOf } from "@/features/users/profile";
import { OfferFileDialog, ReceiveFileDialog } from "@/features/voice/FileDialogs";
import { formatSize, formatTimeLeft } from "@/features/voice/files";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const smallButtonClass = secondaryButtonClass + " px-2 py-1 text-xs";

/**
 * `Date.now()`, advancing every second while `ticking`, for the offers' countdowns, and read
 * afresh whenever `changed` does, so an offer that just arrived is not counted from a second ago.
 */
function useNow(ticking: boolean, changed: unknown): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const tick = () => {
      setNow(Date.now());
    };
    const fresh = setTimeout(tick, 0);
    const timer = ticking ? setInterval(tick, 1000) : undefined;
    return () => {
      clearTimeout(fresh);
      clearInterval(timer);
    };
  }, [ticking, changed]);
  return now;
}

/** A name safe to save a received file under: no directories, and no control characters. */
function saveName(name: string): string {
  const cleaned = name
    .replace(/[/\\]/g, "_")
    .replace(/\p{Cc}/gu, "")
    .trim();
  return cleaned.length > 0 ? cleaned : "file";
}

function save(file: Blob, name: string): void {
  const url = URL.createObjectURL(file);
  const link = document.createElement("a");
  link.href = url;
  link.download = saveName(name);
  link.click();
  setTimeout(() => {
    URL.revokeObjectURL(url);
  }, 60_000);
}

/**
 * The files of the call the user is in: offering one, what others offer them (not those they
 * blocked), their own offers with the time each has left, and their transfers with progress, a
 * way to cancel each, and, once a file has arrived, a way to save it.
 */
export function FilesPanel() {
  const m = useMessages();
  const call = useVoiceCall();
  const files = call.files;
  const [offering, setOffering] = useState(false);
  const [receiving, setReceiving] = useState<string | null>(null);
  const now = useNow(files.offers.length > 0, files.offers);
  const silenced = useSilenced(files.offers.map((offer) => offer.from));
  // Offers the user is receiving or has received; one that failed or was cancelled can be
  // accepted again while it stands.
  const underway = new Set(
    files.transfers
      .filter(
        (t) =>
          t.role === "receiver" &&
          (t.status === "connecting" || t.status === "moving" || t.status === "completed"),
      )
      .map((t) => t.offer),
  );
  const incoming = files.offers.filter(
    (offer) =>
      !offer.own && !silenced.has(offer.from) && offer.expiresAt > now && !underway.has(offer.id),
  );
  const own = files.offers.filter((offer) => offer.own && offer.expiresAt > now);
  const accepting = incoming.find((offer) => offer.id === receiving);
  if (!call.canTransfer && incoming.length === 0 && files.transfers.length === 0) {
    return null;
  }
  return (
    <section aria-labelledby="call-files" className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-2">
        <h2 id="call-files" className="text-sm font-semibold text-ink-muted">
          {m.files.heading}
        </h2>
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
  const sender = useUser(offer.from);
  const handle = sender === undefined ? m.unknownUser : handleOf(sender);
  return (
    <li className={rowClass}>
      <ArrowDownIcon size={16} aria-hidden="true" className="shrink-0 text-ink-muted" />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-medium">{offer.name}</span>
        <span className="text-xs text-ink-muted">
          {format(m.files.offeredBy, { size: formatSize(offer.size, locale), handle })}
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
        return m.files.completed;
      case "cancelled":
        return transfer.endedBy === "self"
          ? m.files.cancelledBySelf
          : format(m.files.cancelledByPeer, { handle });
      case "failed":
        return m.files.failed;
      case "left":
        return format(m.files.left, { handle });
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
                  save(transfer.file, transfer.name);
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
