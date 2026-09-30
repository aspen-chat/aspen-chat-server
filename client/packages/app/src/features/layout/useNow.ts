import { useEffect, useState } from "react";

/**
 * The time, as milliseconds since the epoch, read again every `everyMs` while `active`, for
 * things that end by the clock with no event, such as a call's ring.
 */
export function useNow(everyMs: number, active = true): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) {
      return;
    }
    const timer = window.setInterval(() => {
      setNow(Date.now());
    }, everyMs);
    return () => {
      window.clearInterval(timer);
    };
  }, [everyMs, active]);
  return now;
}
