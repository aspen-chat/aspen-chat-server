import { MANA_FRAME_MS, MANA_SHOW_BELOW, manaDrawnAt } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useChannelMana } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { TopSendersDialog } from "@/features/messages/TopSendersDialog";

/**
 * A channel's mana while it is busy (the server's `app::mana`): a thin bar along the top of the
 * typing line, labelled at the line's end, shown once the channel's mana falls below
 * `MANA_SHOW_BELOW` of its maximum and kept until it is full again, and whenever anything waits
 * for the mana (`queued`), as a long message does while the channel recharges enough to pay for
 * it. Then the label says the channel's priority queue is in force. Beside the label, "View
 * top senders" opens who spent most of the channel's mana in the past hour (`TopSendersDialog`). Between the
 * levels the server tells, it is drawn rising steadily, `MANA_FRAME_MS` apart. It lies over the
 * line rather than taking room of its own, so the conversation never moves when it comes or
 * goes.
 */
export function ManaBar({ channelId }: { channelId: string }) {
  const m = useMessages();
  const mana = useChannelMana(channelId);
  const [now, setNow] = useState(() => Date.now());
  const [shown, setShown] = useState(false);
  const [sendersOpen, setSendersOpen] = useState(false);
  const level = mana === undefined ? null : manaDrawnAt(mana, now);
  const max = mana === undefined ? 0 : mana.max * 1000;
  const full = level === null || level >= max;
  useEffect(() => {
    if (full) {
      return;
    }
    const frame = window.setInterval(() => {
      setNow(Date.now());
    }, MANA_FRAME_MS);
    return () => {
      window.clearInterval(frame);
    };
  }, [full]);
  // Shown once below the threshold, and kept until full: adjusted as it renders, from the level.
  const below = !full && (level < max * MANA_SHOW_BELOW || mana?.queued === true);
  if (full && shown) {
    setShown(false);
  } else if (below && !shown) {
    setShown(true);
  }
  const dialog = (
    <TopSendersDialog channelId={channelId} isOpen={sendersOpen} onOpenChange={setSendersOpen} />
  );
  if (!shown || full || mana === undefined) {
    return dialog;
  }
  const share = Math.max(0, level / max);
  const percent = Math.round(share * 100);
  const label = mana.queued ? m.mana.depleted : m.mana.label;
  return (
    <>
      <div
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-valuetext={format(m.mana.value, { percent })}
        className="motion-fade pointer-events-none absolute inset-x-0 top-0 h-1 bg-line"
      >
        <div className="h-full bg-accent" style={{ width: `${String(share * 100)}%` }} />
      </div>
      <div className="motion-fade message-text pointer-events-none absolute top-1 bottom-0 end-0 flex items-center gap-3 bg-surface-sunken ps-2 pe-4 text-sm">
        <span
          aria-hidden="true"
          className={mana.queued ? "font-semibold text-danger" : "text-ink-muted"}
        >
          {label}
        </span>
        <Button
          onPress={() => {
            setSendersOpen(true);
          }}
          className="pointer-events-auto text-accent underline-offset-2 hover:underline focus-visible:underline focus-visible:outline-none"
        >
          {m.mana.topSenders}
        </Button>
      </div>
      {dialog}
    </>
  );
}
