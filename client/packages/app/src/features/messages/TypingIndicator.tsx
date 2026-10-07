import { type CSSProperties, useId } from "react";
import { useLocale } from "react-aria-components";
import { useChannel, useTypers } from "@/api/hooks";
import { PersonName } from "@/features/users/PersonName";
import { useMessages } from "@/i18n/context";
import { formatNodes, listNodes } from "@/i18n/formatNodes";

/** The most people named; more are "several people". */
const MAX_NAMED = 3;

/**
 * Who else is typing in a channel, just above its message box: their names for up to three
 * people, and "several people" for more, beside three keys pressed in turn. Its line is kept
 * whether or not anyone is typing, so the conversation never moves when someone starts or
 * stops. It is not a live region: a screen reader announcing every start and stop would talk
 * over the conversation itself.
 */
export function TypingIndicator({ channelId }: { channelId: string }) {
  const m = useMessages();
  const { locale } = useLocale();
  const typers = useTypers(channelId);
  const community = useChannel(channelId)?.community ?? null;
  let text = null;
  if (typers.length > MAX_NAMED) {
    text = m.typing.several;
  } else if (typers.length > 0) {
    const names = listNodes(
      locale,
      typers.map((id) => (
        <span key={id} className="font-semibold">
          <PersonName id={id} community={community} width="w-12" />
        </span>
      )),
    );
    text = formatNodes(typers.length === 1 ? m.typing.one : m.typing.some, { names });
  }
  return (
    <div className="flex h-[1lh] items-center px-4 text-xs leading-5 text-ink-muted">
      {text !== null && (
        <p className="motion-fade flex min-w-0 items-center gap-2">
          <TypingKeys />
          <span className="truncate">{text}</span>
        </p>
      )}
    </div>
  );
}

/** Where each switch stands along the row, in the drawing's units. */
const SWITCHES = [0, 22, 44];

/**
 * Three mechanical key switches side on, drawn in line art of one colour (the accent's), each
 * pressed and let go in turn from the left, the next going down just before the last is all
 * the way up (`typing-key` in `styles.css`). Where motion is reduced or off they stand still.
 */
export function TypingKeys() {
  // The stem shows only above the housing, so it slides into it as the key goes down.
  const aboveHousing = useId();
  return (
    <svg
      viewBox="0 0 62 22"
      aria-hidden="true"
      className="typing-keys h-[1.4em] w-auto shrink-0 text-accent"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.3}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <defs>
        <clipPath id={aboveHousing}>
          <rect x={0} y={0} width={18} height={10.85} />
        </clipPath>
      </defs>
      {SWITCHES.map((x, index) => {
        const key = { "--key": index } as CSSProperties;
        return (
          <g key={x} transform={`translate(${String(x)} 0)`}>
            {/* The housing and its pins, which stay put. */}
            <path d="M6 19.5v2M12 19.5v2" />
            <rect x={2} y={15} width={14} height={4.5} rx={0.6} />
            <path d="M3 15l1.5-3.5h9L15 15" />
            {/* The stem and the keycap on it, which go down together. */}
            <g clipPath={`url(#${aboveHousing})`}>
              <path className="typing-key" style={key} d="M7.5 8.5v3M10.5 8.5v3" />
            </g>
            <path className="typing-key" style={key} d="M1 8.5L3 2.2Q9 3.4 15 2.2L17 8.5Z" />
          </g>
        );
      })}
    </svg>
  );
}
