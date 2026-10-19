import { MANA_SHOW_BELOW, MANA_TICK_MS, manaAt } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { useChannelMana } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A channel's mana while it is busy (the server's `app::mana`): a thin bar along the top of the
 * typing line, shown once the channel's mana falls below `MANA_SHOW_BELOW` of its maximum and
 * kept until it is full again. Between the levels the server tells, it fills as the channel
 * recharges, at the server's ticks. It lies over the line rather than taking room of its own,
 * so the conversation never moves when it comes or goes.
 */
export function ManaBar({ channelId }: { channelId: string }) {
  const m = useMessages();
  const mana = useChannelMana(channelId);
  const [now, setNow] = useState(() => Date.now());
  const [shown, setShown] = useState(false);
  const level = mana === undefined ? null : manaAt(mana, now);
  const max = mana === undefined ? 0 : mana.max * 1000;
  const full = level === null || level >= max;
  useEffect(() => {
    if (full) {
      return;
    }
    const tick = window.setInterval(() => {
      setNow(Date.now());
    }, MANA_TICK_MS);
    return () => {
      window.clearInterval(tick);
    };
  }, [full]);
  // Shown once below the threshold, and kept until full: adjusted as it renders, from the level.
  const below = !full && level < max * MANA_SHOW_BELOW;
  if (full && shown) {
    setShown(false);
  } else if (below && !shown) {
    setShown(true);
  }
  if (!shown || full) {
    return null;
  }
  const percent = Math.max(0, Math.round((level / max) * 100));
  return (
    <div
      role="meter"
      aria-label={m.mana.label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      aria-valuetext={format(m.mana.value, { percent })}
      className="motion-fade pointer-events-none absolute inset-x-0 top-0 h-1 bg-line"
    >
      <div className="h-full bg-accent" style={{ width: `${String(percent)}%` }} />
    </div>
  );
}
