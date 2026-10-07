import type { CSSProperties } from "react";
import { useLocale } from "react-aria-components";
import { useChannel, useTypers } from "@/api/hooks";
import { PersonName } from "@/features/users/PersonName";
import { useMessages } from "@/i18n/context";
import { formatNodes, listNodes } from "@/i18n/formatNodes";

/** The most people named; more are "several people". */
const MAX_NAMED = 3;

/**
 * Who else is typing in a channel, just above its message box: their names for up to three
 * people, and "several people" for more, beside three dots lit in turn. Its line is kept
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
          <TypingDots />
          <span className="truncate">{text}</span>
        </p>
      )}
    </div>
  );
}

/**
 * Three dots in the accent's colour, lit one after another from the start of the line
 * (`typing-dot` in `styles.css`). With animations off they stay as they rest, dimmed.
 */
export function TypingDots() {
  return (
    <svg
      viewBox="0 0 22 6"
      aria-hidden="true"
      className="h-[0.6em] w-auto shrink-0 text-accent rtl:-scale-x-100"
      fill="currentColor"
    >
      {[3, 11, 19].map((cx, index) => (
        <circle
          key={cx}
          className="typing-dot"
          style={{ "--dot": index } as CSSProperties}
          cx={cx}
          cy={3}
          r={2.5}
        />
      ))}
    </svg>
  );
}
