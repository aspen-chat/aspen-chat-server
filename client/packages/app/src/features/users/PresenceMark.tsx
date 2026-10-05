import type { UserOnlineStatus } from "@aspen/protocol";
import { useId } from "react";

/** Each status's colour; the shape alone tells them apart, the colour only adds to it. */
const COLOUR: Record<UserOnlineStatus, string> = {
  online: "text-online",
  away: "text-away",
  offline: "text-ink-faint",
};

/**
 * Whether someone is online, away, or offline, told by shape as well as colour, so it reads
 * without telling green from yellow, in greyscale, and in forced colours: a full dot for
 * online, a crescent for away, a ring for offline. `label` names it for assistive technology;
 * without one it is decoration beside text that says the same.
 */
export function PresenceMark({
  status,
  label,
  className = "",
}: {
  status: UserOnlineStatus;
  label?: string;
  className?: string;
}) {
  const cut = useId();
  return (
    <svg
      viewBox="0 0 10 10"
      {...(label === undefined ? { "aria-hidden": true } : { role: "img", "aria-label": label })}
      className={`shrink-0 fill-current ${COLOUR[status]} ${className}`}
    >
      {status === "online" && <circle cx="5" cy="5" r="5" />}
      {status === "away" && (
        <>
          <mask id={cut}>
            <rect width="10" height="10" fill="white" />
            <circle cx="2.5" cy="2.5" r="3.75" fill="black" />
          </mask>
          <circle cx="5" cy="5" r="5" mask={`url(#${cut})`} />
        </>
      )}
      {status === "offline" && (
        <circle cx="5" cy="5" r="3.5" fill="none" stroke="currentColor" strokeWidth="3" />
      )}
    </svg>
  );
}
