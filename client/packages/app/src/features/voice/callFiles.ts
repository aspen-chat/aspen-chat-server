import type { OfferState } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { useSilenced, useVoiceCall } from "@/api/hooks";

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

/**
 * What others in the call offer the user and they may still take, with the clock the offers'
 * countdowns read: not those they blocked, run out, or already being received or received.
 */
export function useIncomingOffers(): { incoming: OfferState[]; now: number } {
  const files = useVoiceCall().files;
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
  return { incoming, now };
}

/**
 * Whether the call has files to show: the user may offer one, or there are some to take or
 * under way.
 */
export function hasFiles(canTransfer: boolean, incoming: number, transfers: number): boolean {
  return canTransfer || incoming > 0 || transfers > 0;
}

/** Whether `FilesButton` shows, for a layout that leaves no room for one that does not. */
export function useCallHasFiles(): boolean {
  const call = useVoiceCall();
  const { incoming } = useIncomingOffers();
  return hasFiles(call.canTransfer, incoming.length, call.files.transfers.length);
}
