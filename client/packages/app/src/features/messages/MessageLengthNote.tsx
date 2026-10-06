import { MESSAGE_MAX_CHARS } from "@/features/messages/messageLength";
import { useMessages } from "@/i18n/context";
import { useNumberFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

/** From how near the limit the message box counts what is written. */
const COUNT_FROM = MESSAGE_MAX_CHARS - 1_000;

/**
 * Beneath a message box: nothing while the text is well within the limit, a count of its
 * characters once it nears it, and once over it, a message saying how far, while sending waits.
 */
export function MessageLengthNote({ length }: { length: number }) {
  const m = useMessages();
  const numbers = useNumberFormat();
  if (length < COUNT_FROM) {
    return null;
  }
  const over = length > MESSAGE_MAX_CHARS;
  return (
    <p className={"flex gap-2 text-xs " + (over ? "text-danger" : "text-ink-muted")}>
      {over && (
        <span role="alert" className="flex-1">
          {format(m.messageTooLong, {
            max: numbers.format(MESSAGE_MAX_CHARS),
            over: numbers.format(length - MESSAGE_MAX_CHARS),
          })}
        </span>
      )}
      <span aria-hidden={!over} className="ms-auto tabular-nums">
        {format(m.messageLengthCount, {
          count: numbers.format(length),
          max: numbers.format(MESSAGE_MAX_CHARS),
        })}
      </span>
    </p>
  );
}
