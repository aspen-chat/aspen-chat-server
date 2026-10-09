import type { UserOnlineStatus } from "@aspen/protocol";
import { useId } from "react";
import { knownStatus } from "./presenceStatus";

/** Each status's colour; the shape alone tells them apart, the colour only adds to it. */
const COLOUR: Record<UserOnlineStatus, string> = {
  online: "text-online",
  away: "text-away",
  doNotDisturb: "text-busy",
  offline: "text-ink-faint",
  invisible: "text-ink-faint",
};

/**
 * Whether someone is online, away, in do not disturb, or offline, told by shape as well as
 * colour, so it reads without telling green from yellow, in greyscale, and in forced colours: a
 * full dot for online, a crescent for away, a dot with a bar cut through it for do not disturb,
 * and a ring for offline, and for invisible, which only the user is told of themself and which
 * shows them offline to everyone else. `label` names it for assistive technology; without one
 * it is decoration beside text that says the same.
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
  const shown = knownStatus(status);
  return (
    <svg
      viewBox="0 0 10 10"
      {...(label === undefined ? { "aria-hidden": true } : { role: "img", "aria-label": label })}
      className={`shrink-0 fill-current ${COLOUR[shown]} ${className}`}
    >
      {shown === "online" && <circle cx="5" cy="5" r="5" />}
      {shown === "away" && (
        <>
          <mask id={cut}>
            <rect width="10" height="10" fill="white" />
            <circle cx="2.5" cy="2.5" r="3.75" fill="black" />
          </mask>
          <circle cx="5" cy="5" r="5" mask={`url(#${cut})`} />
        </>
      )}
      {shown === "doNotDisturb" && (
        <>
          <mask id={cut}>
            <rect width="10" height="10" fill="white" />
            <rect x="2" y="3.75" width="6" height="2.5" rx="1.25" fill="black" />
          </mask>
          <circle cx="5" cy="5" r="5" mask={`url(#${cut})`} />
        </>
      )}
      {(shown === "offline" || shown === "invisible") && (
        <circle cx="5" cy="5" r="3.5" fill="none" stroke="currentColor" strokeWidth="3" />
      )}
    </svg>
  );
}

/**
 * A status over someone's picture, which sits in a `relative` box with it, on a disc of the
 * ground around the picture (`groundClassName`, a background colour) so it stands clear of it.
 */
export function StatusDot({
  status,
  label,
  large = false,
  groundClassName = "bg-surface-raised",
}: {
  status: UserOnlineStatus;
  label: string;
  /** For a large picture, which a small dot would be lost on. */
  large?: boolean;
  groundClassName?: string;
}) {
  return (
    <span className={`absolute -end-0.5 -bottom-0.5 flex rounded-full p-0.5 ${groundClassName}`}>
      <PresenceMark
        status={status}
        label={label}
        className={large ? "h-3.5 w-3.5" : "h-2.5 w-2.5"}
      />
    </span>
  );
}
