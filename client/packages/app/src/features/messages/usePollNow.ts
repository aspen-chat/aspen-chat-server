import type { Poll, ServerClock } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { pollOpen } from "./poll";

/**
 * The current time on `clock`, the clock of the poll's server, which set `closesAt`: read on
 * the device's own, a fast one would hide voting while the server still takes votes. It is
 * refreshed while the poll is open: every second in its last hour, when seconds are shown, and
 * every minute before that.
 */
export function usePollNow(poll: Poll, clock: ServerClock): number {
  const [deviceNow, setNow] = useState(() => Date.now());
  const now = clock.at(deviceNow);
  const open = pollOpen(poll, now);
  const lastHour = Date.parse(poll.closesAt) - now < 3_600_000;
  useEffect(() => {
    if (!open) {
      return;
    }
    const timer = setInterval(
      () => {
        setNow(Date.now());
      },
      lastHour ? 1000 : 60_000,
    );
    return () => {
      clearInterval(timer);
    };
  }, [open, lastHour]);
  return now;
}
